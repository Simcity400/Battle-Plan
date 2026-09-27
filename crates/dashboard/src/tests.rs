use std::{
    io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryRow, TableId, TableRows},
    spool::Record,
};

const CAMPAIGN: &[u8] = b"analytics-test";

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

fn focus_countries_frame(
    branch: u64,
    hour: i32,
    germany: (u32, u32, &[&str]),
    france: &[&str],
) -> DecodedFrame {
    use replay::rows::{CompletedFocusRow, FocusRow};
    let mut frame = source(branch, hour, Some(1));
    let (german_id, german_slot, german_tokens) = germany;
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: german_id,
            canonical_slot: german_slot,
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        },
        CountryRow {
            id: 10001,
            canonical_slot: 9,
            tag: b"FRA".to_vec(),
            ..CountryRow::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [
            (11, german_slot),
            (44, 9),
            (german_id, german_slot),
            (10001, 9),
        ]
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .to_vec(),
    ));
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        [(11, german_tokens), (44, france)]
            .map(|(country, tokens)| FocusRow {
                country,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            [(11, german_tokens), (44, france)]
                .into_iter()
                .flat_map(|(country, tokens)| {
                    tokens
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, token)| CompletedFocusRow {
                            country,
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                        })
                })
                .collect(),
        ));
    frame
}

#[test]
fn focus_branch_cache_keeps_countries_branches_and_gaps_independent() {
    let first = focus_countries_frame(1, 100, (9001, 7, &["a"]), &["f"]);
    let next = focus_countries_frame(1, 101, (9001, 7, &["a", "b"]), &["f"]);
    let mut unavailable = focus_countries_frame(1, 103, (9001, 7, &["a", "b"]), &["f"]);
    *unavailable.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "synthetic missing focus sample".into(),
    };
    let last = focus_countries_frame(1, 104, (9001, 7, &["a", "b"]), &["f", "g"]);
    let alternate = focus_countries_frame(2, 100, (9001, 7, &["other_branch"]), &[]);
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, vec![first, next, unavailable, last]),
            records(2, vec![alternate]),
        ],
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let projection = crate::focus_branch(&replay, 1, None, &cancel).unwrap();
    assert_eq!(projection.countries.len(), 2);
    let mut cache = crate::FocusCache::default();
    for tag in ["GER", "FRA", "GER", "FRA"] {
        let (history, progress_stopped) = cache.get(&replay, 1, tag, &cancel).unwrap();
        assert_eq!(progress_stopped, None);
        assert_eq!(history, projection.countries[tag]);
        assert!(history.at(102).is_err());
        assert!(history.at(103).is_err());
        assert_eq!(history.at(100).unwrap().completed.len(), 1);
        assert_eq!(history.at(104).unwrap().completed.len(), 2);
    }
    assert_eq!(
        cache
            .get(&replay, 1, "FRA", &cancel)
            .unwrap()
            .0
            .at(104)
            .unwrap()
            .completed[1]
            .completed_by_hour,
        Some(104)
    );
    assert_eq!(
        cache
            .get(&replay, 2, "GER", &cancel)
            .unwrap()
            .0
            .at(100)
            .unwrap()
            .completed[0]
            .token,
        "other_branch"
    );
    assert!(
        cache
            .get(&replay, 1, "GER", &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(
        cache.get(&replay, 1, "GER", &cancel).unwrap().0,
        projection.countries["GER"]
    );
}

#[test]
fn focus_dates_reset_when_list_shrinks_or_country_identity_changes() {
    let snapshots = [
        (100, 9001, 7, &[][..]),
        (101, 9001, 7, &["a"][..]),
        (102, 9001, 7, &["a", "b"][..]),
        (103, 9001, 7, &["a"][..]),
        (104, 9001, 7, &["a", "b"][..]),
        (105, 9002, 7, &["a", "b"][..]),
        (106, 9002, 8, &["a", "b", "c"][..]),
        (107, 9002, 8, &["a", "b", "c", "d"][..]),
    ];
    let frames = snapshots
        .into_iter()
        .map(|(hour, id, slot, tokens)| focus_countries_frame(1, hour, (id, slot, tokens), &[]))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let dates = |hour| {
        history
            .at(hour)
            .unwrap()
            .completed
            .into_iter()
            .map(|entry| entry.completed_by_hour)
            .collect::<Vec<_>>()
    };
    assert_eq!(dates(102), [Some(101), Some(102)]);
    assert_eq!(dates(103), [None]);
    assert_eq!(dates(104), [None, Some(104)]);
    assert_eq!(dates(105), [None, None]);
    assert_eq!(dates(106), [None, None, None]);
    assert_eq!(dates(107), [None, None, None, Some(107)]);
}

#[test]
fn focus_dates_preserve_unknown_start_repeats_gaps_and_as_of_history() {
    use replay::rows::{CompletedFocusRow, FocusRow};
    let snapshot = |hour, tokens: &[&str]| {
        let mut frame = source(1, hour, Some(1));
        *frame.tables[TableId::Focus.index()] =
            TableView::Complete(TableRows::Focus(vec![FocusRow {
                country: 11,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            }]));
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(ordinal, token)| CompletedFocusRow {
                        country: 11,
                        ordinal: ordinal as u32,
                        token: token.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                snapshot(100, &["old"]),
                snapshot(101, &["old", "new"]),
                snapshot(104, &["old", "new", "new"]),
                snapshot(105, &["reset"]),
            ],
        )],
    )
    .unwrap();
    let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(history.at(100).unwrap().completed.len(), 1);
    assert_eq!(
        history.at(100).unwrap().completed[0].completed_by_hour,
        None
    );
    assert_eq!(
        history.at(101).unwrap().completed[1].completed_by_hour,
        Some(101)
    );
    assert!(history.at(102).is_err());
    let later = history.at(104).unwrap();
    assert_eq!(
        later
            .completed
            .iter()
            .map(|r| r.completed_by_hour)
            .collect::<Vec<_>>(),
        [None, Some(101), Some(104)]
    );
    assert_eq!(
        history.at(105).unwrap().completed[0].completed_by_hour,
        None
    );
    assert!(history.at(106).is_err());
}

