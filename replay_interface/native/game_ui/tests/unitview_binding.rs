//! Selected-unit view layout and binding gates.
//!
//! Two gate classes:
//!   1. **Layout** — parse vanilla `interface/unitview.gui` through the shared
//!      game_ui `.gui` parser and assert the three selected-unit detail panels
//!      (`army_division_stats_view` / `ship_stats_window` /
//!      `ship_stats_air_wing_entry`) exist and carry their FILE field slots,
//!      with runtime grids classified as documented skips. Skipped when the game
//!      root is absent (same convention as `widget_layout.rs`).
//!   2. **Binding** — drive `fx_division_movement` / `fx_naval_air_activity`
//!      through the real `replay_core` `WorldTimeline` and assert the binding
//!      populates each panel's fields with the exact fixture ground truth.
//!
//! Expected binding values are derived from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs`), never by running the code under test:
//!   * `fx_division_movement` — `division(101, 1, "GER", loc, 10, moving, …)` and
//!     `division(201, 1, "SOV", 2000, 20, false, None, 0.0)`; the `division`
//!     helper pins `template_name = "Fixture Infantry"`, `template_role =
//!     "infantry"`, `organisation = 42.0`, `strength = 0.95`, `equipment = []`.
//!     Div 101 `location_province`: `1000` (tick<4), `1001` (tick<8), else
//!     `2000`; `moving = tick<8`.
//!   * `fx_naval_air_activity` — task force `501` (owner ENG, fleet `500`,
//!     mission `PATROL` — the generator's `mission_name`, which is the engine
//!     enum token `replay_core`'s `MISSION_NAMES[1]` derives, upper case —
//!     `dominant_hull_class "capital"`, `location_province
//!     3000`, `ship_count = 8 + tick`, no inline `ships` array → Absent) and air
//!     wing `800` (owner ENG, name `Fighter Wing`, `count = 100 + tick`,
//!     mission `air_superiority`, strategic_region `301`, air_base `3000:1`,
//!     state_id `30`, `effective_planes = 90 + tick`; no experience/manpower/
//!     plane_definition → Absent).

use game_ui::gui::LayoutSet;
use game_ui::unitview::{
    air_wing_view_from_frame, inspect_unitview_layout, land_unit_view_from_frame,
    naval_unit_view_from_frame,
};
use replay_core::{open_recording, ViewStatus, WorldTimeline};
use std::path::{Path, PathBuf};

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

fn parse_unitview(root: &Path) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_file(
            &root.join("interface/unitview.gui"),
            "interface/unitview.gui",
        )
        .expect("parse vanilla unitview.gui");
    layouts
}

// ---------------------------------------------------------------------------
// LAYOUT gates
// ---------------------------------------------------------------------------

/// GATE L1 — the three selected-unit detail panels parse and every FILE field
/// slot is present (no missing slots), with runtime grids classified as skips.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn unitview_detail_panels_have_all_file_field_slots() {
    let root = game_root();
    let layouts = parse_unitview(&root);
    let report = inspect_unitview_layout(&layouts);

    // Land division detail panel.
    assert!(report.land.found, "army_division_stats_view found");
    assert_eq!(report.land.window, "army_division_stats_view");
    assert!(
        report.land.field_slots_missing.is_empty(),
        "land missing slots: {:?}",
        report.land.field_slots_missing
    );
    assert_eq!(
        report.land.field_slots_present,
        vec![
            "unit_title".to_string(),
            "experience".to_string(),
            "combat_width_text".to_string(),
            "support_header_label".to_string(),
        ]
    );
    // The land panel is grid-heavy (stats/support/regiments/medals/history).
    assert!(
        report.land.runtime_grids.len() >= 5,
        "land runtime grids: {:?}",
        report.land.runtime_grids
    );

    // Naval ship / task-force detail panel.
    assert!(report.naval.found, "ship_stats_window found");
    assert!(
        report.naval.field_slots_missing.is_empty(),
        "naval missing slots: {:?}",
        report.naval.field_slots_missing
    );
    assert_eq!(report.naval.field_slots_present.len(), 7);
    assert!(report
        .naval
        .field_slots_present
        .contains(&"ship_title".to_string()));

    // Air-wing entry row.
    assert!(report.air.found, "ship_stats_air_wing_entry found");
    assert!(
        report.air.field_slots_missing.is_empty(),
        "air missing slots: {:?}",
        report.air.field_slots_missing
    );
    assert_eq!(
        report.air.field_slots_present,
        vec!["name".to_string(), "count".to_string()]
    );
}

// ---------------------------------------------------------------------------
// BINDING gates
// ---------------------------------------------------------------------------

