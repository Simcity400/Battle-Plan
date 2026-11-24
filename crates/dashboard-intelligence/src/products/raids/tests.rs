use super::{
    Course, RaidPreparationRun, RaidVictimSource, coalesce_spans, date_bounds, disappearance, exact_at, outcome_label, outcome_missing, phase_label, province_of, scope_of, victim_source,
};
use engine_native_air::craid_instance as native;
use replay_data::rows::RaidScope;
use replay_lanes::lanes::{Cell, Step};

fn step(hour: i32, phase: i32, outcome: i32, travelled: Option<i64>, preparation: i64) -> Step {
    let travelled = travelled.map_or(Cell::Missing, Cell::Number);
    Step { hour, cells: Some(Box::new([Cell::Number(7), Cell::Number(i64::from(phase)), Cell::Number(i64::from(outcome)), travelled, Cell::Number(preparation)])) }
}

fn gap(hour: i32) -> Step {
    Step { hour, cells: None }
}

#[test]
fn a_raids_phases_are_runs_it_launches_at_its_first_hour_in_progress_and_wins_only_once_ended_with_success() {
    let steps = [
        step(10, native::PHASE_ASSEMBLING, native::OUTCOME_NONE, None, 0),
        step(20, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE, Some(0), 0),
        step(30, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(500), 0),
    ];
    let course = Course::read(&steps, 50);
    assert_eq!(course.runs, vec![(10, 19, 1, 0), (20, 29, 4, 0), (30, 50, 5, 3)]);
    assert_eq!((course.launched, course.ended, course.raider, course.phase, course.outcome), (Some(20), Some(30), Some(7), Some(5), Some(3)));
    assert!(course.succeeded());
    let before = Course::read(&steps, 25);
    assert_eq!(before.runs.last(), Some(&(20, 25, 4, 0)));
    assert!(before.launched.is_some() && !before.succeeded());
    let flew = Course::read(&[step(10, native::PHASE_PREPARED, 0, Some(0), 0), step(15, native::PHASE_PREPARED, 0, Some(250), 0), step(20, native::PHASE_ENDED, 2, Some(250), 0)], 29);
    assert_eq!((flew.launched, flew.ended), (Some(15), Some(20)));
    assert!(!flew.succeeded(), "a limited success is no success");
}

#[test]
fn hours_the_row_is_not_shown_end_a_run_and_are_no_phase() {
    let course = Course::read(&[step(10, native::PHASE_PREPARING, 0, None, 3), gap(15), step(18, native::PHASE_PREPARING, 0, None, 9)], 20);
    assert_eq!(course.runs, vec![(10, 14, 2, 0), (18, 20, 2, 0)]);
    assert_eq!(Course::read(&[gap(10)], 20), Course::default());
}

#[test]
fn preparation_hours_are_runs_of_a_steady_change_per_hour_and_never_span_a_gap() {
    let steps: Vec<Step> = (10..=13).map(|hour| step(hour, native::PHASE_PREPARING, 0, None, i64::from(hour - 5))).chain([gap(14), step(16, native::PHASE_PREPARING, 0, None, 30)]).collect();
    let course = Course::read(&steps, 19);
    assert_eq!(course.preparation, vec![RaidPreparationRun(10, 13, 5, 1), RaidPreparationRun(16, 19, 30, 0)]);
    let held = Course::read(&[step(10, native::PHASE_PREPARING, 0, None, 4), step(13, native::PHASE_PREPARING, 0, None, 9)], 13);
    assert_eq!(held.preparation, vec![RaidPreparationRun(10, 12, 4, 0), RaidPreparationRun(13, 13, 9, 0)]);
    assert!(Course::read(&[step(10, native::PHASE_IN_PROGRESS, 0, None, 40)], 12).preparation.is_empty());
}

#[test]
fn a_target_is_no_scope_while_a_cell_is_not_recorded_and_its_province_follows_the_games_priority() {
    let cells = |captor: Option<Option<i64>>| {
        move |at: usize| match at {
            4 => Some(Some(9)),
            5 => Some(Some(44)),
            6 => Some(Some(1)),
            8 => captor,
            _ => Some(Some(0)),
        }
    };
    let provincial = scope_of(cells(Some(None))).expect("every cell recorded");
    assert_eq!((provincial.leader_country, province_of(&provincial)), (None, Some(44)));
    assert_eq!(scope_of(cells(Some(Some(8)))).map(|scope| scope.leader_country), Some(Some(8)));
    assert_eq!(scope_of(cells(None)), None, "an unrecorded captor is not a captor of none");
}