#[test]
fn focus_order_uses_exact_branch_country_links_and_complete_tables() {
    use replay::rows::{CompletedFocusRow, FocusRow};
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![FocusRow {
        country: 11,
        current: Some(b"next".to_vec()),
        completed_count: 1,
        ..FocusRow::default()
    }]));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 11,
            ordinal: 0,
            token: b"first".to_vec(),
        }]));
    let read = |frames| Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let replay = read(vec![frame.clone()]);
    let cancel = AtomicBool::new(false);
    let result = crate::focus_order(&replay, 1, 100, "GER", &cancel).unwrap();
    assert_eq!(result.current.as_deref(), Some("next"));
    assert_eq!(result.completed[0].token, "first");
    assert!(crate::focus_order(&replay, 2, 100, "GER", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 101, "GER", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 100, "ABSENT", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 100, "GER", &AtomicBool::new(true)).is_err());
    *frame.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    assert!(crate::focus_order(&read(vec![frame]), 1, 100, "GER", &cancel).is_err());
}

fn ownership_frame(branch: u64, hour: i32, owner: u32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for (id, slot, tag) in [(44, 8, b"FRA"), (55, 9, b"CTL"), (66, 10, b"MOD")] {
            rows.push(CountryRow {
                id,
                canonical_slot: slot,
                tag: tag.to_vec(),
                ..CountryRow::default()
            });
        }
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend(
            [(44, 8), (55, 9), (66, 10)]
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }),
        );
        rows.sort_by_key(|row| row.id);
    }
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![replay::rows::StateRow {
            id: 1,
            owner,
            controller: 55,
            movement_infrastructure: 0,
        }]));
    frame
}

#[test]
fn land_owners_include_transient_and_other_branch_owners_but_not_controllers() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(
                1,
                vec![
                    ownership_frame(1, 100, 11),
                    ownership_frame(1, 101, 44),
                    ownership_frame(1, 1000, 11),
                ],
            ),
            records(2, vec![ownership_frame(2, 90, 66)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["FRA", "GER", "MOD"]
    );
    let france = result
        .countries
        .iter()
        .find(|country| country.tag == "FRA")
        .unwrap();
    assert_eq!((france.id.as_str(), france.observed_hour), ("8", 101));
    assert_eq!(
        result
            .countries
            .iter()
            .find(|country| country.tag == "MOD")
            .unwrap()
            .observed_branch_id,
        "2"
    );
}

#[test]
fn land_owners_name_countries_in_the_given_language_and_fall_back_to_tags() {
    let mut frame = ownership_frame(1, 100, 44);
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 44,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 44,
            ..Default::default()
        }]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let localisation: installed_content::localisation::Localisation =
        [("FRA".to_owned(), "France".to_owned())]
            .into_iter()
            .collect();
    let result = crate::land_owners(&replay, Some(&localisation), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "France");
    let untranslated = installed_content::localisation::Localisation::default();
    let result = crate::land_owners(&replay, Some(&untranslated), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "FRA");
    assert!(result.warnings.iter().any(|w| w.contains("show their tag")));
}

#[test]
fn land_owners_report_dangling_owners_and_missing_country_identity() {
    for missing in ["link", "row", "tag"] {
        let mut frame = ownership_frame(1, 101, 44);
        match missing {
            "link" => {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            "row" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            _ => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.iter_mut()
                        .find(|row| row.id == 44)
                        .unwrap()
                        .tag
                        .clear();
                }
            }
        }
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![ownership_frame(1, 100, 11), frame])],
        )
        .unwrap();
        let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            result
                .countries
                .iter()
                .map(|country| country.tag.as_str())
                .collect::<Vec<_>>(),
            ["GER"],
            "{missing}"
        );
        assert!(!result.warnings.is_empty(), "{missing}");
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 0)])],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(result.countries.is_empty());
    assert!(result.warnings.is_empty());
}

#[test]
fn land_owners_keep_partial_positive_evidence_and_do_not_reuse_lost_baselines() {
    let mut partial = ownership_frame(1, 102, 66);
    *partial.tables[TableId::States.index()] = TableView::Truncated {
        rows: TableRows::States(vec![replay::rows::StateRow {
            id: 1,
            owner: 66,
            controller: 55,
            movement_infrastructure: 0,
        }]),
        missing: 1,
    };
    let mut input = records(
        1,
        vec![
            ownership_frame(1, 100, 11),
            ownership_frame(1, 101, 44),
            partial,
        ],
    );
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::States.index()] = TableUpdate::Unchanged { baseline: 99 };
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "MOD"]
    );
    assert!(!result.warnings.is_empty());
}

