use super::test_fixtures::{research_frame, sliced_research_frame};
use super::{Country, Inventory, MAX_BYTES, base_cost, check_budget, history_bytes, remaining};
use crate::ResearchOutcome;
use crate::test_fixtures::{CAMPAIGN, fixture_research_inputs, records};
use replay::container::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::ResearchAppliedBonusRow, rows::ResearchInputRow,
    rows::ResearchTechnologyRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn input(handle: u32, progress: i64) -> ResearchInputRow {
    ResearchInputRow {
        country: 11,
        index: 0,
        technology_handle: handle,
        database_index: replay_data::rows::Recorded::Value(handle as i32),
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
        Some(50)
    );
    observe(&mut country, 51, &input(2, 0));
    assert_eq!(
        country.history.slots[0]
            .progress
            .last()
            .unwrap()
            .entry_first_observed_hour,
        Some(51)
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
        database_index: 1,
        country: 11,
        technology_handle: 1,
        classifier: 5,
        ..Default::default()
    }];
    country.observe_inputs(
        (7, 7),
        1,
        &[&input(2, 0)],
        Some(&Inventory::Table(&inventory)),
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
    // Cross two slot-growth, assignment-switch and missing-hour boundaries.
    for hour in 0..2_100 {
        if hour % 997 == 996 {
            continue;
        }
        let count = 2 + (hour / 720) as u32;
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
    let mut scan = super::Scan {
        countries,
        ..Default::default()
    };
    scan.begin_hour(2_100);
    let saved = serde_json::to_vec(&scan).unwrap();
    let mut restored: super::Scan = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.bytes, 0, "allocation charges are process-local");
    restored.begin_hour(2_101);
    for country in restored.countries.values() {
        assert_eq!(
            country.retained_bytes + 128,
            history_bytes(&country.history)
        );
        assert_eq!(country.budget_bytes, super::country_bytes(country));
    }
    assert_eq!(
        restored.bytes,
        restored
            .countries
            .values()
            .map(super::country_bytes)
            .sum::<usize>()
    );
    countries = restored.countries;
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
        database_index: 1,
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
        Some(&Inventory::Table(&inventory)),
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
        super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false)).unwrap();
    let slot = &histories["GER"].slots[0];
    assert_eq!(slot.entries[0].token, "tech_1");
    assert!(slot.states[0].details.as_ref().unwrap().bonuses.is_none());
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "hourly inputs failed".into(),
    };
    assert!(
        super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn research_records_slot_growth_completions_switches_idle_and_unknown_initial_duration() {
    let replay = research_replay(vec![
        research_frame(1, 100, &[Some(1), None], &[]),
        research_frame(1, 101, &[Some(1), Some(2)], &[]),
        research_frame(1, 102, &[Some(3), Some(4), None], &[1]),
        research_frame(1, 103, &[None, Some(4), Some(2)], &[1]),
    ]);
    let histories = super::research_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
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
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
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
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    // One continuous coverage span, from the first read to the last recorded
    // hour, which still carries the read at 148.
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 150
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
fn carried_hours_hold_the_last_read_until_the_next_and_progress_keeps_its_read_hour() {
    // Read at 100 and 124 (tech 1 replaced by tech 2), carried to 130.
    let mut frames = Vec::new();
    for hour in 100..=130 {
        let (read, assignment) = if hour < 124 {
            (100, Some(1))
        } else {
            (124, Some(2))
        };
        frames.push(sliced_research_frame(hour, read, &[assignment]));
    }
    let histories =
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 130
        }]
    );
    let slot = &history.slots[0];
    assert_eq!(
        slot.availability,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 130
        }]
    );
    // Each hour has the state of the read it carries; none is left uncovered.
    let spans: Vec<_> = slot
        .states
        .iter()
        .map(|state| (state.first_hour, state.last_hour, state.token.as_deref()))
        .collect();
    assert_eq!(
        spans,
        [(100, 123, Some("tech_1")), (124, 130, Some("tech_2"))]
    );
    // Observation bounds and progress keep the read hours.
    assert_eq!(slot.entries[0].last_observed_hour, 100);
    assert_eq!(slot.entries[1].last_observed_hour, 124);
    assert!(
        slot.progress
            .iter()
            .all(|sample| sample.first_hour == sample.last_hour)
    );
}

