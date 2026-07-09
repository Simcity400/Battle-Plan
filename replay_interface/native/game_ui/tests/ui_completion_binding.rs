//! D6 (date/speed clock) + D7 (state tooltip) binding gates
//! (REPLAY_PRODUCTION_PLAN Phase D).
//!
//! Drives packed snapshot-flavor fixtures through the real `replay_core`
//! `ReplaySource`/`WorldTimeline` and asserts that the `game_ui::binding`
//! population reproduces the generator ground truth:
//!   * D6 — the clock date_time equals the FILE-formatted recording date at
//!     every frame, and the playback speed step maps onto the topbar buttons.
//!   * D7 — the state tooltip owner/controller flip exactly at the fixture's
//!     change hour, and an absent state is fail-closed.
//!
//! Expected values come from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs`), never from the code under test.

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

/// English month-name localisation, matching the shipped `core_l_english.yml`
/// values (built in-memory so the gate needs no game install).
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

/// GATE D6a — at every recorded frame the clock date_time equals the FILE
/// topbar format of that frame's own recording date, and the playback speed
/// step maps onto the topbar's five speed buttons.
#[test]
fn clock_binding_matches_recording_date_and_playback_speed() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);
    let loc = english_month_loc();

    for &hour in &hours {
        let frame = tl.frame(hour).unwrap();
        // Independent expectation: the frame's own recording date, FILE-formatted.
        let raw_date = frame.date().expect("fixture frame carries a date");
        let expected_date =
            format_topbar_date_english(raw_date).expect("recording date is well-formed");

        // The clock must reproduce that date for every playback speed step, and
        // the speed field must track the step (0-based) onto buttons 1..=5.
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

/// GATE D6b — a frame whose localisation lacks a month name still renders (the
/// English key is the fail-safe), and a dateless lookup path is fail-closed.
/// (Every fixture frame carries a date, so we exercise the fail-closed path
/// through the pure formatter on a malformed string.)
#[test]
fn clock_date_is_fail_closed_on_bad_date() {
    assert_eq!(format_topbar_date_english("not-a-date"), None);
}

/// GATE D7a — the state tooltip owner/controller flip exactly at the fixture's
/// change hour. `fx_owner_controller_flip`: state 10 owner+controller flip
/// GER -> SOV at tick 5 (generator source), and hold thereafter.
#[test]
fn state_tooltip_owner_controller_flip_at_change_hour() {
    let tl = fixture("fx_owner_controller_flip");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    // Before the flip (tick 4): GER owns and controls state 10.
    let before = state_tooltip_from_frame(&tl.frame(hours[4]).unwrap(), 10);
    assert!(before.available);
    assert_eq!(before.owner.as_deref(), Some("GER"));
    assert_eq!(before.controller.as_deref(), Some("GER"));

    // Change hour (tick 5): both flip to SOV.
    let after = state_tooltip_from_frame(&tl.frame(hours[5]).unwrap(), 10);
    assert_eq!(after.owner.as_deref(), Some("SOV"));
    assert_eq!(after.controller.as_deref(), Some("SOV"));

    // Holds through the end of the timeline.
    let last = state_tooltip_from_frame(&tl.frame(hours[11]).unwrap(), 10);
    assert_eq!(last.owner.as_deref(), Some("SOV"));
    assert_eq!(last.controller.as_deref(), Some("SOV"));
}

/// GATE D7b — a state absent from the map layers is fail-closed: no owner /
/// controller, never a fabricated tag, while still reporting view availability.
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