#[test]
fn land_owner_scan_honors_cancellation_and_block_budget() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 11)])],
    )
    .unwrap();
    assert_eq!(
        crate::land_owners(&replay, None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    // The container refuses an oversized block before decompressing it; the
    // product reports the gap instead of trusting a partial catalogue.
    replay.index[0].raw_length = replay::container::MAX_BLOCK_RAW_BYTES as u64 + 1;
    let owners = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(owners.countries.is_empty());
    assert!(!owners.warnings.is_empty());
}

fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn source(branch: u64, hour: i32, power: Option<i64>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside country fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 9001,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: power,
            command_power_raw: i64::MAX,
            stability_raw: i64::MIN,
            war_support_raw: 70_001,
            convoys_raw: Some(99_001),
            peace_conference_participant: 0,
        }]));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ]));
    DecodedFrame {
        branch,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn records(branch: u64, frames: Vec<DecodedFrame>) -> Vec<Record> {
    let mut out = vec![
        Record::SessionStart(session(branch)),
        Record::BranchStart(BranchStart {
            id: branch,
            session: branch,
            loaded_hour: None,
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    out.extend(
        frames
            .into_iter()
            .map(|mut frame| {
                // Existing research scenarios describe a logical observation.
                // Populate the hourly companion from those same fixture values.
                if matches!(&*frame.tables[TableId::ResearchInputs.index()], TableView::Unavailable { reason } if reason == "fixture hourly research") {
                    fixture_research_inputs(&mut frame);
                }
                Record::Frame(encoder.encode(frame, false).unwrap())
            }),
    );
    out
}

fn fixture_research_inputs(frame: &mut DecodedFrame) {
    use replay::rows::{ResearchInputRow, tables};
    let input = (|| {
        let slots = frame.complete::<tables::ResearchSlots>().ok()?;
        let techs = frame.complete::<tables::ResearchTechnologies>().ok()?;
        let definitions = frame.complete::<tables::TechnologyDefinitions>().ok()?;
        slots
            .iter()
            .map(|s| {
                let mut row = ResearchInputRow {
                    country: s.country,
                    index: s.index,
                    technology_handle: s.technology_handle,
                    technology_present: s.technology_present,
                    research_speed_raw: s.research_speed_raw,
                    banked_days_raw: s.banked_days_raw,
                    applied_days_raw: s.applied_days_raw,
                    ..Default::default()
                };
                if s.technology_present != 0 {
                    let t = techs.iter().find(|t| {
                        t.country == s.country && t.technology_handle == s.technology_handle
                    })?;
                    row.token = definitions
                        .iter()
                        .find(|d| d.technology_handle == s.technology_handle)?
                        .id
                        .clone();
                    row.level = t.level;
                    row.classifier = t.classifier;
                    row.progress_raw = t.progress_raw;
                    row.ahead_reduction_raw = t.ahead_reduction_years_raw;
                }
                Some(row)
            })
            .collect::<Option<Vec<_>>>()
    })();
    *frame.tables[TableId::ResearchInputs.index()] = input.map_or_else(
        || TableView::Unavailable {
            reason: "fixture research unavailable".into(),
        },
        |rows| TableView::Complete(TableRows::ResearchInputs(rows)),
    );
    *frame.tables[TableId::ResearchAppliedBonuses.index()] =
        TableView::Complete(TableRows::ResearchAppliedBonuses(vec![]));
    // Every fixture hour reads every country.
    let mut reads: Vec<_> = frame
        .complete::<tables::ResearchSlots>()
        .map(|slots| slots.iter().map(|slot| slot.country).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .flat_map(|country| {
            [
                TableId::ResearchInputs,
                TableId::ResearchAppliedBonuses,
                TableId::ResearchModifiers,
                TableId::ResearchTechnologies,
            ]
            .map(|table| replay::rows::SliceReadRow {
                table: table.index() as u32,
                owner: country,
                sample_hour: frame.hour,
            })
        })
        .collect();
    reads.sort_by_key(|row| (row.table, row.owner));
    reads.dedup();
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
}

#[test]
fn the_catalogue_names_countries_when_content_is_known_and_shows_tags_otherwise() {
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 9001,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            ..Default::default()
        }]));
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "source-1".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let localisation: installed_content::localisation::Localisation =
        [("GER".to_owned(), "German Reich".to_owned())]
            .into_iter()
            .collect();
    let named = source.named_metadata(Ok(Arc::new(localisation)));
    assert_eq!(named.countries[0].name, "German Reich");
    source.name_countries(None);
    let tagged = source.metadata();
    assert_eq!(tagged.countries[0].name, "GER");
    assert!(
        tagged
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
}

#[test]
fn metadata_skips_an_unavailable_first_census_warns_on_bad_links_and_stops_when_cancelled() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first, source(1, 101, Some(1))])],
        )
        .unwrap(),
    );
    let path = Path::new("memory.battleplan");
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert_eq!(metadata.branches[0].first_hour, 100);
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].observed_hour, 101);
    assert_eq!(metadata.countries[0].tag, "GER");
    assert_eq!(metadata.player_country_id, None);
    assert_eq!(
        crate::metadata::read(&replay, path, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );

    let mut bad = source(1, 100, Some(1));
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *bad.tables[TableId::CountryLinks.index()]
    {
        links[1].canonical_slot = 8;
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![bad])]).unwrap());
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|warning| warning.contains("CountryLinks"))
    );
}

#[test]
fn focus_progress_runs_compress_equal_hours_and_never_bridge_gaps() {
    let with_progress = |hour, progress| {
        let mut frame = focus_countries_frame(1, hour, (9001, 7, &["a"]), &["f"]);
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *frame.tables[TableId::Focus.index()]
        {
            for row in rows.iter_mut() {
                row.progress = progress;
            }
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                with_progress(100, 0),
                with_progress(101, 0),
                with_progress(102, 100_000),
                with_progress(104, 100_000),
            ],
        )],
    )
    .unwrap();
    let history = crate::focus_branch(&replay, 1, None, &AtomicBool::new(false))
        .unwrap()
        .countries["GER"]
        .clone();
    let runs: Vec<_> = history
        .progress
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.progress_raw.as_str()))
        .collect();
    // Hour 103 is missing, so the equal value at 104 starts a new run.
    assert_eq!(
        runs,
        [(100, 101, "0"), (102, 102, "100000"), (104, 104, "100000")]
    );
    // Two countries, so the budget holds two hours of runs. Past it, recording
    // stops rather than failing the product: the order history is complete and
    // the last kept run is never extended.
    let run = std::mem::size_of::<crate::FocusProgress>() + 20;
    let limited = crate::products::national_focus::focus_branch_within(
        &replay,
        1,
        None,
        &AtomicBool::new(false),
        4 * run,
    )
    .unwrap();
    assert_eq!(limited.progress_stopped, Some(104));
    let country = &limited.countries["GER"];
    assert_eq!(country.segments, history.segments);
    assert_eq!(
        country
            .progress
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (102, 102)]
    );
}

/// A unique three-byte tag per country, as the engine's tag loader allows.
fn scale_tag(index: u32) -> Vec<u8> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let digit = |place: u32| DIGITS[(index / place % 62) as usize];
    vec![digit(62 * 62), digit(62), digit(1)]
}

/// Every province its own country: `count` countries,
/// each owning one state.
fn scale_frame(branch: u64, hour: i32, count: u32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..count)
            .map(|index| CountryRow {
                id: 100_000 + index,
                canonical_slot: index,
                tag: scale_tag(index),
                ..CountryRow::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..count)
            .map(|index| CountryLinkRow {
                id: 100_000 + index,
                canonical_slot: index,
            })
            .collect(),
    ));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (0..count)
            .map(|index| replay::rows::StateRow {
                id: index + 1,
                owner: 100_000 + index,
                controller: 100_000 + index,
                movement_infrastructure: 0,
            })
            .collect(),
    ));
    frame
}

