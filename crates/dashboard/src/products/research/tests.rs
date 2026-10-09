use super::{Country, MAX_BYTES, base_cost, check_budget, history_bytes, remaining};
use crate::ResearchOutcome;
use crate::test_fixtures::{CAMPAIGN, fixture_research_inputs, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{
        CountryLinkRow, ResearchAppliedBonusRow, ResearchInputRow, ResearchTechnologyRow, TableId,
        TableRows,
    },
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn input(handle: u32, progress: i64) -> ResearchInputRow {
    ResearchInputRow {
        country: 11,
        index: 0,
        technology_handle: handle,
        technology_present: 1,
        token: format!("tech_{handle}").into_bytes(),
        classifier: 5,
        progress_raw: progress,
        cost_raw: 10_000_000,
        cost_scale_raw: 100_000,
        research_speed_raw: 100_000,
        ..Default::default()
    }
}

fn observe(country: &mut Country, hour: i32, row: &ResearchInputRow) {
    country.observe_inputs((7, 7), hour, &[row], None, Some(&[]), None);
}
fn at(country: &Country, hour: i32) -> Option<u32> {
    let s = country.history.slots[0]
        .states
        .iter()
        .find(|s| s.first_hour <= hour && hour <= s.last_hour)?;
    s.details
        .as_ref()?
        .remaining_days
        .map(|days| days.saturating_sub(((hour - s.first_hour) / 24) as u32))
}

#[test]
fn countdown_uses_only_past_progress_and_compresses_daily_ticks() {
    let mut country = Country::default();
    for hour in 0..=240 {
        let row = input(1, i64::from(hour / 24) * 100_000);
        observe(&mut country, hour, &row);
    }
    assert_eq!(at(&country, 0), None); // Initial assignment's multiplier may be stale.
    assert_eq!(at(&country, 23), None);
    assert_eq!(at(&country, 24), Some(99));
    assert_eq!(at(&country, 239), Some(91));
    assert_eq!(at(&country, 240), Some(90));
    assert_eq!(country.history.slots[0].states.len(), 2);
    assert_eq!(country.samples[&0].len(), 24);
    assert_eq!(remaining(&input(1, 0), 0), None);
    assert_eq!(remaining(&input(1, 10_000_001), 100_000), Some(0));
}

#[test]
fn progress_compresses_separately_from_modifier_and_countdown_states() {
    let mut country = Country::default();
    for hour in 0..=48 {
        observe(
            &mut country,
            hour,
            &input(1, i64::from(hour / 24) * 100_000),
        );
    }
    let slot = &country.history.slots[0];
    assert_eq!(slot.progress.len(), 3);
    assert_eq!(slot.progress[0].first_hour, 0);
    assert_eq!(slot.progress[0].last_hour, 23);
    assert_eq!(slot.progress[1].progress_raw, "100000");
    assert_eq!(slot.progress[2].base_days_raw.as_deref(), Some("10000000"));
    assert_eq!(slot.states.len(), 2);
    observe(&mut country, 50, &input(1, 200_000));
    assert_eq!(
        country.history.slots[0]
            .progress
            .last()
            .unwrap()
            .entry_first_observed_hour,
        50
    );
    observe(&mut country, 51, &input(2, 0));
    assert_eq!(
        country.history.slots[0]
            .progress
            .last()
            .unwrap()
            .entry_first_observed_hour,
        51
    );
}

#[test]
fn base_duration_uses_recorded_cost_operands_and_rejects_invalid_costs() {
    let mut row = input(1, 0);
    row.cost_scale_raw = 150_000;
    assert_eq!(base_cost(&row), Some(15_000_000));
    row.cost_raw = -1;
    assert_eq!(base_cost(&row), None);
    row.cost_debug = 1;
    assert_eq!(base_cost(&row), Some(100_000));
    row.cost_debug = 0;
    row.cost_raw = i64::MAX;
    row.cost_scale_raw = i64::MAX;
    assert_eq!(base_cost(&row), None);
}

#[test]
fn transfers_complete_only_in_the_finishing_slot_and_preserve_technology_identity() {
    let mut country = Country::default();
    let first = input(1, 0);
    let idle = ResearchInputRow {
        index: 1,
        technology_present: 0,
        ..first.clone()
    };
    country.observe_inputs((7, 7), 0, &[&first, &idle], None, Some(&[]), None);
    let vacated = ResearchInputRow {
        technology_present: 0,
        ..first.clone()
    };
    let moved = ResearchInputRow {
        index: 1,
        progress_raw: 3_000_000,
        applied_days_raw: 3_000_000,
        ..first
    };
    country.observe_inputs((7, 7), 1, &[&vacated, &moved], None, Some(&[]), None);
    assert_eq!(
        country.history.slots[0].entries[0].outcome,
        ResearchOutcome::Switched
    );
    assert!(country.pending.is_empty());
    assert_eq!(country.history.slots[1].entries[0].technology_handle, 1);
    assert_eq!(country.history.slots[1].entries[0].continuity_start_hour, 0);
    // Completion may reset progress and advance the repeatable technology's
    // level. That zero must not become the previous level's final progress.
    let completed = ResearchInputRow {
        level: 1,
        classifier: 4,
        progress_raw: 0,
        ..moved.clone()
    };
    country.observe_inputs((7, 7), 2, &[&vacated, &completed], None, Some(&[]), None);
    assert_eq!(
        country.history.slots[1].entries[0].outcome,
        ResearchOutcome::Completed
    );
    assert_eq!(
        country.history.slots[0].entries[0].outcome,
        ResearchOutcome::Switched
    );
    assert_eq!(
        country.history.slots[1].progress.last().unwrap().last_hour,
        1
    );
    country.observe_inputs((7, 7), 4, &[&vacated, &moved], None, Some(&[]), None);
    assert_eq!(
        country.history.slots[1]
            .entries
            .last()
            .unwrap()
            .continuity_start_hour,
        4
    );
}

#[test]
fn resuming_in_the_same_slot_cannot_complete_the_cancelled_assignment() {
    let mut country = Country::default();
    observe(&mut country, 0, &input(1, 0));
    let inventory = [ResearchTechnologyRow {
        country: 11,
        technology_handle: 1,
        classifier: 5,
        ..Default::default()
    }];
    country.observe_inputs(
        (7, 7),
        1,
        &[&input(2, 0)],
        Some(&inventory),
        Some(&[]),
        None,
    );
    assert_eq!(country.pending.len(), 1);
    observe(&mut country, 2, &input(1, 100_000));
    let completed = ResearchInputRow {
        level: 1,
        classifier: 4,
        ..input(1, 0)
    };
    observe(&mut country, 3, &completed);
    let entries = &country.history.slots[0].entries;
    assert_eq!(entries[0].outcome, ResearchOutcome::Switched);
    assert_eq!(entries[2].outcome, ResearchOutcome::Completed);
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.outcome == ResearchOutcome::Completed)
            .count(),
        1
    );
}

