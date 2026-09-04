
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


#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn unitview_detail_panels_have_all_file_field_slots() {
    let root = game_root();
    let layouts = parse_unitview(&root);
    let report = inspect_unitview_layout(&layouts);

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
    assert!(
        report.land.runtime_grids.len() >= 5,
        "land runtime grids: {:?}",
        report.land.runtime_grids
    );

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
        assert_eq!(ger.display_name, None);
        assert!(ger.equipment_available, "equipment measured @h{hour}");
        assert!(ger.equipment.is_empty());

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

        let sov = land_unit_view_from_frame(&frame, "201:1")
            .unwrap_or_else(|| panic!("div 201:1 present @h{hour}"));
        assert_eq!(sov.owner.as_deref(), Some("SOV"));
        assert_eq!(sov.location_province, Some(2000));
        assert_eq!(sov.moving, Some(false));

        assert!(land_unit_view_from_frame(&frame, "999:1").is_none());
    }
}

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
        assert_eq!(tf.mission.as_deref(), Some("PATROL"));
        assert_eq!(tf.dominant_hull_class.as_deref(), Some("capital"));
        assert_eq!(tf.location_province, Some(3000));
        assert_eq!(tf.display_name, None);
        assert_eq!(
            tf.ship_count,
            Some(8 + tick as i64),
            "ship_count @h{hour} (tick {tick})"
        );
        assert!(!tf.ships_available, "ships Absent @h{hour}");
        assert!(tf.ships.is_empty());

        assert!(naval_unit_view_from_frame(&frame, "404").is_none());
    }
}

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
        assert_eq!(wing.experience, None);
        assert_eq!(wing.manpower, None);
        assert_eq!(wing.plane_definition, None);

        assert!(air_wing_view_from_frame(&frame, "808").is_none());
    }
}

#[test]
fn detail_binders_are_domain_isolated() {
    let tl = fixture("fx_naval_air_activity");
    let frame = tl.frame(tl.first_hour()).unwrap();

    assert!(land_unit_view_from_frame(&frame, "101:1").is_some());
    assert!(land_unit_view_from_frame(&frame, "501").is_none());
    assert!(naval_unit_view_from_frame(&frame, "101:1").is_none());
    assert!(air_wing_view_from_frame(&frame, "501").is_none());
    assert_eq!(
        frame.wings_status(),
        ViewStatus::Present,
        "air wings view present"
    );
}