#[test]
fn a_carried_read_ends_when_the_next_read_is_due() {
    // Read at 100 only; its rows are carried through 140.
    let frames = (100..=140)
        .map(|hour| sliced_research_frame(hour, 100, &[Some(1)]))
        .collect();
    let histories =
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    let due = 100 + replay_data::cadence::STAGGER_PERIOD;
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: due - 1
        }]
    );
    assert_eq!(history.slots[0].states.last().unwrap().last_hour, due - 1);
    assert_eq!(history.slots[0].entries[0].last_observed_hour, 100);
}

#[test]
fn historical_reads_are_carried_and_counted_through_the_branch() {
    // Only older recordings lack the occurrence, and writers emit today's format,
    // so the read and the carried hours are driven as the branch walk drives them.
    let mut country = Country::default();
    let mut row = input(1, 100_000);
    row.database_index = replay_data::rows::Recorded::NotRecorded;
    observe(&mut country, 100, &row);
    country.last_read = Some(100);
    for hour in 101..=110 {
        assert!(country.carries((7, 7), hour));
        country.carry((7, 7), hour);
    }
    let (history, unjoined) = (&country.history, country.unjoined_reads);
    assert_eq!(unjoined, 1);
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 100,
            last_hour: 110
        }]
    );
    let slot = &history.slots[0];
    assert!(slot.entries.is_empty());
    assert_eq!(slot.states.len(), 1);
    assert_eq!(slot.states[0].token.as_deref(), Some("tech_1"));
    assert_eq!(
        (slot.states[0].first_hour, slot.states[0].last_hour),
        (100, 110)
    );
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
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
    let history = &histories["GER"];
    assert_eq!(history.coverage.len(), 2);
    // The read at 100 is carried only up to the unobservable hour.
    assert_eq!(history.coverage[0].last_hour, 109);
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
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
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
            rows.push(replay_data::rows::ResearchInputRow {
                country: 999,
                ..Default::default()
            });
        }
        frames.push(frame);
    }
    let histories =
        super::research_branch(&research_replay(frames), 1, &AtomicBool::new(false)).unwrap();
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
    let mut cache = super::ResearchCache::default();
    let cancel = AtomicBool::new(false);
    let history = cache.get(&replay, 1, "GER", &cancel).unwrap().0;
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
        cache.get(&replay, 2, "GER", &cancel).unwrap().0.slots[0].entries[0].token,
        "tech_4"
    );
    assert_eq!(cache.get(&replay, 1, "GER", &cancel).unwrap().0, history);
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
        rows.push(replay_data::rows::SliceReadRow {
            table: TableId::ResearchInputs.index() as u32,
            owner: 9002,
            sample_hour: 100,
        });
        rows.sort_by_key(|row| (row.table, row.owner));
    }
    let replay = research_replay(vec![frame]);
    let cancel = AtomicBool::new(false);
    let mut cache = super::ResearchCache::default();
    let german = cache.get(&replay, 1, "GER", &cancel).unwrap().0;
    let british = cache.get(&replay, 1, "ENG", &cancel).unwrap().0;
    assert_eq!(german.slots[0].entries[0].token, "tech_1");
    assert_eq!(british.slots[0].entries[0].token, "tech_2");
    assert_eq!(cache.get(&replay, 1, "GER", &cancel).unwrap().0, german);
    assert_eq!(cache.get(&replay, 1, "ENG", &cancel).unwrap().0, british);
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
            super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
                .unwrap()
                .is_empty()
        );
    }
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(Vec::new()));
    assert!(
        super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
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
        super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    *frame.tables[TableId::ResearchTechnologies.index()] = TableView::Truncated {
        rows: TableRows::ResearchTechnologies(Vec::new()),
        missing: 1,
    };
    assert!(
        super::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
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
        rows[0].classifier = engine_research::ctechnology::CLASSIFIER_ACTIVE;
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
    let histories = super::research_branch(
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

#[test]
fn repeated_handle_assignments_do_not_complete_transfer_or_share_progress() {
    let mut country = Country::default();
    let first = input(7, 100_000);
    observe(&mut country, 0, &first);
    country.read_after = Some(0);
    observe(&mut country, 24, &input(7, 200_000));
    let mut later = input(7, 9_000_000);
    later.database_index = replay_data::rows::Recorded::Value(8);
    let former = ResearchTechnologyRow {
        country: 11,
        database_index: 7,
        technology_handle: 7,
        classifier: 0,
        progress_raw: 200_000,
        ..Default::default()
    };
    country.read_after = Some(24);
    country.observe_inputs(
        (7, 7),
        48,
        &[&later],
        Some(&Inventory::Table(&[former])),
        Some(&[]),
        None,
    );
    let slot = &country.history.slots[0];
    assert_eq!(slot.entries.len(), 2);
    assert_eq!(slot.entries[0].outcome, ResearchOutcome::Switched);
    assert_eq!(slot.entries[1].outcome, ResearchOutcome::Active);
    assert_eq!(country.samples[&0].len(), 1);
    assert_eq!(
        slot.progress.last().unwrap().entry_first_observed_hour,
        Some(48)
    );
    let unrelated = ResearchTechnologyRow {
        country: 11,
        database_index: 8,
        technology_handle: 7,
        level: 1,
        classifier: 4,
        ..Default::default()
    };
    let mut idle = later.clone();
    idle.technology_present = 0;
    country.read_after = Some(48);
    country.observe_inputs(
        (7, 7),
        72,
        &[&idle],
        Some(&Inventory::Table(&[unrelated])),
        Some(&[]),
        None,
    );
    assert_eq!(
        country.history.slots[0].entries[0].outcome,
        ResearchOutcome::Switched
    );
    assert_eq!(
        country.history.slots[0].entries[1].outcome,
        ResearchOutcome::Completed
    );
}

#[test]
fn historical_inputs_without_occurrence_keep_slot_state_but_join_no_assignment() {
    let mut country = Country::default();
    let mut row = input(7, 100_000);
    row.database_index = replay_data::rows::Recorded::NotRecorded;
    row.research_speed_raw = 150_000;
    observe(&mut country, 0, &row);
    // The next read of an equal handle and name is still no proof of one assignment.
    row.progress_raw = 200_000;
    observe(&mut country, 1, &row);
    let history = &country.history;
    assert_eq!(
        history.coverage,
        [crate::ResearchCoverage {
            first_hour: 0,
            last_hour: 1
        }]
    );
    let slot = &history.slots[0];
    assert_eq!(slot.availability.len(), 1);
    assert!(slot.entries.is_empty());
    // Each read's progress is exact, attributed to the slot's held technology.
    let progress: Vec<_> = slot
        .progress
        .iter()
        .map(|p| {
            (
                p.first_hour,
                p.last_hour,
                p.entry_first_observed_hour,
                p.base_days_raw.as_deref(),
                p.progress_raw.as_str(),
            )
        })
        .collect();
    assert_eq!(
        progress,
        [
            (0, 0, None, Some("10000000"), "100000"),
            (1, 1, None, Some("10000000"), "200000")
        ]
    );
    let state = slot.states.last().unwrap();
    assert_eq!(state.token.as_deref(), Some("tech_7"));
    assert_eq!(state.entry_first_observed_hour, None);
    assert_eq!(state.speed_multiplier_raw, "150000");
    assert_eq!(state.ahead_reduction_years_raw.as_deref(), Some("0"));
    // No witnessed assignment: neither the slot speed nor a progress delta dates the rest.
    assert_eq!(state.details.as_ref().unwrap().remaining_days, None);
    assert_eq!(country.unjoined_reads, 2);
    assert_eq!(
        super::unjoined_warning(2),
        "2 research slot reads name their technology without its recorded occurrence, so they are not joined into assignments: their completions and the time they took are unknown."
    );
    assert!(super::unjoined_warning(1).starts_with("1 research slot read names its technology"));
}

#[test]
fn unjoined_progress_merges_only_within_one_held_technology() {
    let mut country = Country::default();
    let held = |handle: u32, progress: i64| {
        let mut row = input(handle, progress);
        row.database_index = replay_data::rows::Recorded::NotRecorded;
        row
    };
    // The same technology read twice at equal progress is one sample.
    observe(&mut country, 0, &held(7, 300_000));
    observe(&mut country, 1, &held(7, 300_000));
    // Another technology with equal cost and progress starts its own sample.
    observe(&mut country, 2, &held(8, 300_000));
    let slot = &country.history.slots[0];
    let spans: Vec<_> = slot
        .progress
        .iter()
        .map(|p| (p.first_hour, p.last_hour, p.progress_raw.as_str()))
        .collect();
    assert_eq!(spans, [(0, 1, "300000"), (2, 2, "300000")]);
    assert!(
        slot.progress
            .iter()
            .all(|p| p.entry_first_observed_hour.is_none())
    );
    assert_eq!(slot.states.last().unwrap().token.as_deref(), Some("tech_8"));
}

#[test]
fn a_technology_is_shown_in_its_first_available_folder_and_unknowns_are_named() {
    // An undecidable folder placed before an available one may be the one
    // shown, so the technology's folder is unknown, not the later folder.
    let definitions = [
        ("armour_folder", Some(false)),
        ("nsb_armour_folder", Some(true)),
        ("mod_folder", None),
        ("infantry_folder", Some(true)),
    ];
    let placed: BTreeMap<String, Vec<String>> = [
        ("both_armour", vec!["armour_folder", "nsb_armour_folder"]),
        ("old_armour", vec!["armour_folder"]),
        ("modded", vec!["mod_folder"]),
        ("modded_or_infantry", vec!["mod_folder", "infantry_folder"]),
        ("infantry_or_modded", vec!["infantry_folder", "mod_folder"]),
        ("stray", vec!["missing_folder"]),
        ("hidden", vec![]),
    ]
    .into_iter()
    .map(|(token, folders)| {
        (
            token.to_owned(),
            folders.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect();
    let (shown, notes) = super::shown_folders(
        &definitions,
        &placed,
        [
            "both_armour",
            "old_armour",
            "modded",
            "modded_or_infantry",
            "infantry_or_modded",
            "stray",
            "hidden",
            "unknown_token",
        ],
    );
    let folders: Vec<_> = shown
        .iter()
        .map(|s| (s.token.as_str(), s.folder.as_deref()))
        .collect();
    assert_eq!(
        folders,
        [
            ("both_armour", Some("nsb_armour_folder")),
            ("old_armour", None),
            ("modded", None),
            ("modded_or_infantry", None),
            ("infantry_or_modded", Some("infantry_folder")),
            ("stray", None),
            ("hidden", None),
            ("unknown_token", None),
        ]
    );
    assert_eq!(
        notes,
        [
            "2 technologies are placed in a research folder whose availability cannot be decided from content before any available folder, so their folder is unknown: modded, modded_or_infantry.",
            "Research folder missing_folder is not defined in common/technology_tags, so 1 technology placed in it cannot be shown there: stray.",
        ]
    );
}

#[test]
fn a_finished_technology_left_in_a_slot_without_an_assignment_is_not_research() {
    for database_index in [
        replay_data::rows::Recorded::NotRecorded,
        replay_data::rows::Recorded::Value(7),
    ] {
        let mut country = Country::default();
        let mut row = input(7, 10_000_000);
        row.database_index = database_index;
        row.classifier = engine_research::ctechnology::CLASSIFIER_COMPLETED;
        observe(&mut country, 0, &row);
        let slot = &country.history.slots[0];
        assert!(slot.entries.is_empty());
        assert!(slot.progress.is_empty());
        assert_eq!(slot.states[0].token, None);
        assert_eq!(slot.states[0].ahead_reduction_years_raw, None);
    }
}

#[test]
fn completed_same_name_occurrence_cannot_supply_former_assignment_progress() {
    let mut country = Country::default();
    observe(&mut country, 0, &input(7, 100_000));
    let mut completed = input(7, 9_000_000);
    completed.database_index = replay_data::rows::Recorded::Value(8);
    completed.classifier = engine_research::ctechnology::CLASSIFIER_COMPLETED;
    let former = ResearchTechnologyRow {
        country: 11,
        database_index: 7,
        technology_handle: 7,
        progress_raw: 100_000,
        ..Default::default()
    };
    country.observe_inputs(
        (7, 7),
        1,
        &[&completed],
        Some(&Inventory::Table(&[former])),
        Some(&[]),
        None,
    );
    let slot = &country.history.slots[0];
    assert_eq!(slot.entries.len(), 1);
    assert_eq!(slot.entries[0].outcome, ResearchOutcome::Switched);
    assert_eq!(slot.progress.len(), 1);
    assert_eq!(slot.progress[0].progress_raw, "100000");
    assert_eq!(slot.states.last().unwrap().entry_first_observed_hour, None);

    let mut country = Country::default();
    observe(&mut country, 0, &input(7, 100_000));
    completed.database_index = replay_data::rows::Recorded::Value(7);
    completed.level = 1;
    observe(&mut country, 1, &completed);
    observe(&mut country, 2, &completed);
    assert_eq!(
        country.history.slots[0]
            .states
            .last()
            .unwrap()
            .entry_first_observed_hour,
        Some(0)
    );
}