#[test]
fn a_new_level_in_another_slot_does_not_hide_the_previous_level_completion() {
    let mut country = Country::default();
    let first = ResearchInputRow {
        level: 1,
        ..input(1, 0)
    };
    let idle = ResearchInputRow {
        index: 1,
        technology_present: 0,
        ..first.clone()
    };
    country.observe_inputs((7, 7), 0, &[&first, &idle], None, Some(&[]), None);
    let vacated = ResearchInputRow {
        technology_present: 0,
        ..first.clone()
    };
    let next = ResearchInputRow {
        index: 1,
        level: 2,
        ..first
    };
    country.observe_inputs((7, 7), 1, &[&vacated, &next], None, Some(&[]), None);
    assert_eq!(
        country.history.slots[0].entries[0].outcome,
        ResearchOutcome::Completed
    );
    assert_eq!(
        country.history.slots[1].entries[0].outcome,
        ResearchOutcome::Active
    );
    assert_eq!(country.history.slots[1].entries[0].level, 2);
}

#[test]
fn incremental_history_budget_matches_full_accounting_over_a_campaign() {
    let mut country = Country::default();
    for hour in 0..24 * 365 * 3 {
        if hour % 997 == 996 {
            continue;
        }
        let count = 2 + (hour / (24 * 365)) as u32;
        let rows: Vec<_> = (0..count)
            .map(|index| ResearchInputRow {
                index,
                banked_days_raw: i64::from(hour % 31) * 100_000,
                ..input(
                    1 + index + (hour / 720) as u32 * 4,
                    i64::from(hour % 720) * 10_000,
                )
            })
            .collect();
        country.observe_inputs(
            (7, 7),
            hour,
            &rows.iter().collect::<Vec<_>>(),
            None,
            Some(&[]),
            None,
        );
        if hour % 257 == 0 {
            assert_eq!(
                country.retained_bytes + 128,
                history_bytes(&country.history)
            );
        }
    }
    assert_eq!(
        country.retained_bytes + 128,
        history_bytes(&country.history)
    );
    let mut countries = BTreeMap::from([("ENG".into(), country)]);
    countries.get_mut("ENG").unwrap().retained_bytes = MAX_BYTES;
    assert!(check_budget(&countries).is_err());
}