#[test]
fn metadata_catalogue_lists_every_country_once_across_branches_within_the_reply_budget() {
    const COUNTRIES: u32 = 65_535;
    let branches: Vec<_> = (1..=3)
        .map(|branch| records(branch, vec![scale_frame(branch, 100, COUNTRIES)]))
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("scale.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    // Not countries x branches: each country once, at its first branch.
    assert_eq!(metadata.countries.len(), COUNTRIES as usize);
    assert!(
        metadata
            .countries
            .iter()
            .all(|country| country.observed_branch_id == "1")
    );
    let reply = crate::transport::SourceReply {
        id: "lease".into(),
        path: metadata.path.clone(),
        revision: "revision".into(),
        metadata,
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 2,
        "{bytes} bytes for {COUNTRIES} countries"
    );
}

#[test]
fn land_owners_list_every_country_once_across_branches() {
    const COUNTRIES: u32 = 5_000;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(2, vec![scale_frame(2, 90, COUNTRIES)]),
            records(
                1,
                vec![
                    scale_frame(1, 100, COUNTRIES),
                    scale_frame(1, 101, COUNTRIES),
                ],
            ),
            records(3, vec![scale_frame(3, 80, COUNTRIES)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries.len(), COUNTRIES as usize);
    assert!(result.warnings.is_empty());
    // Sorted by tag, each witnessed at its first hour on the lowest branch.
    assert!(
        result
            .countries
            .windows(2)
            .all(|pair| pair[0].tag < pair[1].tag)
    );
    assert!(
        result.countries.iter().all(|country| (
            country.observed_branch_id.as_str(),
            country.observed_hour
        ) == ("1", 100))
    );
    let bytes = serde_json::to_vec(&result).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 4,
        "{bytes} bytes"
    );
}

#[test]
fn focus_budget_is_per_requested_country_not_per_recording() {
    use replay::rows::{CompletedFocusRow, FocusRow};
    const COUNTRIES: u32 = 5_000;
    // An hourly change for every country: 105,000 segments across the
    // recording, more than one scan may hold, but 21 for any one country.
    let frames = (0..21)
        .map(|step| {
            let mut frame = scale_frame(1, 100 + step, COUNTRIES);
            *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
                (0..COUNTRIES)
                    .map(|index| FocusRow {
                        country: 100_000 + index,
                        completed_count: 1,
                        ..FocusRow::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(
                    (0..COUNTRIES)
                        .map(|index| CompletedFocusRow {
                            country: 100_000 + index,
                            ordinal: 0,
                            token: format!("focus_{step}").into_bytes(),
                        })
                        .collect(),
                ));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let cancel = AtomicBool::new(false);
    let mut cache = crate::FocusCache::default();
    for index in [0, 2_777, COUNTRIES - 1] {
        let tag = String::from_utf8(scale_tag(index)).unwrap();
        let (history, progress_stopped) = cache.get(&replay, 1, &tag, &cancel).unwrap();
        assert_eq!(progress_stopped, None);
        assert_eq!(history.segments.len(), 21);
        assert_eq!(history.at(120).unwrap().completed[0].token, "focus_20");
    }
    // The same recording scanned for every country at once exceeds the scan
    // budget, which is why a request never scans other countries.
    assert!(crate::focus_branch(&replay, 1, None, &cancel).is_err());
}

fn styled_frame(hour: i32, red: f32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 9001,
                red: red.to_bits(),
                green: 0.5_f32.to_bits(),
                blue: 1.5_f32.to_bits(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            ..Default::default()
        }]));
    frame
}

#[test]
fn country_styles_take_the_engine_name_and_the_recorded_map_colour() {
    // Colour codes in the game's text are removed from chart names.
    let localisation: installed_content::localisation::Localisation =
        [("GER".to_owned(), "§YGerman§! Reich".to_owned())]
            .into_iter()
            .collect();
    let styles = crate::products::country_styles::HourStyles::read(
        &styled_frame(100, 0.2),
        Some(&localisation),
    )
    .unwrap();
    let style = styles.get("GER").unwrap();
    assert_eq!(style.name.as_deref(), Some("German Reich"));
    // The engine truncates f * 255, and out-of-range floats saturate.
    assert_eq!(style.color, Some([51, 127, 255]));
    let untranslated =
        crate::products::country_styles::HourStyles::read(&styled_frame(100, 0.2), None).unwrap();
    assert_eq!(untranslated.get("GER").unwrap().name, None);
    assert!(untranslated.get("FRA").is_none());
}

#[test]
fn country_styles_answer_per_hour_and_keep_missing_values_missing() {
    use crate::products::{
        LoadContext, Product,
        country_styles::{CountryStyleParams, CountryStyles},
    };
    let mut no_colour = source(1, 101, Some(1));
    *no_colour.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![styled_frame(100, 0.2), no_colour])],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "styles".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let request = |hour: i32| CountryStyleParams {
        branch_id: "1".into(),
        hour,
        tags: vec!["GER".into(), "XXX".into()],
    };
    let context = LoadContext { content: None };
    let first =
        CountryStyles::load(&source, &request(100), &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(first.styles[0].color, Some([51, 127, 255]));
    assert_eq!(first.styles[0].name, None);
    assert_eq!(
        first.styles[1],
        crate::products::country_styles::CountryStyle {
            tag: "XXX".into(),
            name: None,
            color: None
        }
    );
    assert!(
        first
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
    assert!(first.warnings.iter().any(|w| w.starts_with("XXX:")));
    let second =
        CountryStyles::load(&source, &request(101), &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(second.styles[0].color, None);
    assert!(
        second
            .warnings
            .iter()
            .any(|w| w == "GER: no recorded map colour")
    );
    assert!(
        CountryStyles::validate(&CountryStyleParams {
            branch_id: "1".into(),
            hour: 1,
            tags: vec![]
        })
        .is_err()
    );
    assert!(
        CountryStyles::validate(&CountryStyleParams {
            branch_id: "x".into(),
            hour: 1,
            tags: vec!["GER".into()]
        })
        .is_err()
    );
}

#[test]
fn country_styles_report_an_incomplete_country_table_as_incomplete() {
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let styles = crate::products::country_styles::HourStyles::read(&frame, None).unwrap();
    assert!(styles.get("GER").is_none());
    assert!(styles.missing_reason("GER").contains("incomplete"));
    let complete =
        crate::products::country_styles::HourStyles::read(&source(1, 100, Some(1)), None).unwrap();
    assert_eq!(complete.missing_reason("XXX"), "not a country at this hour");
}

/// Countries GER (link 9001), FRA (44), CTL (55) and MOD (66), with the given
/// human countries, factions (id, name, members) and wars at this hour.
fn group_frame(
    hour: i32,
    humans: &[u32],
    factions: &[(u64, &str, &[u32])],
    wars: &[(u32, u32)],
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let mut humans: Vec<_> = humans
        .iter()
        .map(|&country| replay::rows::HumanControllerRow { country, count: 1 })
        .collect();
    humans.sort_by_key(|row| row.country);
    *frame.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(humans));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        factions
            .iter()
            .map(|&(id, name, _)| replay::rows::FactionRow {
                id,
                name: name.as_bytes().to_vec(),
            })
            .collect(),
    ));
    let mut members: Vec<_> = factions
        .iter()
        .flat_map(|&(faction, _, members)| {
            members.iter().enumerate().map(move |(ordinal, &country)| {
                replay::rows::FactionMemberRow {
                    country,
                    faction,
                    ordinal: ordinal as u32,
                }
            })
        })
        .collect();
    members.sort_by_key(|member| member.country);
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    let mut relations: Vec<_> = wars
        .iter()
        .map(
            |&(first_country, second_country)| replay::rows::WarRelationRow {
                first_country,
                second_country,
                ..Default::default()
            },
        )
        .collect();
    relations.sort_by_key(|row| (row.first_country, row.second_country));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(relations));
    frame
}

