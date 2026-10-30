//! The plan the driver selects decides what the hourly walk observes,
//! flushes, loads and saves, and whose refusals reach whom; ordinary
//! preparation goes through the driver, publishes only sections its
//! publishers own, and answers its queries from the pages its plan names.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::driver::plan;
use std::{collections::BTreeSet, sync::Mutex};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// The parts' step in the walk's serial flush order.
const PARTS_FLUSH: usize = 38;

/// The walk's saved state of branch 1.
fn walk_state(source: &Source) -> serde_json::Value {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let file = store
        .read(&key, &branch_section(1), 0)
        .unwrap()
        .expect("the walk's branch state");
    serde_json::from_reader(zstd::stream::Decoder::new(file).unwrap()).unwrap()
}

fn without_checkpoint(payloads: Payloads) -> Payloads {
    payloads
        .into_iter()
        .filter(|((name, _), _)| name != CHECKPOINT)
        .collect()
}

/// What the hourly walk did with stability's parts in one preparation, counted where
/// it does it: the hours it observed, the flush steps it ran and the branch
/// states it restored it from.
#[derive(Debug, Default, PartialEq, Eq)]
struct Walked {
    observed: BTreeSet<(u64, i32)>,
    flushed: usize,
    restored: usize,
}

fn walked(source: &Source) -> Walked {
    let observed = Arc::new(Mutex::new(BTreeSet::new()));
    let seen = Arc::clone(&observed);
    let hook: lanes::Hook = Arc::new(move |family, frame, _| {
        if family == "parts" {
            seen.lock().unwrap().insert((frame.branch, frame.hour));
        }
        lanes::Fault::Continue
    });
    let ((prepared, restored), steps) = flush::with_recorded_steps(|| {
        with_test_restored(|| {
            lanes::with_test_lanes(2, Some(hook), || {
                run(source, None, &AtomicBool::new(false), |_| {})
            })
        })
    });
    prepared.unwrap();
    let observed = std::mem::take(&mut *observed.lock().unwrap());
    Walked {
        observed,
        flushed: steps
            .iter()
            .filter(|&&(position, _, _)| position == PARTS_FLUSH)
            .count(),
        restored: restored.iter().filter(|&&family| family == "parts").count(),
    }
}

/// The parts prepared on their own, or every family hour by hour.
fn planned<T>(own: bool, run: impl FnOnce() -> T) -> T {
    if own {
        independent::with_test_only(&["parts"], run)
    } else {
        independent::with_test_hourly(run)
    }
}

/// The extent the parts' queries answer on branch 1.
fn parts_coverage(source: &Source) -> (i32, i32, bool) {
    let coverage = product_coverage(source, None, 1, "stability-parts", &AtomicBool::new(false))
        .unwrap()
        .expect("the parts' coverage");
    (coverage.first_hour, coverage.last_hour, coverage.complete)
}