#[test]
fn a_switch_gap_or_identity_change_cannot_lend_progress_to_an_estimate() {
    let mut country = Country::default();
    for hour in 0..=24 {
        observe(
            &mut country,
            hour,
            &input(1, i64::from(hour / 24) * 100_000),
        );
    }
    observe(&mut country, 25, &input(2, 0));
    assert_eq!(at(&country, 25), Some(100)); // Witnessed assignment uses starting multiplier.
    observe(&mut country, 27, &input(2, 0));
    assert_eq!(at(&country, 27), None);
    assert_eq!(
        country.history.slots[0].entries[1].outcome,
        ResearchOutcome::Unknown
    );
    country.observe_inputs((8, 7), 28, &[&input(2, 0)], None, Some(&[]), None);
    assert_eq!(at(&country, 28), None);
}

#[test]
fn attribution_preserves_ids_unknowns_and_independent_bonuses() {
    let mut country = Country::default();
    let mut row = input(1, 0);
    row.bonus_count = 2;
    let bonuses = [
        ResearchAppliedBonusRow {
            country: 11,
            index: 0,
            ordinal: 0,
            technology_handle: 1,
            id: -17,
            resolved: 1,
            name: b"radio_bonus".to_vec(),
            cost_reduction_raw: None,
            speed_bonus_raw: 50_000,
            ahead_reduction_raw: 200_000,
        },
        ResearchAppliedBonusRow {
            country: 11,
            index: 0,
            ordinal: 1,
            technology_handle: 1,
            id: 99,
            ..Default::default()
        },
    ];
    country.observe_inputs((7, 7), 0, &[&row], None, Some(&bonuses), None);
    let data = country.history.slots[0].states[0]
        .details
        .as_ref()
        .unwrap()
        .bonuses
        .as_ref()
        .unwrap();
    assert_eq!(data[0].token.as_deref(), Some("radio_bonus"));
    assert_eq!(data[0].speed_bonus_raw.as_deref(), Some("50000"));
    assert_eq!(data[1].id, 99);
    assert!(data[1].token.is_none());
    country.observe_inputs((7, 7), 1, &[&row], None, None, None);
    assert!(
        country.history.slots[0].states[1]
            .details
            .as_ref()
            .unwrap()
            .bonuses
            .is_none()
    );
    assert_eq!(country.history.coverage[0].last_hour, 1);
}