#[test]
fn country_groups_follow_joins_leaves_new_factions_and_break_at_gaps() {
    use crate::products::country_groups::{
        HourSpan, MembershipRun, UnobservedHours, WarPairRun, scan_branch,
    };
    let run = |tag: &str, first_hour: i32, last_hour: i32| MembershipRun {
        tag: tag.into(),
        first_hour,
        last_hour,
    };
    let frames = vec![
        group_frame(100, &[9001], &[(5, "AXIS_NAME", &[9001])], &[]),
        group_frame(
            101,
            &[9001, 44],
            &[(5, "AXIS_NAME", &[9001, 66])],
            &[(44, 9001)],
        ),
        group_frame(
            102,
            &[9001, 44],
            &[(5, "AXIS_NAME", &[9001, 66]), (6, "Custom Pact", &[44])],
            // The same war listed from both sides in one hour is still one run.
            &[(44, 9001), (9001, 44)],
        ),
        // Hour 103 is missing: every run ends at 102.
        group_frame(
            104,
            &[9001],
            &[(6, "Custom Pact", &[44, 55])],
            &[(44, 9001)],
        ),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.humans(),
        [
            run("GER", 100, 102),
            run("FRA", 101, 102),
            run("GER", 104, 104)
        ]
    );
    let (name, first, last, members) = scan.faction(5).unwrap();
    assert_eq!((name, first, last), ("AXIS_NAME", 100, 102));
    assert_eq!(members, [run("GER", 100, 102), run("MOD", 101, 102)]);
    let (name, first, last, members) = scan.faction(6).unwrap();
    assert_eq!((name, first, last), ("Custom Pact", 102, 104));
    assert_eq!(
        members,
        [
            run("FRA", 102, 102),
            run("FRA", 104, 104),
            run("CTL", 104, 104)
        ]
    );
    let pair = |first_hour, last_hour| WarPairRun {
        first: "FRA".into(),
        second: "GER".into(),
        first_hour,
        last_hour,
    };
    assert_eq!(scan.wars(), [pair(101, 102), pair(104, 104)]);
    // The missing hour is listed, so its breaks are not read as joins or leaves.
    let missing = vec![HourSpan {
        first_hour: 103,
        last_hour: 103,
    }];
    assert_eq!(
        scan.unobserved(),
        &UnobservedHours {
            humans: missing.clone(),
            factions: missing.clone(),
            wars: missing
        }
    );
    assert!(scan.warnings().is_empty(), "{:?}", scan.warnings());
}

#[test]
fn country_groups_drop_ended_wars_and_keep_each_history_s_gaps_its_own() {
    use crate::products::country_groups::{HourSpan, MembershipRun, WarPairRun, scan_branch};
    let mut ended = group_frame(101, &[9001, 44], &[], &[(44, 9001)]);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *ended.tables[TableId::WarRelations.index()]
    {
        // Recorded as ending at hour 101: no longer at war from that hour.
        rows[0].start_hour = 50;
        rows[0].end_hour = 101;
    }
    *ended.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let frames = vec![
        group_frame(100, &[9001], &[], &[(44, 9001)]),
        ended,
        group_frame(102, &[9001, 44], &[], &[]),
    ];
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    // The branch ends in a recorded gap: those hours are unobserved, not leaves.
    replay.gaps.push(replay::branches::Gap {
        branch: 1,
        first_hour: 103,
        last_hour: 105,
        reason: "fixture".into(),
    });
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.wars(),
        [WarPairRun {
            first: "FRA".into(),
            second: "GER".into(),
            first_hour: 100,
            last_hour: 100
        }]
    );
    // FRA's human join at 101 is not hidden by the factions table missing that hour.
    assert_eq!(
        scan.humans(),
        [
            MembershipRun {
                tag: "GER".into(),
                first_hour: 100,
                last_hour: 102
            },
            MembershipRun {
                tag: "FRA".into(),
                first_hour: 101,
                last_hour: 102
            }
        ]
    );
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(scan.unobserved().humans, [span(103, 105)]);
    assert_eq!(scan.unobserved().wars, [span(103, 105)]);
    assert_eq!(scan.unobserved().factions, [span(101, 101), span(103, 105)]);
}

