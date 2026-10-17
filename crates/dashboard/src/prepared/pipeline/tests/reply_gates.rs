//! A pending or partial prepared reply names the preparation status entries
//! it waits for (its `gate`), and the status lists the coverage of each
//! family prepared on its own beside the hourly walk's: a client re-asks when
//! the coverage a reply waits for moves, also while the walk accepts nothing.
use super::independent_groups::scenario;
use super::*;
use crate::prepared::coverage::{WALK, waiting};
use serde_json::{Value, json};

/// The whole prepared reply of `product` on `branch`, gate included.
fn envelope(source: &Source, product: &str, branch: &str) -> Value {
    envelope_params(source, product, json!({ "branchId": branch }))
}

fn envelope_params(source: &Source, product: &str, params: Value) -> Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    serde_json::from_str(&reply).unwrap()
}

fn assert_cold_groups(reply: &Value, first: i32) {
    let data = &reply["data"];
    assert!(data.is_object(), "{reply}");
    assert_eq!(data["coverage"]["firstHour"], first);
    assert_eq!(data["coverage"]["lastHour"], first - 1);
    assert_eq!(data["coverage"]["complete"], false);
    assert!(data["pending"].is_array());
    assert!(data["approximate"].is_array());
    assert!(
        !data["pending"].as_array().unwrap().is_empty()
            || !data["approximate"].as_array().unwrap().is_empty(),
        "unprepared hours are pending or sampled: {reply}"
    );
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Prepare with the hourly walk cancelled at its first commit, so only the
/// families prepared on their own are accepted.
fn stall_walk(source: &Source, replay: &Replay) {
    let flag = AtomicBool::new(false);
    let error = run_checked(
        source,
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
    assert_eq!(
        unprepared_blocks(source, None).unwrap(),
        Some(replay.index.len()),
        "the hourly walk accepted no block"
    );
}

/// A product's gate follows the coverage it waits for: the family prepared
/// on its own that gates it, each family of a joint product (the hourly
/// walk's for those it prepares), or the walk's. A pending reply names the
/// families it found short; one complete from a family's own coverage, while
/// the walk accepts nothing, names none.
#[test]
fn pending_replies_name_the_coverage_they_wait_for() {
    independent::with_test_only(&["groups"], || {
        assert_eq!(gate("country-groups"), ["groups"]);
        // The feed reads the groups (prepared on their own) and families the
        // walk prepares.
        assert_eq!(gate("event-feed"), ["groups", WALK]);
        independent::with_test_hourly(|| {
            assert_eq!(gate("country-groups"), [WALK]);
            assert_eq!(gate("event-feed"), [WALK]);
        });

        let replay = scenario(6, true);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("reply-gates");
        let flag = AtomicBool::new(false);
        for branch in ["1", "2"] {
            // Nothing prepared: the groups wait for their own coverage.
            let pending = envelope(&source, "country-groups", branch);
            assert_cold_groups(&pending, if branch == "1" { 100 } else { 20 });
            assert_eq!(pending["gate"], json!(["groups"]), "branch {branch}");
            let (_, short) = waiting(|| {
                product_coverage(&source, None, branch.parse().unwrap(), "event-feed", &flag)
                    .unwrap()
            });
            assert_eq!(
                short.into_iter().collect::<Vec<_>>(),
                ["groups", WALK],
                "branch {branch}: the feed waits for the groups and the walk"
            );
            independent::with_test_hourly(|| {
                let pending = envelope(&source, "country-groups", branch);
                assert_cold_groups(&pending, if branch == "1" { 100 } else { 20 });
                assert_eq!(pending["gate"], json!([WALK]), "branch {branch}: hourly");
            });
        }

        stall_walk(&source, &replay);
        for branch in ["1", "2"] {
            let answered = envelope(&source, "country-groups", branch);
            assert!(!answered["data"].is_null(), "branch {branch}: answered");
            assert!(
                answered.get("gate").is_none(),
                "branch {branch}: a complete reply waits for nothing: {answered}"
            );
            let (coverage, short) = waiting(|| {
                product_coverage(
                    &source,
                    None,
                    branch.parse().unwrap(),
                    "country-groups",
                    &flag,
                )
                .unwrap()
            });
            assert!(coverage.is_some_and(|coverage| coverage.complete));
            assert!(short.is_empty(), "branch {branch}: {short:?}");
            // The hourly walk's own products still wait for it.
            independent::with_test_hourly(|| {
                let pending = envelope(&source, "country-groups", branch);
                assert_cold_groups(&pending, if branch == "1" { 100 } else { 20 });
                assert_eq!(pending["gate"], json!([WALK]), "branch {branch}: hourly");
            });
        }
        clear(source);
    });
}

#[test]
fn roster_replies_follow_incomplete_identities_after_intelligence_completes() {
    independent::with_test_only(&["intelligence", "identities"], || {
        assert_eq!(gate("operatives-roster"), ["identities", "intelligence"]);
        independent::with_test_hourly(|| {
            assert_eq!(gate("operatives-roster"), [WALK]);
        });
        let replay = scenario(6, false);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("roster-identity-gate");
        let params = json!({ "branchId": "1", "tags": ["AAA"], "includeDeparted": false });
        let cold = envelope_params(&source, "operatives-roster", params.clone());
        assert_eq!(cold["gate"], json!(["identities", "intelligence"]));
        assert_eq!(cold["data"]["preparing"], json!(["countries"]));
        assert!(
            cold["data"]
                .as_object()
                .unwrap()
                .contains_key("selectionCoverage")
        );
        assert!(cold["data"]["selectionCoverage"].is_null());

        // Roster values can finish before the branch-wide holder selection.
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        for (family, last_hour, complete) in
            [("intelligence", 125, true), ("identities", 103, false)]
        {
            let coverage = Coverage {
                schema: SCHEMA,
                first_hour: 100,
                last_hour,
                complete,
            };
            writer
                .write_at(
                    &independent::coverage_section(family, 1),
                    0,
                    serde_json::to_vec(&coverage).unwrap().as_slice(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
        let pending = envelope_params(&source, "operatives-roster", params);
        assert_eq!(pending["gate"], json!(["identities"]));
        assert_eq!(pending["data"]["selectionCoverage"]["lastHour"], 103);
        assert_eq!(pending["data"]["preparing"], json!(["countries"]));
        clear(source);
    });
}

/// The status lists a family prepared on its own's coverage by branch as its
/// commits publish it, from its first block on (its early batches), what its
/// coverage pages hold and what a restored preparation lists.
#[test]
fn the_status_lists_each_family_own_coverage() {
    independent::with_test_only(&["groups"], || {
        // Branch 1's six blocks (hours 100 to 125), then branch 2's one.
        let replay = scenario(6, true);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("status-families");
        let flag = AtomicBool::new(false);
        let mut seen: Vec<(i32, i32, bool)> = Vec::new();
        let accepted = run(&source, None, &flag, |accepted| {
            if let Some(coverage) = accepted
                .families
                .get("groups")
                .and_then(|branches| branches.get("1"))
            {
                let extent = (coverage.first_hour, coverage.last_hour, coverage.complete);
                if !seen.contains(&extent) {
                    seen.push(extent);
                }
            }
        })
        .unwrap();
        // After branch 1's 1st, 2nd and 4th blocks, and at its end.
        assert_eq!(
            seen,
            [
                (100, 103, false),
                (100, 107, false),
                (100, 117, false),
                (100, 125, true)
            ],
            "the groups' coverage as they commit"
        );

        let (store, key) = super::super::super::files(&source, None).unwrap();
        let restored = restore(&source, None, &flag).unwrap().expect("prepared");
        for listed in [&accepted.families, &restored.families] {
            let branches = &listed["groups"];
            assert_eq!(branches.len(), 2, "both branches: {branches:?}");
            for (branch, coverage) in branches {
                let page = own_page(&store, &key, "groups", branch.parse().unwrap())
                    .unwrap()
                    .expect("the groups' coverage page");
                assert_eq!(
                    serde_json::to_value(coverage).unwrap(),
                    serde_json::to_value(&page.coverage).unwrap(),
                    "branch {branch}: the page's coverage"
                );
                assert!(coverage.complete, "branch {branch}");
            }
        }

        // The status reply carries them as the preparation progress holds them.
        {
            let progress = source.state::<super::super::super::Progress>();
            let mut progress = progress.lock().unwrap();
            progress.coverage = restored.coverage.clone();
            progress.families = restored.families.clone();
        }
        let status = serde_json::to_value(super::super::super::status(
            &source,
            &source.key().source_id,
            "status-families",
        ))
        .unwrap();
        assert_eq!(
            status["families"],
            serde_json::to_value(&restored.families).unwrap()
        );
        assert_eq!(status["families"]["groups"]["2"]["lastHour"], json!(23));
        clear(source);
    });
}