#[test]
fn delayed_inventory_completion_keeps_the_assignment_end_and_observation_separate() {
    let mut country = Country::default();
    observe(&mut country, 0, &input(1, 0));
    observe(&mut country, 1, &input(2, 0));
    let inventory = [ResearchTechnologyRow {
        country: 11,
        technology_handle: 1,
        level: 1,
        researched: 1,
        classifier: 4,
        ..Default::default()
    }];
    country.observe_inputs(
        (7, 7),
        2,
        &[&input(2, 0)],
        Some(&inventory),
        Some(&[]),
        None,
    );
    let first = &country.history.slots[0].entries[0];
    assert_eq!(first.outcome, ResearchOutcome::Completed);
    assert_eq!(first.ended_by_hour, Some(1));
    assert_eq!(first.outcome_observed_hour, Some(2));
}

fn research_replay(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

#[test]
fn hourly_research_is_independent_of_staggered_catalogue_and_never_falls_back_after_failure() {
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    fixture_research_inputs(&mut frame);
    for table in [
        TableId::ResearchSlots,
        TableId::ResearchTechnologies,
        TableId::TechnologyDefinitions,
        TableId::ResearchAppliedBonuses,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "independent table failure".into(),
        };
    }
    let histories =
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false)).unwrap();
    let slot = &histories["GER"].slots[0];
    assert_eq!(slot.entries[0].token, "tech_1");
    assert!(slot.states[0].details.as_ref().unwrap().bonuses.is_none());
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "hourly inputs failed".into(),
    };
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
}