#[test]
fn country_groups_leave_out_unknown_countries_and_localise_faction_keys() {
    use crate::products::country_groups::{faction_name, scan_branch};
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![group_frame(100, &[777], &[(5, "", &[9001, 888])], &[])],
        )],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert!(scan.humans().is_empty());
    assert_eq!(scan.faction(5).unwrap().3.len(), 1);
    assert!(
        scan.warnings()
            .iter()
            .any(|w| w.starts_with("2 recorded country references"))
    );
    assert_eq!(faction_name("", None), None);
    assert_eq!(faction_name("My Pact", None).as_deref(), Some("My Pact"));
    let localisation: installed_content::localisation::Localisation = [
        ("AXIS_NAME".to_owned(), "Axis".to_owned()),
        (
            "SCOPED_NAME".to_owned(),
            "[ROOT.GetAdjective] Pact".to_owned(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        faction_name("AXIS_NAME", Some(&localisation)).as_deref(),
        Some("Axis")
    );
    // A key that needs runtime scope has no name rather than showing the key.
    assert_eq!(faction_name("SCOPED_NAME", Some(&localisation)), None);
    assert_eq!(
        faction_name("My Pact", Some(&localisation)).as_deref(),
        Some("My Pact")
    );
}

#[test]
fn measure_runs_compress_equal_hours_break_at_gaps_and_mark_net_drops() {
    use crate::products::country_measure::{BalanceDrop, FixedRun, Measure, scan};
    let run = |first_hour, last_hour, value: &str, starts_segment| FixedRun {
        first_hour,
        last_hour,
        value_raw: value.into(),
        starts_segment,
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                source(1, 100, Some(1_000_000)),
                source(1, 101, Some(1_000_000)),
                source(1, 102, Some(900_000)),
                // A missing value is not zero and ends the run.
                source(1, 103, None),
                source(1, 104, Some(500_000)),
                // Hour 105 is missing entirely: no drop is counted across it.
                source(1, 106, Some(400_000)),
                source(1, 107, Some(450_000)),
            ],
        )],
    )
    .unwrap();
    let result = scan(
        &replay,
        1,
        Measure::PoliticalPower,
        &["GER", "ITA"],
        &AtomicBool::new(false),
    )
    .unwrap();
    let germany = &result["GER"];
    assert_eq!(
        germany.runs,
        [
            run(100, 101, "1000000", true),
            // A new value in the next hour continues the line.
            run(102, 102, "900000", false),
            // After a missing value or a missing hour the line starts again.
            run(104, 104, "500000", true),
            run(106, 106, "400000", true),
            run(107, 107, "450000", false)
        ]
    );
    assert_eq!(
        germany.drops,
        [BalanceDrop {
            hour: 102,
            before_raw: "1000000".into(),
            after_raw: "900000".into()
        }]
    );
    assert!(result["ITA"].runs.is_empty());
}

#[test]
fn measures_reply_every_requested_country_and_warn_for_unrecorded_ones() {
    use crate::products::{
        LoadContext, Product,
        country_measure::{CountryMeasure, Measure, MeasureParams},
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![source(1, 100, Some(1)), source(1, 101, Some(2))],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "pp".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = MeasureParams {
        branch_id: "1".into(),
        measure: Measure::PoliticalPower,
        tags: vec!["GER".into(), "XXX".into()],
    };
    let reply = CountryMeasure::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.decimals, 5);
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "XXX"]
    );
    assert_eq!(reply.series[0].runs.len(), 2);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
    // A second request is answered from the cache.
    let again = CountryMeasure::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(again.series, reply.series);
    // Command power comes from its own field and is cached apart from political power.
    let command = CountryMeasure::load(
        &source,
        &MeasureParams {
            measure: Measure::CommandPower,
            ..params.clone()
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(command.measure, Measure::CommandPower);
    assert_ne!(command.series[0].runs, reply.series[0].runs);
    let too_many = MeasureParams {
        branch_id: "1".into(),
        measure: Measure::PoliticalPower,
        tags: (0..17).map(|index| format!("T{index}")).collect(),
    };
    assert!(CountryMeasure::validate(&too_many).is_err());
}

/// GER (link 9001) holding the given ideas. `GER_schacht_idea` is an appointed
/// advisor: like the game, it is in the appointed-advisor list, not the active ideas.
fn idea_frame(hour: i32, ideas: &[&str]) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let advisor = |country, ordinal, character, idea: &str| replay::rows::HiredAdvisorRow {
        country,
        ordinal,
        character,
        idea: idea.as_bytes().to_vec(),
        slot: b"political_advisor".to_vec(),
    };
    let mut hired = vec![advisor(44, 0, 9, "FRA_advisor")];
    let mut rows: Vec<_> = ideas
        .iter()
        .filter(|token| {
            let appointed = **token == "GER_schacht_idea";
            if appointed {
                hired.push(advisor(9001, 0, 7, token));
            }
            !appointed
        })
        .enumerate()
        .map(|(ordinal, token)| replay::rows::ActiveIdeaRow {
            country: 9001,
            ordinal: ordinal as u32,
            token: token.as_bytes().to_vec(),
        })
        // Another country's idea is never GER's.
        .chain([replay::rows::ActiveIdeaRow {
            country: 44,
            ordinal: 0,
            token: b"FRA_spirit".to_vec(),
        }])
        .collect();
    rows.sort_by_key(|row| (row.country, row.ordinal));
    hired.sort_by_key(|row| (row.country, row.ordinal));
    *frame.tables[TableId::ActiveIdeas.index()] = TableView::Complete(TableRows::ActiveIdeas(rows));
    *frame.tables[TableId::HiredAdvisors.index()] =
        TableView::Complete(TableRows::HiredAdvisors(hired));
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
            id: 7,
            country: 9001,
            token: b"GER_schacht".to_vec(),
            ..Default::default()
        }]));
    frame
}

#[test]
fn idea_history_spans_break_at_gaps_and_advisors_keep_their_recorded_role() {
    use crate::products::idea_history::{IdeaCharacter, IdeaSpan, scan};
    let span = |token: &str, first_hour, last_hour| IdeaSpan {
        token: token.into(),
        first_hour,
        last_hour,
        slot: None,
        character: None,
    };
    // The recorded role goes with the advisor idea's span.
    let advisor = |first_hour, last_hour| IdeaSpan {
        slot: Some("political_advisor".into()),
        character: Some(IdeaCharacter {
            id: 7,
            token: "GER_schacht".into(),
        }),
        ..span("GER_schacht_idea", first_hour, last_hour)
    };
    let frames = vec![
        idea_frame(100, &["civilian_economy", "GER_spirit"]),
        idea_frame(101, &["civilian_economy", "GER_spirit", "GER_schacht_idea"]),
        idea_frame(
            102,
            &[
                "partial_economic_mobilisation",
                "GER_spirit",
                "GER_schacht_idea",
            ],
        ),
        // Hour 103 is missing: every span ends at 102.
        idea_frame(104, &["partial_economic_mobilisation"]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.spans,
        [
            span("GER_spirit", 100, 102),
            span("civilian_economy", 100, 101),
            advisor(101, 102),
            span("partial_economic_mobilisation", 102, 102),
            span("partial_economic_mobilisation", 104, 104),
        ]
    );
    assert_eq!(
        scanned.unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 103,
            last_hour: 103
        }]
    );
    assert!(
        scan(&replay, 1, "FRA", &AtomicBool::new(false))
            .unwrap()
            .spans
            .iter()
            .all(|span| span.token == "FRA_spirit" || span.token == "FRA_advisor")
    );
}

