//! The parked-audio pickup package and the contract the replay side compiles
//! say the same thing, checked from the side that is allowed to read both.
//!
//! Lane VD left `replay_app`'s world-audio gates paired with the work orders
//! that close them, and the package those orders live in is a JSON file under
//! `runs/`. Audio is parked by the owner until every non-audio item lands, so
//! nobody is watching the two: a brief deleted from the file while a gate still
//! names it, or a brief added that closes no gate, would go unnoticed for
//! months and strand whoever picks the work up.
//!
//! `replay_app` cannot check this itself. A replay-side file that names a path
//! under the acquisition tree is a filesystem reach across the boundary, which
//! `game_ui/tests/replay_dependency_boundary.rs` refuses, and opening the file
//! through a constant rather than a literal only hides the reach from that
//! gate's text scan. So the check lives here, where reading either tree is
//! native.
//!
//! # Why this test parses nothing
//!
//! An earlier version read the ids out of `world_audio_law.rs` as TEXT. Three
//! review rounds each found one more way that can be fooled: a commented-out
//! decoy declaration, an identifier standing where a literal was expected with
//! the old value left in a trailing comment, and a use whose line was
//! character-for-character the allowed one while the call around it opened the
//! file. Each fix bought one round. The class is open-ended, so it is closed
//! structurally rather than patched again: the contract moved to
//! `portable_engine_evidence::vd_audio_pickup`, a crate owned by neither tree,
//! both sides LINK it, and this test compares COMPILED values against the JSON.
//! There is no longer any source text to misread.
//!
//! That move also took the path string out of the replay tree, so the boundary
//! gate needs no exemption for it at all - the allowlist entry it briefly
//! carried is gone.
//!
//! # What production change would make this fail?
//!
//! Deleting a brief from the package, adding one, renaming an id, editing
//! `OPEN_BRIEFS`, or pointing `WORK_ORDERS` at a file that is not the package.

use portable_engine_evidence::vd_audio_pickup::{
    owning_lane, OPEN_BRIEFS, RESIDUALS_HELD_ELSEWHERE, WORK_ORDERS,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/extractor/native/replay_decoder.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

#[test]
fn vd_audio_pickup_binding() {
    // The path the replay side prints IS the file opened here: one constant,
    // linked by both, so the two cannot drift into naming different files.
    let orders: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join(WORK_ORDERS)).unwrap_or_else(|error| {
            panic!("the replay side points at {WORK_ORDERS}, which does not read: {error}")
        }),
    )
    .expect("the work orders parse");

    let listed: Vec<String> = orders["bar_2_engine_mechanism_fidelity"]["work_orders"]
        .as_array()
        .expect("bar_2_engine_mechanism_fidelity.work_orders is an array")
        .iter()
        .map(|entry| {
            entry["id"]
                .as_str()
                .expect("every brief has an id")
                .to_string()
        })
        .collect();
    let briefs: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
    assert_eq!(
        briefs.len(),
        listed.len(),
        "two briefs in {WORK_ORDERS} share an id: {listed:?}"
    );

    let open: BTreeSet<&str> = OPEN_BRIEFS.iter().copied().collect();
    assert_eq!(
        briefs, open,
        "{WORK_ORDERS} and vd_audio_pickup::OPEN_BRIEFS disagree about which work orders are \
         open. Closing one is an edit under bar_2_engine_mechanism_fidelity.completion_standard, \
         which requires cited a729 evidence - not a deletion from either side"
    );

    let rows = orders["residuals_that_stand"]
        .as_array()
        .expect("residuals_that_stand is an array");
    let row_ids: Vec<&str> = rows
        .iter()
        .map(|row| row["id"].as_str().expect("every residual has an id"))
        .collect();
    assert_eq!(
        row_ids.iter().collect::<BTreeSet<_>>().len(),
        row_ids.len(),
        "two residuals in {WORK_ORDERS} share an id: {row_ids:?}"
    );

    // Every residual a gate may point at instead of a brief is written up, and
    // held by the lane the whitelist names.
    let licensed: BTreeSet<&str> = RESIDUALS_HELD_ELSEWHERE
        .iter()
        .map(|(residual, _)| *residual)
        .collect();
    assert_eq!(
        licensed.len(),
        RESIDUALS_HELD_ELSEWHERE.len(),
        "RESIDUALS_HELD_ELSEWHERE licenses an id twice: {RESIDUALS_HELD_ELSEWHERE:?}"
    );
    for (residual, lane) in RESIDUALS_HELD_ELSEWHERE {
        let matching: Vec<&Value> = rows
            .iter()
            .filter(|row| row["id"].as_str() == Some(residual))
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "{residual} is licensed to {lane} and {WORK_ORDERS} carries {} rows for it",
            matching.len()
        );
        let owner = matching[0]
            .get("held_by")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{residual} is licensed to {lane} and has no held_by"));
        assert_eq!(
            owning_lane(owner),
            owning_lane(lane),
            "{residual} is licensed to {lane} and {WORK_ORDERS} says {owner:?}"
        );
    }

    // And nothing else claims to be held elsewhere. An unlicensed held_by is a
    // residual dressed up as one a gate may point at.
    for row in rows {
        let id = row["id"].as_str().expect("every residual has an id");
        if row.get("held_by").is_none() {
            continue;
        }
        assert!(
            licensed.contains(id),
            "{id} carries held_by in {WORK_ORDERS}, which RESIDUALS_HELD_ELSEWHERE does not \
             license"
        );
    }
}