fn research_frame(
    branch: u64,
    hour: i32,
    assignments: &[Option<u32>],
    completed: &[u32],
) -> DecodedFrame {
    use replay::rows::{ResearchSlotRow, ResearchTechnologyRow, TechnologyDefinitionRow};
    let mut frame = source(branch, hour, Some(1));
    *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "fixture hourly research".into(),
    };
    *frame.tables[TableId::ResearchSlots.index()] = TableView::Complete(TableRows::ResearchSlots(
        assignments
            .iter()
            .enumerate()
            .map(|(index, handle)| ResearchSlotRow {
                country: 11,
                index: index as u32,
                technology_handle: handle.unwrap_or(0),
                technology_present: u8::from(handle.is_some()),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(
            (1..=4)
                .map(|handle| {
                    let done = completed.contains(&handle);
                    ResearchTechnologyRow {
                        country: 11,
                        technology_handle: handle,
                        level: i32::from(done),
                        researched: u8::from(done),
                        classifier: if done {
                            engine::ctechnology::CLASSIFIER_COMPLETED
                        } else {
                            engine::ctechnology::CLASSIFIER_ACTIVE
                        },
                        ..Default::default()
                    }
                })
                .collect(),
        ));
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(
            (1..=4)
                .map(|technology_handle| TechnologyDefinitionRow {
                    technology_handle,
                    id: format!("tech_{technology_handle}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

#[test]
fn research_records_slot_growth_completions_switches_idle_and_unknown_initial_duration() {
    let replay = research_replay(vec![
        research_frame(1, 100, &[Some(1), None], &[]),
        research_frame(1, 101, &[Some(1), Some(2)], &[]),
        research_frame(1, 102, &[Some(3), Some(4), None], &[1]),
        research_frame(1, 103, &[None, Some(4), Some(2)], &[1]),
    ]);
    let histories = crate::research_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 103
        }]
    );
    assert_eq!(history.slots.len(), 3);
    let first = &history.slots[0].entries;
    assert_eq!(first[0].token, "tech_1");
    assert_eq!(first[0].started_after_hour, None);
    assert_eq!(first[0].outcome, crate::ResearchOutcome::Completed);
    assert_eq!(first[0].ended_by_hour, Some(102));
    assert_eq!(first[1].outcome, crate::ResearchOutcome::Switched);
    assert_eq!(first[1].started_after_hour, Some(101));
    assert_eq!(
        history.slots[1].entries[0].outcome,
        crate::ResearchOutcome::Switched
    );
    assert_eq!(
        history.slots[1].entries[1].outcome,
        crate::ResearchOutcome::Active
    );
    assert_eq!(history.slots[2].availability[0].first_hour, 102);
    assert_eq!(history.slots[2].entries[0].first_observed_hour, 103);
}

#[test]
fn research_slot_states_preserve_effects_idle_and_gaps_without_hourly_duplicates() {
    let mut frames = vec![
        research_frame(1, 100, &[Some(1)], &[]),
        research_frame(1, 101, &[Some(1)], &[]),
        research_frame(1, 102, &[Some(1)], &[]),
        research_frame(1, 103, &[None], &[]),
        research_frame(1, 105, &[Some(1)], &[]),
    ];
    for frame in &mut frames {
        let TableView::Complete(TableRows::ResearchSlots(slots)) =
            &mut *frame.tables[TableId::ResearchSlots.index()]
        else {
            panic!()
        };
        slots[0].research_speed_raw = if frame.hour == 102 { 200_000 } else { 150_000 };
        slots[0].applied_days_raw = 3_000_000;
        slots[0].banked_days_raw = i64::MAX;
        let TableView::Complete(TableRows::ResearchTechnologies(technologies)) =
            &mut *frame.tables[TableId::ResearchTechnologies.index()]
        else {
            panic!()
        };
        technologies[0].ahead_reduction_years_raw = 200_000;
        technologies[0].progress_raw = i64::from(frame.hour); // Progress changes do not duplicate stable effects.
    }
    let histories =
        crate::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let slot = &histories["GER"].slots[0];
    assert_eq!(slot.states.len(), 4);
    assert_eq!(
        (slot.states[0].first_hour, slot.states[0].last_hour),
        (100, 101)
    );
    assert_eq!(slot.states[0].entry_first_observed_hour, Some(100));
    assert_eq!(slot.states[0].speed_multiplier_raw, "150000");
    assert_eq!(slot.states[0].banked_days_raw, i64::MAX.to_string());
    assert_eq!(slot.states[0].applied_days_raw, "3000000");
    assert_eq!(
        slot.states[0].ahead_reduction_years_raw.as_deref(),
        Some("200000")
    );
    assert_eq!(slot.states[1].speed_multiplier_raw, "200000");
    assert_eq!(slot.states[2].token, None);
    assert_eq!(slot.states[2].entry_first_observed_hour, None);
    assert_eq!(slot.states[2].ahead_reduction_years_raw, None);
    assert_eq!(slot.states[3].entry_first_observed_hour, Some(105));
    assert_eq!(slot.availability.len(), 2);
}

/// One recorded hour of a sliced research recording: the rows read at
/// `read` (with that read's assignments) carried into hour `hour`.
fn sliced_research_frame(hour: i32, read: i32, assignments: &[Option<u32>]) -> DecodedFrame {
    let mut frame = research_frame(1, read, assignments, &[]);
    fixture_research_inputs(&mut frame);
    let mut carried = research_frame(1, hour, assignments, &[]);
    carried.tables[TableId::ResearchInputs.index()] =
        frame.tables[TableId::ResearchInputs.index()].clone();
    carried.tables[TableId::ResearchAppliedBonuses.index()] =
        frame.tables[TableId::ResearchAppliedBonuses.index()].clone();
    carried.tables[TableId::SliceReads.index()] = frame.tables[TableId::SliceReads.index()].clone();
    carried
}

#[test]
fn sliced_research_reads_a_day_apart_are_continuous_observations() {
    // Country 11 is read at 100, 124 and 148; its rows are carried between.
    // Tech 1 is replaced by tech 2 between the first and second reads.
    let mut frames = Vec::new();
    for hour in 100..=150 {
        let (read, assignment) = match hour {
            100..=123 => (100, Some(1)),
            124..=147 => (124, Some(2)),
            _ => (148, Some(2)),
        };
        frames.push(sliced_research_frame(hour, read, &[assignment]));
    }
    let histories =
        crate::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    // One continuous coverage span, from the first read to the last.
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 148
        }]
    );
    let entries = &history.slots[0].entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].token, "tech_1");
    assert_eq!(entries[0].outcome, crate::ResearchOutcome::Switched);
    assert_eq!(entries[0].ended_by_hour, Some(124));
    // The switch happened after the previous read, bounded to a day.
    assert_eq!(entries[1].token, "tech_2");
    assert_eq!(entries[1].first_observed_hour, 124);
    assert_eq!(entries[1].started_after_hour, Some(100));
    assert_eq!(entries[1].last_observed_hour, 148);
    assert_eq!(entries[1].outcome, crate::ResearchOutcome::Active);
}