#[test]
fn the_walk_never_observes_flushes_loads_or_saves_a_family_prepared_on_its_own() {
    let first = recording();
    let grown = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    let own_page = (independent::coverage_section("parts", 1), 0);
    let continuation = (
        crate::prepared::driver::registry::continuation_section("parts", 1),
        0,
    );
    for own in [true, false] {
        let label = if own { "own" } else { "hourly" };
        let mut fixture = Fixture::new(&first);
        let source = fixture.source("first");
        let (prepared, state, pages, extent) = planned(own, || {
            assert_eq!(Refused::default().live("parts"), !own, "{label}");
            assert!(
                prepass::families()
                    .iter()
                    .any(|family| family.name == "resources")
            );
            let prepared = walked(&source);
            (
                prepared,
                walk_state(&source),
                payloads(&source, None),
                parts_coverage(&source),
            )
        });
        assert_eq!(extent, (10, 11, true), "{label}");
        // The recording grows: preparation resumes from what it accepted.
        fixture.replace(&grown);
        let source = fixture.source("grown");
        let (resumed, resumed_state, resumed_pages, resumed_extent) = planned(own, || {
            (
                walked(&source),
                walk_state(&source),
                payloads(&source, None),
                parts_coverage(&source),
            )
        });
        assert_eq!(resumed_extent, (10, 13, true), "{label}");
        if own {
            assert_eq!(prepared, Walked::default(), "the walk prepared the parts");
            assert_eq!(
                resumed,
                Walked::default(),
                "the resumed walk prepared the parts"
            );
            for state in [&state, &resumed_state] {
                assert!(state.get("parts").is_none(), "the walk saved the parts");
                assert!(state.get("resources").is_some());
            }
            for pages in [&pages, &resumed_pages] {
                assert!(pages.contains_key(&own_page) && pages.contains_key(&continuation));
            }
        } else {
            assert_eq!(
                prepared.observed,
                BTreeSet::from([(1, 10), (1, 11), (2, 20), (2, 21)])
            );
            assert_eq!(resumed.observed, BTreeSet::from([(1, 12), (1, 13)]));
            assert!(prepared.flushed > 0 && resumed.flushed > 0);
            assert!(resumed.restored > 0, "the resumed walk restored no parts");
            for state in [&state, &resumed_state] {
                assert!(state.get("parts").is_some(), "the walk left the parts out");
            }
            for pages in [&pages, &resumed_pages] {
                assert!(!pages.contains_key(&own_page) && !pages.contains_key(&continuation));
            }
        }
    }
}

#[test]
fn a_refusal_reaches_the_same_hour_dependents_its_registry_names() {
    let mut refused = Refused::default();
    refused.refuse("identities", 10, "refused".into(), None);
    for family in ["identities", "territory", "event_feed"] {
        assert!(!refused.open(family), "{family}");
    }
    assert_eq!(refused.0.len(), 3);
    assert_eq!(refused.0["identities"].from, None);
    assert_eq!(refused.0["territory"].from.as_deref(), Some("identities"));
    assert_eq!(refused.0["event_feed"].from.as_deref(), Some("identities"));

    let mut refused = Refused::default();
    refused.refuse("groups", 10, "refused".into(), None);
    assert_eq!(
        refused.0.keys().map(String::as_str).collect::<Vec<_>>(),
        ["air_superiority", "groups"]
    );
    // Query joins are not same-hour dependents: territory refuses alone.
    let mut refused = Refused::default();
    refused.refuse("territory", 10, "refused".into(), None);
    assert_eq!(
        refused.0.keys().map(String::as_str).collect::<Vec<_>>(),
        ["territory"]
    );
}

#[test]
fn the_walk_decodes_the_tables_its_plan_selects() {
    assert_eq!(selected(), plan::rows().walk_tables().as_slice());
    // A family normally prepared on its own back in the walk reads it all.
    independent::with_test_hourly(|| assert_eq!(selected(), TableId::ALL.as_slice()));
}

/// Every family ordinary preparation prepares on its own, and the job
/// beside them.
const PUBLISHERS: [&str; 56] = [
    "aces",
    "air",
    "air-missions",
    "air-statistics",
    "air_ledger",
    "air_superiority",
    "casualties",
    "construction",
    "convoys",
    "diplomacy_state",
    "divisions",
    "doctrine-mastery",
    "equipment_losses",
    "event_feed",
    "experience",
    "faction_state",
    "factories",
    "focus",
    "fuel",
    "fuel_branches",
    "groups",
    "ideas",
    "identities",
    "intel-network-coverage",
    "intelligence",
    "leaders",
    "lend_lease_balance",
    "map-pages",
    "market",
    "minefields",
    "naval",
    "naval-refits",
    "naval-repairs",
    "naval-supremacy",
    "navy",
    "nukes",
    "occupation",
    "occupation-reserve",
    "parties",
    "parts",
    "peace",
    "politics_state",
    "production",
    "production-lines",
    "production_catalogue",
    "projects",
    "raids",
    "regional-weather",
    "repairs",
    "research",
    "stockpile",
    "subjects",
    "tension",
    "territory",
    "trade",
    "wars",
];

