
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
        assert!(ger.navy.is_empty(), "GER navy @h{hour}");
        assert!(ger.air.is_empty(), "GER air @h{hour}");

        let sov = outliner_unit_lists_from_frame(&frame, "SOV");
        assert_eq!(uids(&sov.army), vec!["201:1"], "SOV army @h{hour}");
        assert!(sov.navy.is_empty(), "SOV navy @h{hour}");
        assert!(sov.air.is_empty(), "SOV air @h{hour}");

        let eng = outliner_unit_lists_from_frame(&frame, "ENG");
        assert!(eng.army.is_empty(), "ENG army @h{hour}");
    }

    let frame = tl.frame(hours[0]).unwrap();
    let ger = outliner_unit_lists_from_frame(&frame, "GER");
    assert_eq!(ger.army[0].icon_sprite, OUTLINER_ARMY_ICON);
}

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
        assert_eq!(uids(&eng.navy), vec!["501"], "ENG navy @h{hour}");
        assert_eq!(uids(&eng.air), vec!["800"], "ENG air @h{hour}");
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
    assert_eq!(
        rows,
        vec![
            (OUTLINER_NAVY_ICON, "501", None),
            (OUTLINER_AIR_ICON, "800", None),
        ]
    );
    assert!(!binding.collapsed);

    let ger = outliner_binding_from_frame(&frame, "GER");
    let ger_rows: Vec<(&str, &str)> = ger
        .rows
        .iter()
        .map(|r| (r.icon_sprite.as_str(), r.text.as_str()))
        .collect();
    assert_eq!(ger_rows, vec![(OUTLINER_ARMY_ICON, "101:1")]);
}
