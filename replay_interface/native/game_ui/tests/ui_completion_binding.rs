
use game_ui::binding::{
    clock_binding_from_frame, format_topbar_date_english, playback_speed_button,
    state_tooltip_from_frame,
};
use game_ui::loc::Localisation;
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

fn english_month_loc() -> Localisation {
    let mut loc = Localisation::default();
    for (key, value) in [
        ("January", "January"),
        ("February", "February"),
        ("March", "March"),
        ("April", "April"),
        ("May", "May"),
        ("June", "June"),
        ("July", "July"),
        ("August", "August"),
        ("September", "September"),
        ("October", "October"),
        ("November", "November"),
        ("December", "December"),
    ] {
        loc.entries.insert(key.to_string(), value.to_string());
    }
    loc
}

#[test]
fn clock_binding_matches_recording_date_and_playback_speed() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);
    let loc = english_month_loc();

    for &hour in &hours {
        let frame = tl.frame(hour).unwrap();
        let raw_date = frame.date().expect("fixture frame carries a date");
        let expected_date =
            format_topbar_date_english(raw_date).expect("recording date is well-formed");
        let expected_date = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ]
        .into_iter()
        .fold(expected_date, |date, month| {
            date.replace(month, &format!("${month}$"))
        });

        for step in 0..=6usize {
            let clock = clock_binding_from_frame(&frame, step, &loc);
            assert_eq!(
                clock.date_time.as_deref(),
                Some(expected_date.as_str()),
                "clock date @hour {hour}"
            );
            assert_eq!(
                clock.speed,
                Some(playback_speed_button(step)),
                "clock speed @step {step}"
            );
        }
    }
}

#[test]
fn clock_date_is_fail_closed_on_bad_date() {
    assert_eq!(format_topbar_date_english("not-a-date"), None);
}

#[test]
fn state_tooltip_owner_controller_flip_at_change_hour() {
    let tl = fixture("fx_owner_controller_flip");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    let before = state_tooltip_from_frame(&tl.frame(hours[4]).unwrap(), 10);
    assert!(before.available);
    assert_eq!(before.owner.as_deref(), Some("GER"));
    assert_eq!(before.controller.as_deref(), Some("GER"));

    let after = state_tooltip_from_frame(&tl.frame(hours[5]).unwrap(), 10);
    assert_eq!(after.owner.as_deref(), Some("SOV"));
    assert_eq!(after.controller.as_deref(), Some("SOV"));

    let last = state_tooltip_from_frame(&tl.frame(hours[11]).unwrap(), 10);
    assert_eq!(last.owner.as_deref(), Some("SOV"));
    assert_eq!(last.controller.as_deref(), Some("SOV"));
}

#[test]
fn state_tooltip_absent_state_is_fail_closed() {
    let tl = fixture("fx_owner_controller_flip");
    let hours = tl.hours().to_vec();
    let missing = state_tooltip_from_frame(&tl.frame(hours[0]).unwrap(), 99999);
    assert!(missing.available, "owner view is present for this frame");
    assert_eq!(missing.owner, None);
    assert_eq!(missing.controller, None);
    assert_eq!(missing.state_id, 99999);
}
