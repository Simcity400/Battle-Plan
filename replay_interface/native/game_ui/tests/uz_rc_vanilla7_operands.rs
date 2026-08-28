//! What rc_vanilla7 can and cannot settle about the exact ability panel.
//!
//! `extractor/native/runs/uz_ability_panel/READ_LAW.md` section 6 lists the
//! operands the derivation needs. This gate counts which of them the typed
//! replay world actually carries over the capture's 238 hours, so section 9's
//! rc_vanilla8 expectation rests on numbers rather than on a reading of the
//! recorder arguments. Scheduling a domain is not carrying its rows: three of
//! the four domains this gate looks for were scheduled and arrive empty.

use replay_core::{open_recording, Leaf, WorldTimeline};
use std::path::PathBuf;

/// A worktree carries no `captures/`, so the retained corpus is addressed the
/// way `map_viewer::grading_law` addresses its own: an env override over the
/// main checkout's path.
fn capture() -> PathBuf {
    std::env::var_os("BATTLE_PLAN_RECORDING_CAPTURES_ROOT").or_else(|| std::env::var_os("HOI4_RECORDING_CAPTURES_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"captures"))
        .join("rc_vanilla7/native_session.battleplan")
}

#[test]
fn rc_vanilla7_carries_one_of_the_evaluators_operands_and_no_evaluator_context() {
    let path = capture();
    assert!(
        path.is_file(),
        "the operand coverage gate requires the retained capture at {}",
        path.display()
    );
    let timeline = WorldTimeline::open(open_recording(&path).expect("open rc_vanilla7"))
        .expect("world timeline");
    let hours = timeline.hours().to_vec();
    assert_eq!(hours.len(), 238, "rc_vanilla7 is a 238-hour capture");

    let mut dlc_masks = std::collections::BTreeSet::new();
    let mut hours_with_focus = 0usize;
    let mut hours_with_flags = 0usize;
    let mut hours_with_leaders = 0usize;
    let mut hours_with_assignments = 0usize;
    let mut max_leaders_in_an_hour = 0usize;

    for hour in &hours {
        let frame = timeline.frame(*hour).expect("frame");
        if let Leaf::Measured(mask) = frame.session_header().dlc_mask {
            dlc_masks.insert(mask);
        }
        if frame
            .focus_by_country()
            .values()
            .any(|view| matches!(view.completed, Leaf::Measured(_)))
        {
            hours_with_focus += 1;
        }
        if frame
            .scripted_gui()
            .country_flag_states
            .values()
            .any(|states| matches!(states, Leaf::Measured(_)))
        {
            hours_with_flags += 1;
        }
        let leaders: usize = frame
            .character_rosters_exact()
            .values()
            .map(|roster| roster.unit_leaders.len())
            .sum();
        if leaders > 0 {
            hours_with_leaders += 1;
            max_leaders_in_an_hour = max_leaders_in_an_hour.max(leaders);
        }
        if frame.character_rosters_exact().values().any(|roster| {
            roster
                .unit_leaders
                .iter()
                .any(|leader| leader.assignment.is_some())
        }) {
            hours_with_assignments += 1;
        }
    }

    println!(
        "UZ operand coverage over rc_vanilla7: hours={} dlc_masks={dlc_masks:?} \
         hours_with_focus={hours_with_focus} hours_with_flags={hours_with_flags} \
         hours_with_leaders={hours_with_leaders} \
         hours_with_assignments={hours_with_assignments} \
         max_leaders_in_an_hour={max_leaders_in_an_hour}",
        hours.len()
    );

    assert_eq!(
        dlc_masks,
        std::collections::BTreeSet::from([0x0003_EDFF]),
        "the DLC word is a session constant, and this is the capture's value"
    );
    assert_eq!(
        hours_with_focus, 237,
        "d12 carries a measured completed-focus set on all but one hour"
    );
    assert_eq!(
        hours_with_flags, 0,
        "d51's country flag states never reach the typed world in this capture"
    );
    assert_eq!(
        hours_with_leaders, 0,
        "d47's unit-leader family is absent from every hour of this capture"
    );
    assert_eq!(hours_with_assignments, 0);
    assert_eq!(max_leaders_in_an_hour, 0);
}