#[test]
fn ideas_join_content_for_category_slot_and_traits() {
    use crate::products::idea_history::{Joined, categories, info};
    let tags = installed_content::ideas::parse_tags(
        b"idea_categories = { government = { slot = economy character_slot = political_advisor character_slot = political_advisor }           country = { type = national_spirit } }",
    )
    .unwrap();
    let ideas = installed_content::ideas::parse_ideas(
        b"ideas = { economy = { law = yes civilian_economy = { modifier = { consumer_goods_factor = 0.35 } } }           country = { GER_spirit = { modifier = { stability_factor = 0.1 } } } }",
    )
    .unwrap();
    let content = installed_content::ideas::IdeaContent {
        tags: installed_content::ideas::IdeaTags { categories: tags },
        order: ideas.iter().map(|idea| idea.token.clone()).collect(),
        ideas: ideas
            .into_iter()
            .map(|idea| (idea.token.clone(), idea))
            .collect(),
    };
    // The advisor's seat and traits come from its character's advisor role.
    let characters = installed_content::characters::parse(
        b"characters = { GER_schacht = { advisor = { slot = political_advisor idea_token = GER_schacht_idea traits = { captain_of_industry } } } }",
    )
    .unwrap();
    let traits = std::collections::BTreeMap::new();
    let localise = |key: &str| (key == "economy").then(|| "Economy".to_owned());
    let joined = Joined {
        ideas: &content,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    let law = info(&joined, "civilian_economy", None);
    assert_eq!(
        (law.category.as_deref(), law.slot.as_deref(), law.law),
        (Some("government"), Some("economy"), true)
    );
    assert_eq!(law.effects[0].value, Some(0.35));
    let spirit = info(&joined, "GER_spirit", None);
    assert_eq!(
        (spirit.category.as_deref(), spirit.slot),
        (Some("country"), None)
    );
    let advisor = info(&joined, "GER_schacht_idea", None);
    assert_eq!(
        (advisor.category.as_deref(), advisor.slot.as_deref()),
        (Some("government"), Some("political_advisor"))
    );
    assert_eq!(advisor.traits[0].token, "captain_of_industry");
    // Content has no such trait: its effects are unknown, not none.
    assert!(!advisor.traits[0].known);
    let unknown = info(&joined, "MOD_mystery", None);
    assert_eq!((unknown.category, unknown.slot), (None, None));
    // An advisor content cannot resolve is placed by the seat the recording gave it.
    let recorded = info(&joined, "MOD_mystery", Some("political_advisor"));
    assert_eq!(
        (recorded.category.as_deref(), recorded.slot.as_deref()),
        (Some("government"), Some("political_advisor"))
    );
    let listed = categories(&joined);
    let government = &listed[0];
    assert_eq!(
        government
            .slots
            .iter()
            .map(|slot| (slot.token.as_str(), slot.character, slot.seats))
            .collect::<Vec<_>>(),
        [("economy", false, 1), ("political_advisor", true, 2)]
    );
    assert_eq!(government.slots[0].name.as_deref(), Some("Economy"));
    assert_eq!(government.slots[0].laws, ["civilian_economy"]);
    assert!(government.slots[1].laws.is_empty());
}

#[test]
fn idea_history_marks_gaps_in_order_and_to_the_branch_end() {
    use crate::products::{country_groups::HourSpan, idea_history::scan};
    let mut incomplete = idea_frame(103, &["GER_spirit"]);
    *incomplete.tables[TableId::ActiveIdeas.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    // Without the appointed advisors an hour is not observed either.
    let mut no_advisors = idea_frame(104, &["GER_spirit"]);
    *no_advisors.tables[TableId::HiredAdvisors.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    // Nor when the country exists but its identity cannot be read.
    let mut unmapped = idea_frame(105, &["GER_spirit"]);
    *unmapped.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    let frames = vec![
        idea_frame(100, &["GER_spirit"]),
        // Hours 101 and 102 are missing, 103's ideas and 104's advisors are incomplete.
        incomplete,
        no_advisors,
        unmapped,
        idea_frame(106, &["GER_spirit"]),
    ];
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    // The branch ends in a recorded gap: those hours are unobserved, not a removal.
    replay.gaps.push(replay::branches::Gap {
        branch: 1,
        first_hour: 107,
        last_hour: 109,
        reason: "fixture".into(),
    });
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(scanned.unobserved, [span(101, 105), span(107, 109)]);
}

fn stability_inputs() -> replay::rows::CountryStabilityInputRow {
    replay::rows::CountryStabilityInputRow {
        country: 9001,
        stability_raw: 50_000,
        war_support_raw: 40_000,
        ruling_party_popularity_raw: 4_500_000,
        propaganda_stability_q15: -3277,
        coastal_protection_stability_raw: 2_500,
        interception_sum_raw: 10_000,
        world_tension_raw: 50_000,
        base_stability_war_factor: -20_000,
        base_stability_party_popularity_factor: 15_000,
        min_stability: 0,
        max_stability: 100_000,
        war_support_offensive_war: -20_000,
        war_support_defensive_war: 20_000,
        war_support_tension_impact: 40_000,
        min_war_support: 0,
        max_war_support: 100_000,
        interception_war_support_scale_q15: 16_384,
        interception_bombing_war_support_impact: 30_000,
        base_max_command_power: 8_000_000,
        stability_factor: Some(30_000),
        party_popularity_stability_factor: Some(0),
        war_stability_factor: Some(-10_000),
        offensive_war_stability_factor: Some(5_000),
        defensive_war_stability_factor: Some(0),
        war_support_factor: Some(0),
        stability_weekly: Some(0),
        war_support_weekly: Some(0),
        max_command_power: Some(1_000_000),
        max_command_power_mult: Some(10_000),
        keys: vec![
            0x8d, 0x90, 0x91, 0x17b, 0x17c, 0x95, 0x8e, 0x96, 0x14b, 0x14d,
        ],
        language: b"l_english".to_vec(),
        ..Default::default()
    }
}

#[test]
fn stability_formula_follows_the_engine_getters() {
    use crate::products::country_stability::formula::*;
    // Q15 conversions: rounding half up on the fraction, truncation back.
    assert_eq!(q15_to_fixed5(32_768), 100_000);
    assert_eq!(q15_to_fixed5(16_384), 50_000);
    assert_eq!(q15_to_fixed5(-3277), -10_001);
    // Whole parts beyond the engine's bound saturate.
    assert_eq!(q15_to_fixed5(0x36f9_bfb2 << 15), 0x53e2_d61f_d320);
    assert_eq!(q15_to_fixed5(-0x36f9_bfb2 << 15), -0x53e2_d61f_d321);
    assert_eq!(fixed5_to_q15(-20_000), -6553);
    assert_eq!(fixed5_to_q15(100_000), 32_768);
    // Popularity: two truncating divisions, toward zero for a negative factor.
    assert_eq!(popularity_term(15_000, 4_500_000), 6_750);
    assert_eq!(popularity_term(15_000, 3_333_333), 4_999);
    assert_eq!(popularity_term(-15_000, 3_333_333), -4_999);
    // At an offensive war: base, war term, popularity, propaganda, factor, coast.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.offensive_war = 1;
    let war = stability(&row);
    assert_eq!(
        war.unclamped,
        Some(50_000 - 25_000 + 6_750 - 10_001 + 30_000 + 2_500)
    );
    assert_eq!(war.value, war.unclamped);
    // An offensive war without IsAtWar(mode 1) adds nothing, as at peace.
    row.at_war = 0;
    assert_eq!(stability(&row).value, Some(79_249));
    // Clamped to the defines.
    row.stability_factor = Some(90_000);
    let clamped = stability(&row);
    assert_eq!(
        (clamped.unclamped, clamped.value),
        (Some(139_249), Some(100_000))
    );
    row.stability_raw = -500_000;
    assert_eq!(stability(&row).value, Some(0));
    // An unregistered key needed at war leaves the value unknown.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.defensive_war = 1;
    row.defensive_war_stability_factor = None;
    assert_eq!(stability(&row).value, None);
    // War support: war define, tension, interception scaled and clamped.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.offensive_war = 1;
    assert_eq!(interception_term(&row), 4_999);
    assert_eq!(
        war_support(&row).value,
        Some(40_000 - 20_000 + 20_000 + 4_999)
    );
    row.interception_sum_raw = -10_000_000;
    assert_eq!(interception_term(&row), -30_000);
    // Maximum command power: (base + flat) * (1 + mult).
    assert_eq!(max_command_power(&row).value, Some(9_900_000));
    row.max_command_power_mult = None;
    assert_eq!(max_command_power(&row).value, None);
}

fn stability_frame(
    hour: i32,
    inputs: replay::rows::CountryStabilityInputRow,
    read: i32,
    lines: Vec<replay::rows::CountryStabilityLineRow>,
) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let reads = [
        TableId::CountryStabilityInputs,
        TableId::CountryStabilityLines,
    ]
    .into_iter()
    .filter(|table| *table == TableId::CountryStabilityInputs || !lines.is_empty())
    .map(|table| replay::rows::SliceReadRow {
        table: table.index() as u32,
        owner: 9001,
        sample_hour: read,
    })
    .collect();
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(vec![inputs]));
    *frame.tables[TableId::CountryStabilityLines.index()] =
        TableView::Complete(TableRows::CountryStabilityLines(lines));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            replay::rows::ModifierDefinitionRow {
                index: 0x8d,
                token: 1,
                flags: 3,
                ..Default::default()
            },
        ]));
    frame
}