#[test]
fn an_unobservable_hour_between_sliced_reads_breaks_their_continuity() {
    let mut frames = Vec::new();
    for hour in 100..=124 {
        let read = if hour < 124 { 100 } else { 124 };
        let mut frame = sliced_research_frame(hour, read, &[Some(1)]);
        if hour == 110 {
            *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
                reason: "synthetic failed slice".into(),
            };
        }
        frames.push(frame);
    }
    let histories =
        crate::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(history.coverage.len(), 2);
    let entries = &history.slots[0].entries;
    assert_eq!(entries[0].outcome, crate::ResearchOutcome::Unknown);
    assert_eq!(entries[1].started_after_hour, None);
}

#[test]
fn a_missed_sliced_read_of_an_owner_breaks_its_continuity() {
    // Country 11 is read at 100, its read at 124 did not happen (its rows are
    // carried with sample hour 100), and it is read again at 148.
    let mut frames = Vec::new();
    for hour in 100..=148 {
        let read = if hour < 148 { 100 } else { 148 };
        frames.push(sliced_research_frame(hour, read, &[Some(1)]));
    }
    let histories =
        crate::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(history.coverage.len(), 2);
    let entries = &history.slots[0].entries;
    assert_eq!(entries[0].outcome, crate::ResearchOutcome::Unknown);
    assert_eq!(entries[1].started_after_hour, None);
}

#[test]
fn an_unlinked_input_row_does_not_hide_linked_reads_in_its_frame() {
    // Country 11 is read at 100 and 124; the 124 frame also carries a row of
    // country 999, which has no CountryLinks entry.
    let mut frames = Vec::new();
    for hour in 100..=124 {
        let read = if hour < 124 { 100 } else { 124 };
        let mut frame = sliced_research_frame(hour, read, &[Some(1)]);
        if hour == 124 {
            let TableView::Complete(TableRows::ResearchInputs(rows)) =
                &mut *frame.tables[TableId::ResearchInputs.index()]
            else {
                panic!("fixture research inputs")
            };
            rows.push(replay::rows::ResearchInputRow {
                country: 999,
                ..Default::default()
            });
        }
        frames.push(frame);
    }
    let histories =
        crate::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 124
        }]
    );
    assert_eq!(history.slots[0].entries[0].last_observed_hour, 124);
}

#[test]
fn research_never_dates_through_gaps_or_country_reuse_and_cache_is_branch_scoped() {
    let mut missing = research_frame(1, 101, &[Some(1)], &[]);
    *missing.tables[TableId::ResearchSlots.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let mut reused = research_frame(1, 104, &[Some(2)], &[1]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[0].id = 11;
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(
                1,
                vec![
                    research_frame(1, 100, &[Some(1)], &[]),
                    missing,
                    research_frame(1, 102, &[Some(1)], &[]),
                    research_frame(1, 103, &[Some(2)], &[1]),
                    reused,
                ],
            ),
            records(2, vec![research_frame(2, 100, &[Some(4)], &[])]),
        ],
    )
    .unwrap();
    let mut cache = crate::ResearchCache::default();
    let cancel = AtomicBool::new(false);
    let history = cache.get(&replay, 1, "GER", &cancel).unwrap();
    assert_eq!(history.coverage.len(), 2);
    assert_eq!(history.coverage[0].last_hour, 100);
    let entries = &history.slots[0].entries;
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].outcome, crate::ResearchOutcome::Unknown);
    assert_eq!(entries[0].last_observed_hour, 100);
    assert_eq!(entries[1].started_after_hour, None);
    assert_eq!(entries[2].outcome, crate::ResearchOutcome::Unknown);
    assert_eq!(entries[3].started_after_hour, None);
    assert_eq!(
        cache.get(&replay, 2, "GER", &cancel).unwrap().slots[0].entries[0].token,
        "tech_4"
    );
    assert_eq!(cache.get(&replay, 1, "GER", &cancel).unwrap(), history);
    assert!(cache.get(&replay, 1, "FRA", &cancel).is_err());
    assert!(
        cache
            .get(&replay, 1, "GER", &AtomicBool::new(true))
            .is_err()
    );
}