/// Ordinary preparation goes through the driver: it publishes what naming
/// every row implementation publishes and restores it, every section it
/// publishes has a publisher of the plan, and its queries pin and answer
/// from the pages the plan names.
#[test]
fn ordinary_preparation_goes_through_the_driver_and_answers_from_its_plan() {
    let replay = recording();
    let flag = AtomicBool::new(false);

    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("driver");
    let accepted = crate::prepared::driver::prepare(&source, None, &flag, |_| {}).unwrap();
    let driven = payloads(&source, None);
    let restored = crate::prepared::driver::restore(&source, None, &flag)
        .unwrap()
        .expect("a complete preparation restores");
    let reported = |accepted: &Accepted| {
        serde_json::to_value((&accepted.sections, &accepted.coverage, &accepted.families)).unwrap()
    };
    assert_eq!(reported(&restored), reported(&accepted));
    assert_eq!(
        accepted
            .families
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        PUBLISHERS
    );

    let rows = plan::rows();
    for (section, _) in driven.keys() {
        match rows.owner(section) {
            Some(plan::Owner::Family(family)) => assert!(
                rows.backend(family.name()).is_some(),
                "{section} is published for {family:?}, which nothing prepares"
            ),
            Some(_) => {}
            None => panic!("{section} has no publisher"),
        }
    }

    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let published =
        dashboard_base::prepared::gates::current(&store, &key).expect("the store's decoded gates");
    let family = |name: &str| independent::coverage_section(name, 1);
    let walk = crate::prepared::coverage::coverage_section(1);
    for (product, pages) in [
        (
            "event-feed",
            [
                "event_feed",
                "focus",
                "groups",
                "ideas",
                "nukes",
                "projects",
                "subjects",
            ]
            .map(family)
            .to_vec(),
        ),
        (
            "scoreboard",
            [
                "air",
                "army",
                "casualties",
                "factories",
                "identities",
                "navy",
                "nukes",
                "production",
                "stability",
                "territory",
            ]
            .map(family)
            .into_iter()
            .chain([walk.clone()])
            .collect(),
        ),
        ("army-size", ["army", "identities"].map(family).to_vec()),
        ("production-line-detail", vec![family("production-lines")]),
    ] {
        let pinned = query_pages(&published, 1, product).unwrap();
        assert_eq!(
            pinned
                .iter()
                .map(|(section, _)| section.clone())
                .collect::<BTreeSet<_>>(),
            pages.into_iter().collect::<BTreeSet<_>>(),
            "{product}"
        );
        // Ordinary preparation is the row driver's: a segment family's page stays unpublished.
        for (section, page) in &pinned {
            let segments = ["stability", "supply", "army"]
                .iter()
                .any(|family| *section == independent::coverage_section(family, 1));
            assert_eq!(page.is_some(), !segments, "{product} pins {section}");
        }
    }
    for product in ["stability-parts", "event-feed", "resource-balance"] {
        let coverage = product_coverage(&source, None, 1, product, &flag)
            .unwrap()
            .unwrap_or_else(|| panic!("{product} has no coverage"));
        assert_eq!(
            (coverage.first_hour, coverage.last_hour, coverage.complete),
            (10, 11, true),
            "{product}"
        );
    }

    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("listed");
    let listed = independent::with_test_only(&independent::row_names(), || {
        let listed = run(&source, None, &flag, |_| {}).unwrap();
        assert_eq!(reported(&listed), reported(&accepted));
        payloads(&source, None)
    });
    assert_payloads_equal(
        &without_checkpoint(driven),
        &without_checkpoint(listed),
        "driver versus every row implementation",
    );
}