#[test]
fn outcomes_keep_full_native_vocabulary_label_unsupported_codes_and_never_mark_vanished_as_canceled() {
    let mut loc = |key: &'static str| Some(key.to_owned());
    for (code, expected) in [
        (native::OUTCOME_NONE, "RAID_INST_OUTCOME_NONE"),
        (native::OUTCOME_FAILURE, "RAID_INST_OUTCOME_FAILURE"),
        (native::OUTCOME_LIMITED_SUCCESS, "RAID_INST_OUTCOME_LIMITED_SUCCESS"),
        (native::OUTCOME_SUCCESS, "RAID_INST_OUTCOME_SUCCESS"),
        (native::OUTCOME_CRITICAL_SUCCESS, "RAID_INST_OUTCOME_CRITICAL_SUCCESS"),
        (native::OUTCOME_CANCELED, "RAID_INST_OUTCOME_CANCELED"),
    ] {
        assert_eq!(outcome_label(code, &mut loc).as_deref(), Some(expected));
    }
    assert_eq!(outcome_label(99, &mut loc).as_deref(), Some("Unsupported outcome (99)"));
    assert_eq!(phase_label(-2, &mut loc).as_deref(), Some("Unsupported phase (-2)"));
    assert_eq!(outcome_missing(true, false, Some(native::OUTCOME_LIMITED_SUCCESS)), None);
    assert_eq!(outcome_missing(true, false, Some(native::OUTCOME_CANCELED)), None);
    assert_eq!(outcome_missing(true, false, Some(99)).as_deref(), Some("Unsupported raid outcome (99)"));
    assert_eq!(outcome_missing(true, false, Some(native::OUTCOME_NONE)).as_deref(), Some("raid outcome was not recorded"));
    assert_eq!(outcome_missing(false, true, Some(native::OUTCOME_NONE)).as_deref(), Some("raid outcome was not recorded"));
    assert_eq!(outcome_missing(false, false, Some(native::OUTCOME_NONE)), None);
}

#[test]
fn phase_dates_are_exact_only_when_adjacent_reads_pin_one_hour_and_bounded_otherwise() {
    let runs = [(10, 14, 1, 0), (15, 24, 2, 0), (25, 30, 5, 3)];
    let exact_spans = coalesce_spans(vec![(5, 19), (20, 35)]);
    let exact = date_bounds(&runs, 10, &exact_spans, &exact_spans);
    assert_eq!(exact, vec![(true, Some(9)), (true, Some(14)), (true, Some(24))]);
    assert!(exact_at(25, 10, true, &runs, &exact, &exact_spans));

    let unread_middle = coalesce_spans(vec![(10, 12), (15, 18), (25, 30)]);
    let bounded = date_bounds(&runs, 10, &unread_middle, &unread_middle);
    assert_eq!(bounded, vec![(false, None), (false, Some(12)), (false, Some(18))]);
    assert!(!exact_at(25, 10, false, &runs, &bounded, &unread_middle));

    let clipped = date_bounds(&runs, 5, &exact_spans, &exact_spans);
    assert_eq!(clipped[0], (false, None));
}

#[test]
fn disappearance_distinguishes_vanished_unread_from_live_and_ended_raids() {
    let complete = vec![(10, 20), (25, 30)];
    let available = vec![(10, 20), (25, 30)];
    assert_eq!(disappearance(true, 19, None, &complete, &available), (None, false));
    assert_eq!(disappearance(false, 19, Some(20), &complete, &available), (Some(20), true));
    assert_eq!(disappearance(false, 20, None, &complete, &available), (Some(25), false));
    assert_eq!(disappearance(false, 30, None, &complete, &available), (None, false));
}

#[test]
fn victim_source_separates_fixed_targets_from_mutable_controllers() {
    let fixed_building = RaidScope { building_type: 9, building_country: 4, ..RaidScope::default() };
    let fixed_leader = RaidScope { leader_country: Some(7), ..RaidScope::default() };
    let mutable_province = RaidScope { province: 44, ..RaidScope::default() };
    let mutable_building = RaidScope { building_type: 9, building_location: 44, building_flags: 1, ..RaidScope::default() };
    assert_eq!(victim_source(&fixed_building, true), RaidVictimSource::FixedTarget);
    assert_eq!(victim_source(&fixed_leader, false), RaidVictimSource::FixedTarget);
    assert_eq!(victim_source(&mutable_province, true), RaidVictimSource::ControllerAtFirstEndedRead);
    assert_eq!(victim_source(&mutable_building, false), RaidVictimSource::LastObservedController);
}