#[test]
fn research_cache_keeps_country_requests_independent() {
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    fixture_research_inputs(&mut frame);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        let mut second = rows[0].clone();
        second.id = 9002;
        second.canonical_slot = 8;
        second.tag = b"ENG".to_vec();
        rows.push(second);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 9002,
            canonical_slot: 8,
        });
    }
    if let TableView::Complete(TableRows::ResearchInputs(rows)) =
        &mut *frame.tables[TableId::ResearchInputs.index()]
    {
        let mut second = rows[0].clone();
        second.country = 9002;
        second.technology_handle = 2;
        second.token = b"tech_2".to_vec();
        rows.push(second);
    }
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *frame.tables[TableId::SliceReads.index()]
    {
        rows.push(replay::rows::SliceReadRow {
            table: TableId::ResearchInputs.index() as u32,
            owner: 9002,
            sample_hour: 100,
        });
        rows.sort_by_key(|row| (row.table, row.owner));
    }
    let replay = research_replay(vec![frame]);
    let cancel = AtomicBool::new(false);
    let mut cache = crate::ResearchCache::default();
    let german = cache.get(&replay, 1, "GER", &cancel).unwrap();
    let british = cache.get(&replay, 1, "ENG", &cancel).unwrap();
    assert_eq!(german.slots[0].entries[0].token, "tech_1");
    assert_eq!(british.slots[0].entries[0].token, "tech_2");
    assert_eq!(cache.get(&replay, 1, "GER", &cancel).unwrap(), german);
    assert_eq!(cache.get(&replay, 1, "ENG", &cancel).unwrap(), british);
}

#[test]
fn research_missing_definitions_or_truncated_technologies_are_not_empty_research() {
    for table in [
        TableId::ResearchSlots,
        TableId::ResearchTechnologies,
        TableId::TechnologyDefinitions,
    ] {
        let mut frame = research_frame(1, 100, &[Some(1)], &[]);
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic unavailable".into(),
        };
        assert!(
            crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
                .unwrap()
                .is_empty()
        );
    }
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(Vec::new()));
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        links.retain(|link| link.id != 11);
    }
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::ResearchTechnologies.index()] = TableView::Truncated {
        rows: TableRows::ResearchTechnologies(Vec::new()),
        missing: 1,
    };
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn research_repeat_levels_and_removed_slots_preserve_distinct_assignments() {
    let mut first = research_frame(1, 100, &[Some(1), Some(2)], &[1]);
    if let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *first.tables[TableId::ResearchTechnologies.index()]
    {
        rows[0].classifier = engine::ctechnology::CLASSIFIER_ACTIVE;
    }
    let mut second = first.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *second.tables[TableId::ResearchTechnologies.index()]
    {
        rows[0].level = 2;
    }
    let mut third = second.clone();
    third.hour = 102;
    if let TableView::Complete(TableRows::ResearchSlots(rows)) =
        &mut *third.tables[TableId::ResearchSlots.index()]
    {
        rows.pop();
    }
    let histories = crate::research_branch(
        &research_replay(vec![first, second, third]),
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    let slots = &histories["GER"].slots;
    assert_eq!(slots[0].entries.len(), 2);
    assert_eq!(
        slots[0].entries[0].outcome,
        crate::ResearchOutcome::Completed
    );
    assert_eq!(slots[0].entries[0].level, 1);
    assert_eq!(slots[0].entries[1].level, 2);
    assert_eq!(slots[1].entries[0].outcome, crate::ResearchOutcome::Unknown);
    assert_eq!(slots[1].availability[0].last_hour, 101);
}
