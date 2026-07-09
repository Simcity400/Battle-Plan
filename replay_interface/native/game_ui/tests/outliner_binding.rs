//! D3 outliner-population gates (REPLAY_PRODUCTION_PLAN Phase D, item D3).
//!
//! These drive the packed snapshot-flavor fixtures through the real
//! `replay_core` `ReplaySource`/`WorldTimeline` and assert that
//! `game_ui::binding::outliner_unit_lists_from_frame` populates the
//! army/navy/air lists with the exact fixture ground-truth row counts and row
//! key strings, across multiple POV countries and at multiple probed hours.
//!
//! Expected values are derived from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs`), never by running the code under test:
//!   * `fx_division_movement` — divisions `101:1` (owner GER) and `201:1`
//!     (owner SOV) present at every tick (the base snapshot's two divisions;
//!     div 101 is repositioned but its identity/owner never change).
//!   * `fx_naval_air_activity` — one fleet task force `501:1` (owner ENG) and
//!     one air wing `800:1` (owner ENG); the base snapshot's divisions
//!     `101:1`/`201:1` are still present.

use game_ui::binding::{
    outliner_binding_from_frame, outliner_unit_lists_from_frame, OUTLINER_AIR_ICON,
    OUTLINER_ARMY_ICON, OUTLINER_NAVY_ICON,
};
use replay_core::{open_recording, ViewStatus, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

fn uids(rows: &[game_ui::binding::OutlinerUnitRow]) -> Vec<&str> {
    rows.iter().map(|r| r.uid.as_str()).collect()
}

/// GATE 1a — army rows are owner-filtered per POV across every hour.
/// `fx_division_movement`: div `101:1` belongs to GER, `201:1` to SOV.
#[test]
fn army_rows_owner_filtered_per_pov() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for &hour in &hours {
        let frame = tl.frame(hour).unwrap();

        let ger = outliner_unit_lists_from_frame(&frame, "GER");
        assert_eq!(ger.army_status, ViewStatus::Present);
        assert_eq!(uids(&ger.army), vec!["101:1"], "GER army @h{hour}");
        // GER has no fleets/wings in this fixture (empty arrays => Present).
        assert!(ger.navy.is_empty(), "GER navy @h{hour}");
        assert!(ger.air.is_empty(), "GER air @h{hour}");

        let sov = outliner_unit_lists_from_frame(&frame, "SOV");
        assert_eq!(uids(&sov.army), vec!["201:1"], "SOV army @h{hour}");
        assert!(sov.navy.is_empty(), "SOV navy @h{hour}");
        assert!(sov.air.is_empty(), "SOV air @h{hour}");

        // A POV with no units gets empty lists (never fabricated rows).
        let eng = outliner_unit_lists_from_frame(&frame, "ENG");
        assert!(eng.army.is_empty(), "ENG army @h{hour}");
    }

    // Each army icon is the FILE-cited theatre army shield.
    let frame = tl.frame(hours[0]).unwrap();
    let ger = outliner_unit_lists_from_frame(&frame, "GER");
    assert_eq!(ger.army[0].icon_sprite, OUTLINER_ARMY_ICON);
}

/// GATE 1b — navy/air rows are owner-filtered per POV; the divisions from the
/// base snapshot still populate the army branch. Two distinct POVs (ENG has the
/// naval/air units; GER has only an army division).
#[test]
fn navy_air_rows_owner_filtered_per_pov() {
    let tl = fixture("fx_naval_air_activity");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for &hour in &hours {
        let frame = tl.frame(hour).unwrap();

        let eng = outliner_unit_lists_from_frame(&frame, "ENG");
        assert_eq!(eng.navy_status, ViewStatus::Present);
        assert_eq!(eng.air_status, ViewStatus::Present);
        assert_eq!(uids(&eng.navy), vec!["501:1"], "ENG navy @h{hour}");
        assert_eq!(uids(&eng.air), vec!["800:1"], "ENG air @h{hour}");
        // ENG owns no division in the base snapshot.
        assert!(eng.army.is_empty(), "ENG army @h{hour}");

        let ger = outliner_unit_lists_from_frame(&frame, "GER");
        assert_eq!(uids(&ger.army), vec!["101:1"], "GER army @h{hour}");
        assert!(ger.navy.is_empty(), "GER navy @h{hour}");
        assert!(ger.air.is_empty(), "GER air @h{hour}");
    }

    let frame = tl.frame(hours[0]).unwrap();
    let eng = outliner_unit_lists_from_frame(&frame, "ENG");
    assert_eq!(eng.navy[0].icon_sprite, OUTLINER_NAVY_ICON);
    assert_eq!(eng.air[0].icon_sprite, OUTLINER_AIR_ICON);
}

/// GATE 1c — the flat `OutlinerBinding` fed to the layout compositor is the
/// army→navy→air concatenation with row text == UID and fail-closed `None`
/// progress. Uses `fx_naval_air_activity` @ ENG (army empty, navy+air one each).
#[test]
fn flat_binding_orders_army_navy_air_with_uid_text() {
    let tl = fixture("fx_naval_air_activity");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let binding = outliner_binding_from_frame(&frame, "ENG");

    let rows: Vec<(&str, &str, Option<f32>)> = binding
        .rows
        .iter()
        .map(|r| (r.icon_sprite.as_str(), r.text.as_str(), r.progress))
        .collect();
    // army (none) then navy `501:1` then air `800:1`.
    assert_eq!(
        rows,
        vec![
            (OUTLINER_NAVY_ICON, "501:1", None),
            (OUTLINER_AIR_ICON, "800:1", None),
        ]
    );
    assert!(!binding.collapsed);

    // GER POV: army only.
    let ger = outliner_binding_from_frame(&frame, "GER");
    let ger_rows: Vec<(&str, &str)> = ger
        .rows
        .iter()
        .map(|r| (r.icon_sprite.as_str(), r.text.as_str()))
        .collect();
    assert_eq!(ger_rows, vec![(OUTLINER_ARMY_ICON, "101:1")]);
}