#[test]
fn stability_product_runs_carry_reads_and_break_down_one_hour() {
    use crate::products::country_stability::scan;
    use replay::rows::CountryStabilityLineRow as Line;
    let line = |line, kind, identity: &[u8], value, text: &[u8]| Line {
        country: 9001,
        line,
        key: 0x8d,
        kind,
        identity: identity.to_vec(),
        value,
        text: text.to_vec(),
        ..Default::default()
    };
    let lines = vec![
        line(0, Line::IDEA, b"idea_a", 10_000, b""),
        line(2, Line::UNPAIRED, b"", 15_000, b"Decision: X"),
    ];
    let mut changed = stability_inputs();
    changed.stability_raw = 60_000;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(100, stability_inputs(), 100, lines.clone()),
                // Carried from the hour-100 read.
                stability_frame(101, stability_inputs(), 100, lines),
                stability_frame(102, changed, 102, vec![]),
                // Hour 103 is missing: the next hour starts a new segment.
                stability_frame(104, stability_inputs(), 104, vec![]),
            ],
        )],
    )
    .unwrap();
    let (result, warnings) = scan(
        &replay,
        1,
        &["GER", "ITA"],
        Some(101),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .stability
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.value_raw.as_str(),
                run.starts_segment,
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (100, 101, "79249", true),
            (102, 102, "89249", false),
            (104, 104, "79249", true)
        ]
    );
    assert_eq!(germany.read_hours, [100, 102, 104]);
    let breakdown = germany.breakdown.as_ref().unwrap();
    assert_eq!((breakdown.hour, breakdown.read_hour), (101, 100));
    assert_eq!(breakdown.language, "l_english");
    assert_eq!(breakdown.stability.value_raw.as_deref(), Some("79249"));
    assert!(breakdown.lines_available);
    let factor = &breakdown.modifiers[0];
    assert_eq!(factor.key, "stability_factor");
    assert_eq!(factor.good, Some(true));
    assert_eq!(factor.lines.len(), 2);
    assert_eq!(factor.lines[0].identity, "idea_a");
    assert_eq!(factor.lines[1].kind, "unpaired");
    assert_eq!(factor.lines[1].text.as_deref(), Some("Decision: X"));
    // 30000 recorded, 25000 in lines: the rest is shown, not hidden.
    assert_eq!(factor.remainder_raw.as_deref(), Some("5000"));
    assert!(result["ITA"].stability.is_empty());
    // A breakdown at an hour whose lines came from another read says so.
    let (result, warnings) =
        scan(&replay, 1, &["GER"], Some(102), &AtomicBool::new(false)).unwrap();
    let breakdown = result["GER"].breakdown.as_ref().unwrap();
    assert!(breakdown.lines_available && breakdown.modifiers[0].lines.is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn a_stability_breakdown_needs_the_same_country_identity_as_its_history() {
    use crate::products::{
        LoadContext, Product,
        country_stability::{CountryStability, CountryStabilityParams},
    };
    let mut unlinked = stability_frame(101, stability_inputs(), 101, vec![]);
    *unlinked.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    stability_frame(100, stability_inputs(), 100, vec![]),
                    unlinked,
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |hour| {
        let params = CountryStabilityParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            hour: Some(hour),
        };
        CountryStability::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    assert!(load(100).series[0].breakdown.is_some());
    // CountryLinks no longer maps GER: the history leaves the hour out, and
    // so does the breakdown.
    assert!(load(101).series[0].breakdown.is_none());
}