/// GATE B1 — the selected land division binds its fixture ground truth at every
/// hour: invariant template/owner/strength/org fields plus the per-tick
/// `location_province` / `moving` from the generator source. A POV-independent
/// selection by `div_uid`.
#[test]
fn land_division_panel_binds_fixture_ground_truth() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (tick, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();

        let ger = land_unit_view_from_frame(&frame, "101:1")
            .unwrap_or_else(|| panic!("div 101:1 present @h{hour}"));
        assert!(ger.available, "land view available @h{hour}");
        assert_eq!(ger.owner.as_deref(), Some("GER"), "@h{hour}");
        assert_eq!(ger.template_name.as_deref(), Some("Fixture Infantry"));
        assert_eq!(ger.template_role.as_deref(), Some("infantry"));
        assert_eq!(ger.template_id, Some(1));
        assert_eq!(ger.strength, Some(0.95));
        assert_eq!(ger.organisation, Some(42.0));
        // VIEW GAP: no division custom name surfaced.
        assert_eq!(ger.display_name, None);
        // equipment leaf is Measured-but-empty in the fixture.
        assert!(ger.equipment_available, "equipment measured @h{hour}");
        assert!(ger.equipment.is_empty());

        // Per-tick position + movement (generator source).
        let expected_loc = if tick < 4 {
            1000
        } else if tick < 8 {
            1001
        } else {
            2000
        };
        assert_eq!(
            ger.location_province,
            Some(expected_loc),
            "loc @h{hour} (tick {tick})"
        );
        assert_eq!(ger.moving, Some(tick < 8), "moving @h{hour} (tick {tick})");

        // The SOV division is selectable independently, always at province 2000.
        let sov = land_unit_view_from_frame(&frame, "201:1")
            .unwrap_or_else(|| panic!("div 201:1 present @h{hour}"));
        assert_eq!(sov.owner.as_deref(), Some("SOV"));
        assert_eq!(sov.location_province, Some(2000));
        assert_eq!(sov.moving, Some(false));

        // A non-existent selection fails closed to None.
        assert!(land_unit_view_from_frame(&frame, "999:1").is_none());
    }
}

/// GATE B2 — the selected naval task force binds its fixture ground truth:
/// invariant fleet/owner/mission/hull fields plus per-tick `ship_count`; the
/// `ships` composition is recording-Absent → fail-closed empty + unavailable.
#[test]
fn naval_task_force_panel_binds_fixture_ground_truth() {
    let tl = fixture("fx_naval_air_activity");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (tick, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();

        let tf = naval_unit_view_from_frame(&frame, "501")
            .unwrap_or_else(|| panic!("task force 501 present @h{hour}"));
        assert!(tf.available, "naval view available @h{hour}");
        assert_eq!(tf.owner.as_deref(), Some("ENG"));
        assert_eq!(tf.fleet_uid.as_deref(), Some("500"));
        // The mission name is the engine ENUM TOKEN, upper case. `replay_core`
        // derives it from `mission_enum` through its `MISSION_NAMES` table, the
        // fixture generator emits the same spelling, and the binding passes the
        // value through untransformed -- so the table's spelling is the only
        // correct one here. (Why this expectation was ever lower case is not
        // recorded: no lower-case `mission_name` appears anywhere in the
        // generator's history.)
        assert_eq!(tf.mission.as_deref(), Some("PATROL"));
        assert_eq!(tf.dominant_hull_class.as_deref(), Some("capital"));
        assert_eq!(tf.location_province, Some(3000));
        // VIEW GAP: no task-force custom name surfaced.
        assert_eq!(tf.display_name, None);
        // Per-tick ship count (generator source: 8 + tick).
        assert_eq!(
            tf.ship_count,
            Some(8 + tick as i64),
            "ship_count @h{hour} (tick {tick})"
        );
        // The task force carries no inline `ships` array → recording-Absent.
        assert!(!tf.ships_available, "ships Absent @h{hour}");
        assert!(tf.ships.is_empty());

        assert!(naval_unit_view_from_frame(&frame, "404").is_none());
    }
}

/// GATE B3 — the selected air wing binds its fixture ground truth: name / mission
/// / region / base / state and per-tick `plane_count` + `effective_planes`, with
/// recording-Absent experience/manpower/plane_definition failing closed to None.
#[test]
fn air_wing_panel_binds_fixture_ground_truth() {
    let tl = fixture("fx_naval_air_activity");
    let hours = tl.hours().to_vec();

    for (tick, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();

        let wing = air_wing_view_from_frame(&frame, "800")
            .unwrap_or_else(|| panic!("wing 800 present @h{hour}"));
        assert!(wing.available, "air view available @h{hour}");
        assert_eq!(wing.owner.as_deref(), Some("ENG"));
        assert_eq!(wing.display_name.as_deref(), Some("Fighter Wing"));
        assert_eq!(wing.mission_kind.as_deref(), Some("air_superiority"));
        assert_eq!(wing.strategic_region_id, Some(301));
        assert_eq!(wing.air_base.as_deref(), Some("3000:1"));
        assert_eq!(wing.state_id, Some(30));
        // Per-tick counts (generator source).
        assert_eq!(
            wing.plane_count,
            Some(100 + tick as i64),
            "plane_count @h{hour}"
        );
        assert_eq!(
            wing.effective_planes,
            Some(90 + tick as i64),
            "effective_planes @h{hour}"
        );
        // Recording-Absent fields fail closed.
        assert_eq!(wing.experience, None);
        assert_eq!(wing.manpower, None);
        assert_eq!(wing.plane_definition, None);

        assert!(air_wing_view_from_frame(&frame, "808").is_none());
    }
}

/// GATE B4 — cross-domain selection isolation: at one frame each detail binder
/// resolves only its own domain's unit and fails closed on a foreign UID.
#[test]
fn detail_binders_are_domain_isolated() {
    let tl = fixture("fx_naval_air_activity");
    let frame = tl.frame(tl.first_hour()).unwrap();

    // The base snapshot still carries the two land divisions.
    assert!(land_unit_view_from_frame(&frame, "101:1").is_some());
    // A land binder never resolves a naval/air UID, and vice versa.
    assert!(land_unit_view_from_frame(&frame, "501").is_none());
    assert!(naval_unit_view_from_frame(&frame, "101:1").is_none());
    assert!(air_wing_view_from_frame(&frame, "501").is_none());
    assert_eq!(
        frame.wings_status(),
        ViewStatus::Present,
        "air wings view present"
    );
}
