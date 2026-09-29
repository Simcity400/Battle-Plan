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
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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
    // As recorded: the capitulation table is present, here with no one capitulated.
    *frame.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(Vec::new()));
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
    // The lists that named an unknown country are incomplete that hour: the humans
    // list, and that faction's own roster (not every faction's); the wars list was complete.
    let hour = [crate::products::country_groups::HourSpan {
        first_hour: 100,
        last_hour: 100,
    }];
    assert_eq!(scan.unobserved().humans, hour);
    assert!(scan.unobserved().factions.is_empty());
    assert_eq!(scan.faction_incomplete(5), Some(&hour[..]));
    assert!(scan.unobserved().wars.is_empty());
    assert!(
        scan.warnings()
            .iter()
            .any(|w| w.starts_with("2 recorded country references"))
    );
    assert_eq!(faction_name("", None), Ok(None));
    assert_eq!(faction_name("My Pact", None), Ok(Some("My Pact".into())));
    let localisation = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " AXIS_NAME:0 \"Achse\"\n",
        " SCOPED_NAME:0 \"[ROOT.GetAdjective] Pact\"\n",
        " PRICE_NAME:0 \"Pakt $$ ]\"\n",
    ));
    assert_eq!(
        faction_name("AXIS_NAME", Some(&localisation)),
        Ok(Some("Achse".into()))
    );
    // Faction names reach their text boxes plain (140D77360): brackets are
    // glyphs.
    assert_eq!(
        faction_name("SCOPED_NAME", Some(&localisation)),
        Ok(Some("[ROOT.GetAdjective] Pact".into()))
    );
    // The game's localiser: `$$` is one `$` and a lone `]` a glyph.
    assert_eq!(
        faction_name("PRICE_NAME", Some(&localisation)),
        Ok(Some("Pakt $ ]".into()))
    );
    // A typed name is copied as its bytes.
    assert_eq!(
        faction_name("Pacte d'Acier", Some(&localisation)),
        Ok(Some("Pacte d'Acier".into()))
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
    // As recorded: the saved name in `token`, the script token as a lexer id.
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
            id: 7,
            country: 9001,
            name_token_id: 42,
            token: b"Hjalmar Schacht".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay::rows::LexicalTokenRow {
                token: 42,
                literal: b"GER_schacht".to_vec(),
            },
        ]));
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
            name: "Hjalmar Schacht".into(),
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
    // The identity starts at its first hour and again after the missing hour.
    assert_eq!(scanned.identity_starts, [100, 104]);
    assert!(
        scan(&replay, 1, "FRA", &AtomicBool::new(false))
            .unwrap()
            .spans
            .iter()
            .all(|span| span.token == "FRA_spirit" || span.token == "FRA_advisor")
    );
}

#[test]
fn advisor_traits_select_the_first_native_name_match() {
    use crate::products::idea_history::{Joined, info};
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/country_leader/a.txt",
            "leader_traits = { Brilliant = { army_morale_factor = 0.1 } brilliant = { army_morale_factor = 0.2 } \
             1trait = { army_morale_factor = 0.3 } 2trait = { army_morale_factor = 0.4 } }",
        ),
        (
            "common/country_leader/b.txt",
            "leader_traits = { BRILLIANT = { army_morale_factor = 0.5 } lost = { odd = { } } }",
        ),
        (
            "common/country_leader/c.txt",
            "leader_traits = { Later = { army_morale_factor = 0.6 } }",
        ),
        (
            "localisation/traits_l_english.yml",
            "\u{feff}l_english:\n Brilliant:0 \"First trait\"\n bRiLLiant:0 \"Reference spelling\"\n",
        ),
    ]);
    let (traits, _) = installed_content::leader_traits::read(&catalogue.content.layers).unwrap();
    let characters = installed_content::characters::parse(
        b"characters = { holder = { advisor = { slot = political_advisor idea_token = advisor traits = { bRiLLiant 1trait 2trait Later Missing } } } }",
    )
    .unwrap();
    let ideas = installed_content::ideas::IdeaContent::default();
    let localise = |key: &str, view| catalogue.localised(key, view);
    let joined = Joined {
        ideas: &ideas,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    let advisor = info(&joined, "advisor", None, Err("no holder"));
    // 1412894E0 -> 140712D20 -> 140710200 keeps the first name match.
    // The following unread suffix cannot change that match, but can hide
    // a match before Later or define Missing.
    let first = &advisor.traits[0];
    assert!(first.known);
    assert_eq!(first.unknown, None);
    assert_eq!(first.lines[0].fixed5, "10000");
    assert_eq!(first.name.as_deref(), Some("First trait"));
    // Names rejected by the lexer are still distinct hashed-name lookups.
    assert_eq!(advisor.traits[1].lines[0].fixed5, "30000");
    assert_eq!(advisor.traits[2].lines[0].fixed5, "40000");
    for unknown in &advisor.traits[3..] {
        assert!(!unknown.known);
        assert_eq!(unknown.name, None);
        assert!(unknown.unknown.as_deref().unwrap().contains("b.txt"));
    }
}

#[test]
fn ideas_join_content_for_category_slot_and_traits() {
    use crate::products::idea_history::{Joined, categories, info};
    let tags = installed_content::ideas::parse_tags(
        b"idea_categories = { government = { slot = economy character_slot = political_advisor character_slot = political_advisor }           country = { type = national_spirit } military_staff = { character_slot = theorist } }",
    );
    // An officer-corps group's entries are character advisor templates
    let file = installed_content::ideas::parse_ideas(
        b"ideas = { economy = { law = yes civilian_economy = { modifier = { consumer_goods_factor = 0.35 } } }           country = { GER_spirit = { modifier = { STABILITY_FACTOR = 0.1 hidden_modifier = { Stability_Factor = 0.2 } } } }           theorist = { GER_theorist = { traits = { military_theorist } modifier = { army_org = 1 } } } }",
        &["theorist".to_owned()],
    );
    let content = installed_content::ideas::IdeaContent::from_parsed(tags, file.ideas);
    // The advisor's seat and traits come from its character's advisor role.
    let mut characters = installed_content::characters::parse(
        b"characters = { GER_schacht = { advisor = { slot = political_advisor idea_token = GER_schacht_idea traits = { captain_of_industry } } }           GER_split = { instance = { allowed = { has_dlc = \"A\" } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { a } } }             instance = { allowed = { NOT = { has_dlc = \"A\" } } advisor = { slot = political_advisor idea_token = GER_split_idea traits = { b } } } } }",
    )
    .unwrap();
    characters.merge(file.advisors);
    let traits = installed_content::leader_traits::LeaderTraits::default();
    let localise =
        |key: &str, _: crate::catalogue::View| (key == "economy").then(|| "Economy".to_owned());
    let joined = Joined {
        ideas: &content,
        characters: &characters,
        traits: &traits,
        localise: &localise,
    };
    const NO_HOLDER: Result<&installed_content::characters::BuiltTemplate, &str> = Err("no holder");
    let law = info(&joined, "civilian_economy", None, NO_HOLDER);
    assert_eq!(
        (law.category.as_deref(), law.slot.as_deref(), law.law),
        (Some("government"), Some("economy"), true)
    );
    assert_eq!(law.effects[0].fixed5, "35000");
    let spirit = info(&joined, "GER_spirit", None, NO_HOLDER);
    assert_eq!(
        (spirit.category.as_deref(), spirit.slot),
        (Some("country"), None)
    );
    // One line per modifier, by the identity its folded key names
    // (1424A7A90), the last value kept and hidden by `hidden_modifier`.
    assert_eq!(
        spirit.effects,
        [crate::products::idea_history::EffectLine {
            key: "stability_factor".into(),
            fixed5: "20000".into(),
            hidden: true,
        }]
    );
    let advisor = info(&joined, "GER_schacht_idea", None, NO_HOLDER);
    assert_eq!(
        (advisor.category.as_deref(), advisor.slot.as_deref()),
        (Some("government"), Some("political_advisor"))
    );
    assert_eq!(advisor.traits[0].token, "captain_of_industry");
    assert_eq!(content.idea("GER_theorist"), Ok(None));
    let theorist = info(&joined, "GER_theorist", None, NO_HOLDER);
    assert_eq!(
        (theorist.category.as_deref(), theorist.slot.as_deref()),
        (Some("military_staff"), Some("theorist"))
    );
    // The officer template keeps its role's `modifier` (1413CA970).
    assert!(theorist.effects_known);
    assert_eq!(
        (
            theorist.effects[0].key.as_str(),
            theorist.effects[0].fixed5.as_str()
        ),
        ("army_org", "100000")
    );
    assert_eq!(theorist.traits[0].token, "military_theorist");
    // Content has no such trait: its effects are unknown, not none.
    assert!(!advisor.traits[0].known);
    // Instances that disagree without a holder's template: the reason says
    // so rather than "not in the content files".
    let split = info(
        &joined,
        "GER_split_idea",
        Some("political_advisor"),
        NO_HOLDER,
    );
    assert!(!split.effects_known);
    assert!(
        split
            .unknown
            .as_deref()
            .is_some_and(|reason| reason.starts_with(
                "advisor GER_split_idea: its character's alternatives define it differently"
            ) && reason.ends_with("no holder")),
        "{:?}",
        split.unknown
    );
    let unknown = info(&joined, "MOD_mystery", None, NO_HOLDER);
    assert_eq!((unknown.category, unknown.slot), (None, None));
    // An advisor content cannot resolve is placed by the seat the recording gave it.
    let recorded = info(&joined, "MOD_mystery", Some("political_advisor"), NO_HOLDER);
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
    assert_eq!(scanned.identity_starts, [100, 106]);
}

/// GER (link 9001) with two parties at the given popularities (raw), one ruling,
/// and its last election hour.
fn party_frame(
    hour: i32,
    fascism: i64,
    democratic: i64,
    ruling: &str,
    election: i32,
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let party = |id: u32, token: &str, popularity: i64| replay::rows::PoliticalPartyRow {
        country: 9001,
        id,
        popularity,
        is_ruling: u8::from(token == ruling),
        token: token.as_bytes().to_vec(),
        ..Default::default()
    };
    let mut parties = vec![
        party(1, "democratic", democratic),
        party(2, "fascism", fascism),
        // Another country's party is never GER's.
        replay::rows::PoliticalPartyRow {
            country: 44,
            id: 1,
            popularity: 1,
            is_ruling: 1,
            token: b"democratic".to_vec(),
            ..Default::default()
        },
    ];
    parties.sort_by_key(|row| (row.country, row.id));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(parties));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            last_election: election,
            ..Default::default()
        }]));
    frame
}

#[test]
fn party_history_follows_popularity_ruling_changes_and_elections() {
    use crate::products::party_history::{RulingRun, scan};
    let frames = vec![
        party_frame(100, 40_00000, 35_00000, "fascism", 10),
        party_frame(101, 40_00000, 35_00000, "fascism", 10),
        party_frame(102, 45_00000, 30_00000, "fascism", 10),
        // Hour 103 is missing; at 104 an election was held and the democrats rule.
        party_frame(104, 20_00000, 60_00000, "democratic", 104),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let fascism = &scanned.parties["fascism"];
    assert_eq!(
        fascism
            .runs
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.value_raw.as_str(),
                run.starts_segment
            ))
            .collect::<Vec<_>>(),
        [
            (100, 101, "4000000", true),
            (102, 102, "4500000", false),
            (104, 104, "2000000", true)
        ]
    );
    let ruling = |token: &str, first_hour, last_hour| RulingRun {
        token: token.into(),
        first_hour,
        last_hour,
        starts_segment: true,
    };
    assert_eq!(
        scanned.ruling,
        [ruling("fascism", 100, 102), ruling("democratic", 104, 104)]
    );

    assert_eq!(scanned.elections, [104]);
    assert_eq!(
        scanned.unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 103,
            last_hour: 103
        }]
    );
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
    // Maximum command power: (base + flat) * (1 + mult), with the modifier
    // getter values and the allocated power from CountryCommandPower.
    assert_eq!(
        max_command_power(&row, Some(&launch(0))).value,
        Some(9_900_000)
    );
    // The game shows and clamps to the maximum minus the allocated command
    // power, which can be negative.
    assert_eq!(
        max_command_power(&row, Some(&launch(400_000))).value,
        Some(9_500_000)
    );
    assert_eq!(
        max_command_power(&row, Some(&launch(-10_600_000))).value,
        Some(20_500_000)
    );
    // No CountryCommandPower row: unknown, never a zero allocation.
    assert_eq!(max_command_power(&row, None).value, None);
    let mut unregistered = launch(0);
    unregistered.max_command_power_factor = None;
    assert_eq!(max_command_power(&row, Some(&unregistered)).value, None);
}

/// The country's CountryCommandPower row: the capacity modifiers' getter
/// values and `allocated` command power.
fn launch(allocated: i64) -> replay::rows::CountryCommandPowerRow {
    replay::rows::CountryCommandPowerRow {
        canonical_slot: 7,
        allocated_command_power_raw: allocated,
        max_command_power: Some(1_000_000),
        max_command_power_factor: Some(10_000),
        ..Default::default()
    }
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

/// The maximum command power takes the allocated power from
/// CountryCommandPower, joined through CountryLinks by canonical slot at the
/// same frame; an hour without that row has no value, never a zero one.
#[test]
fn stability_product_takes_the_allocated_command_power_from_raid_country_inputs() {
    use crate::products::country_stability::scan;
    let with_allocation = |hour, allocated: Option<i64>| {
        let mut frame = stability_frame(hour, stability_inputs(), hour, vec![]);
        *frame.tables[TableId::CountryCommandPower.index()] = TableView::Complete(
            TableRows::CountryCommandPower(allocated.map(launch).into_iter().collect()),
        );
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                with_allocation(100, Some(400_000)),
                with_allocation(101, None),
            ],
        )],
    )
    .unwrap();
    let (result, _) = scan(&replay, 1, &["GER"], Some(101), &AtomicBool::new(false)).unwrap();
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .max_command_power
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
        .collect();
    assert_eq!(runs, [(100, 100, "9500000")]);
    let breakdown = germany.breakdown.as_ref().unwrap();
    assert_eq!(breakdown.max_command_power.value_raw, None);
    let allocated = breakdown
        .max_command_power
        .terms
        .iter()
        .find(|term| term.term == "allocatedCommandPower")
        .unwrap();
    assert_eq!(allocated.value_raw, None);
}

/// A maximum computed from a carried stability read and fresh command power
/// names only the stability read: CountryCommandPower is read every hour.
#[test]
fn stability_product_names_only_the_stability_read_of_the_maximum() {
    use crate::products::country_stability::scan;
    let frame = |hour, stability_read, allocated| {
        let mut frame = stability_frame(hour, stability_inputs(), stability_read, vec![]);
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(vec![launch(allocated)]));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                frame(100, 100, 400_000),
                // Stability carried from 100; the allocation read fresh.
                frame(101, 100, 500_000),
                frame(102, 102, 500_000),
            ],
        )],
    )
    .unwrap();
    let (result, _) = scan(&replay, 1, &["GER"], None, &AtomicBool::new(false)).unwrap();
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .max_command_power
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
        .collect();
    assert_eq!(runs, [(100, 100, "9500000"), (101, 102, "9400000")]);
    let reads: Vec<_> = germany
        .carried
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.read_hour))
        .collect();
    assert_eq!(reads, [(101, 101, 100)]);
}

/// The stability product's bytes are the same as before the command power
/// values moved to CountryCommandPower: the maximum command power breakdown
/// and the modifier totals, built from the same values the old way (all on
/// the stability row) and the new way, serialise identically.
#[test]
fn stability_output_is_byte_identical_with_the_values_on_country_command_power() {
    use crate::products::country_stability::{
        breakdown, formula, modifiers, modifiers_with_totals,
    };
    // The old formula, over values the stability row used to hold.
    fn old_max_command_power(
        base: i64,
        max: Option<i64>,
        mult: Option<i64>,
        allocated: i64,
    ) -> formula::Terms {
        let value = (|| {
            let base = base.wrapping_add(max?);
            Some((mult?.wrapping_add(100_000).wrapping_mul(base) / 100_000).wrapping_sub(allocated))
        })();
        formula::Terms {
            terms: vec![
                ("baseMaxCommandPower", Some(base)),
                ("maxCommandPower", max),
                ("maxCommandPowerMult", mult),
                ("allocatedCommandPower", Some(allocated)),
            ],
            unclamped: value,
            value,
        }
    }
    let row = stability_inputs();
    for (max, mult, allocated) in [
        (Some(1_000_000), Some(10_000), 400_000),
        (Some(0), Some(0), -10_600_000),
        (None, Some(10_000), 0),
        (Some(5), None, 7),
    ] {
        let launch = replay::rows::CountryCommandPowerRow {
            canonical_slot: 7,
            allocated_command_power_raw: allocated,
            max_command_power: max,
            max_command_power_factor: mult,
            ..Default::default()
        };
        let old = breakdown(old_max_command_power(
            row.base_max_command_power,
            max,
            mult,
            allocated,
        ));
        let new = breakdown(formula::max_command_power(&row, Some(&launch)));
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        // The old totals: all ten on the stability row.
        let stability = row.totals();
        let old_totals: [Option<i64>; 10] = std::array::from_fn(|index| match index {
            8 => max,
            9 => mult,
            index => stability[index],
        });
        let mut refused = crate::catalogue::Refusals::default();
        let old =
            modifiers_with_totals(&row, old_totals, None, |_| Some(true), "FRA", &mut refused);
        let new = modifiers(
            &row,
            Some(&launch),
            None,
            |_| Some(true),
            "FRA",
            &mut refused,
        );
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
    }
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
    let carried: Vec<_> = germany
        .carried
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.read_hour))
        .collect();
    // Only hour 101 carries an earlier (hour-100) read; the others were read then.
    assert_eq!(carried, [(101, 101, 100)]);
    assert!(germany.recorded);
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
    let load_with = |hour, breakdown_only| {
        let params = CountryStabilityParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            hour: Some(hour),
            breakdown_only,
        };
        CountryStability::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let load = |hour| load_with(hour, false);
    assert!(load(100).series[0].breakdown.is_some());
    // A breakdown alone leaves the histories out.
    let alone = load_with(100, true);
    assert!(alone.series[0].breakdown.is_some());
    assert!(alone.series[0].stability.is_empty() && !alone.series[0].recorded);
    assert!(alone.warnings.is_empty(), "{:?}", alone.warnings);
    // CountryLinks no longer maps GER: the history leaves the hour out, and
    // so does the breakdown.
    assert!(load(101).series[0].breakdown.is_none());
}

#[test]
fn country_groups_date_war_starts_and_capitulations_as_the_engine_does() {
    use crate::products::country_groups::{Capitulation, WarStart, scan_branch};
    let frame = |hour, capitulated: bool| {
        let mut frame = group_frame(hour, &[], &[], &[]);
        // GER's side (9001) started a war with FRA (44) at hour 100; GER is listed second.
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![
                replay::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 9001,
                    start_hour: 100,
                    first_was_instigator: 0,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![
                replay::rows::CapitulationRow {
                    country: 44,
                    capitulated: u8::from(capitulated),
                    capitulated_hour: 101,
                    ..Default::default()
                },
            ]));
        frame
    };
    // At hour 102 GER's link id names another identity: the war and the
    // capitulation keep the countries they were first seen with.
    let mut renamed = frame(102, true);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.tag == b"GER") {
            row.tag = b"BBB".to_vec();
        }
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![frame(100, false), frame(101, true), renamed],
        )],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scan.war_starts(),
        [WarStart {
            hour: 100,
            attacking: "GER".into(),
            defending: "FRA".into()
        }]
    );
    assert_eq!(
        scan.capitulations(),
        [Capitulation {
            hour: 101,
            tag: "FRA".into()
        }]
    );
    assert_eq!(
        scan.identity_changes(),
        [crate::products::country_groups::IdentityChange {
            hour: 102,
            from: "GER".into(),
            to: "BBB".into()
        }]
    );
    assert!(scan.warnings().is_empty(), "{:?}", scan.warnings());
    // A war dated before it was first seen, after an unrecorded hour or once its
    // id names another country, cannot be named at its own hour: it is left out.
    let without_war = |hour| {
        let mut frame = frame(hour, false);
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![]));
        frame
    };
    let late = |hour, start_hour| {
        let mut frame = frame(hour, false);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows[0].start_hour = start_hour;
        }
        frame
    };
    let mut renamed_late = late(101, 100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed_late.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.tag == b"GER") {
            row.tag = b"BBB".to_vec();
        }
    }
    for frames in [
        vec![without_war(100), late(102, 101)],
        vec![without_war(100), renamed_late],
    ] {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert!(scan.war_starts().is_empty(), "{:?}", scan.war_starts());
        assert!(
            scan.warnings()
                .iter()
                .any(|warning| warning.contains("which countries they were is not known")),
            "{:?}",
            scan.warnings()
        );
    }
    // An hour whose capitulations cannot be read says so.
    let mut unread = frame(103, true);
    *unread.tables[TableId::Capitulation.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(100, false), unread])],
    )
    .unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert!(
        scan.warnings()
            .iter()
            .any(|warning| warning.contains("Capitulations could not be read"))
    );
}

#[test]
fn stability_parts_add_up_to_the_value_and_are_known_only_where_recorded() {
    use crate::products::{country_groups::HourSpan, stability_parts::scan_parts};
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
    // stability_factor is 30 000: the two lines explain 25 000, the rest is a remainder.
    let lines = vec![
        line(0, Line::IDEA, b"idea_a", 10_000, b""),
        line(2, Line::UNPAIRED, b"", 15_000, b"Decision: X"),
    ];
    // A read whose lines table was not recorded: its parts are not known.
    let mut unlined = stability_frame(103, stability_inputs(), 103, vec![]);
    *unlined.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(100, stability_inputs(), 100, lines.clone()),
                // Carried from the hour-100 read.
                stability_frame(101, stability_inputs(), 100, lines),
                // A read with no lines: the whole modifier term is unexplained.
                stability_frame(102, stability_inputs(), 102, vec![]),
                unlined,
            ],
        )],
    )
    .unwrap();
    let (stability, war_support, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    let observed = [HourSpan {
        first_hour: 100,
        last_hour: 102,
    }];
    assert_eq!(stability.observed, observed);
    assert_eq!(war_support.observed, observed);
    let at = |parts: &crate::products::stability_parts::MeasureParts, hour: i32| -> i64 {
        parts
            .parts
            .iter()
            .flat_map(|part| &part.runs)
            .filter(|run| run.first_hour <= hour && hour <= run.last_hour)
            .map(|run| run.value_raw.parse::<i64>().unwrap())
            .sum()
    };
    // The parts add up to the displayed values (79.249% and the war support total).
    assert_eq!(at(&stability, 100), 79_249);
    assert_eq!(at(&stability, 101), 79_249);
    assert_eq!(at(&stability, 102), 79_249);
    let remainder = stability
        .parts
        .iter()
        .find(|part| part.id == "remainder")
        .unwrap();
    assert_eq!(
        remainder
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "5000"), (102, 102, "30000")]
    );
    let displayed = crate::products::country_stability::formula::war_support(&stability_inputs());
    assert_eq!(at(&war_support, 100), displayed.value.unwrap());
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    for id in [
        "term:base",
        "line:idea:idea_a:0:0",
        "unpaired:Decision: X",
        "remainder",
    ] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let idea = stability
        .parts
        .iter()
        .find(|part| part.id == "line:idea:idea_a:0:0")
        .unwrap();
    assert_eq!(
        idea.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "10000")]
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("1 reads recorded no modifier lines"))
    );
    // Lines missing at one hour and recorded again at the next hour of the same
    // read: the parts are known again there, not lost until the next read.
    let mut missing = stability_frame(200, stability_inputs(), 200, vec![]);
    *missing.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                missing,
                stability_frame(201, stability_inputs(), 200, vec![]),
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 201,
            last_hour: 201
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    // Parts carried from a read are not reused for an hour whose lines table is missing.
    let mut dropped = stability_frame(301, stability_inputs(), 300, vec![]);
    *dropped.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(300, stability_inputs(), 300, vec![]),
                dropped,
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 300,
            last_hour: 300
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn stability_parts_name_a_read_s_characters_as_recorded_at_that_read() {
    use crate::products::stability_parts::scan_parts;
    use replay::rows::CountryStabilityLineRow as Line;
    let leader = vec![Line {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: Line::COUNTRY_LEADER,
        identity_id: 7,
        value: 30_000,
        ..Default::default()
    }];
    let named = |mut frame: DecodedFrame, saved: &str| {
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
                id: 7,
                token: saved.as_bytes().to_vec(),
                name_token_id: -1,
                ..Default::default()
            }]));
        frame
    };
    // Read 100's lines are missing at hour 100 and arrive at 101, after a rename.
    let mut first = named(
        stability_frame(100, stability_inputs(), 100, leader.clone()),
        "Old Name",
    );
    *first.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let second = named(
        stability_frame(101, stability_inputs(), 100, leader),
        "New Name",
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, second])]).unwrap();
    let (stability, _, _) = scan_parts(&replay, 1, "GER").unwrap();
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with(":Old Name")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with(":New Name")), "{ids:?}");
}

/// OriginalTagLists::derive reads the branch from its first frame: a load
/// gives slot order (1401DA010), later name_tag changes append by hour.
#[test]
fn original_tag_lists_derive_from_a_loaded_branch() {
    use crate::scope::{OriginalTagLists, RecordedScopes, tests::dynamic_frame};
    let frames = vec![
        dynamic_frame(10, ["GER", "GER", ""], [0, 1, 1, 1]),
        dynamic_frame(11, ["", "GER", ""], [0, 1, 1, 1]),
        dynamic_frame(12, ["", "GER", "GER"], [0, 0, 0, 1]),
    ];
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(10),
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::LoadSignal,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for mut frame in frames.clone() {
        frame.branch = 1;
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let derived = OriginalTagLists::derive(
        &replay,
        1,
        &[10, 12].into_iter().collect(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let resolve = |hour: i32, frame: &DecodedFrame| {
        RecordedScopes::read_with(frame, derived[&hour].clone(), Default::default())
            .resolve_tag("GER")
    };
    // At the load, D01 (slot 1) comes first.
    assert_eq!(resolve(10, &frames[0]), Ok(1));
    // At hour 12 only D03 (appended at hour 12) owns a state.
    assert_eq!(resolve(12, &frames[2]), Ok(3));
    let members: Vec<Vec<u32>> = derived[&12]
        .list("GER")
        .iter()
        .map(|group| group.members.clone())
        .collect();
    assert_eq!(members, [vec![2], vec![3]]);
}

/// Germany (raw links 11 and `germany`, slot 7), Italy (21, slot 8) and Spain
/// (31, slot 9), with steel and oil. `read` times a walk of the resource slot's
/// exchange table only, as a shared read times only its anchor; the country
/// table is timed every hour. Deals are (handle, exporter, importer, delivered).
fn resource_frame(
    hour: i32,
    read: bool,
    steel: i64,
    germany: u32,
    deals: &[(u64, u32, u32, i64)],
) -> DecodedFrame {
    use replay::rows::{
        CountryResourceRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow,
    };
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(31, 9, b"SPR"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(31, 9),
            link(germany, 7),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 501,
                literal: b"steel".to_vec(),
            },
            LexicalTokenRow {
                token: 502,
                literal: b"oil".to_vec(),
            },
        ]));
    *frame.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(vec![
            ResourceDefinitionRow {
                id: 1,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            },
            ResourceDefinitionRow {
                id: 2,
                token: 502,
                active: 1,
                factory_cost: 12_500,
            },
        ]));
    let pools = |country, resource, produced| CountryResourceRow {
        country,
        resource,
        produced,
        imported: 800_000,
        to_export: 2_500_000,
        exported: 1_700_000,
        used: produced + 800_000 - 2_500_000,
        // Special projects take 0.4; the production lines need 30.
        special_total: produced + 800_000 - 2_500_000 - 40_000,
        production_demand: -3_000_000,
        ..Default::default()
    };
    *frame.tables[TableId::CountryResources.index()] =
        TableView::Complete(TableRows::CountryResources(vec![
            // Germany's rows name it by its other link.
            pools(11, 1, steel),
            pools(11, 2, 0),
            pools(21, 1, 99_900_000),
        ]));
    *frame.tables[TableId::ResourceExchanges.index()] =
        TableView::Complete(TableRows::ResourceExchanges(
            deals
                .iter()
                .map(|&(id, exporter, importer, delivered)| ResourceExchangeRow {
                    id,
                    exporter,
                    importer,
                    resource: 1,
                    delivered,
                    assigned_factories: 1,
                    required_factories: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    if read {
        frame.metrics.table_ns[TableId::ResourceExchanges.index()] = 1;
    }
    frame
}

#[test]
fn resource_values_start_at_reads_carry_until_the_next_and_never_cross_gaps() {
    use crate::products::resource_balance::{TradeDirection, scan};
    let import = [(0x33, 21, 9001, 800_000)];
    let both = [(0x33, 21, 9001, 800_000), (0x44, 11, 31, 1_700_000)];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, true, 30_100_000, 9001, &import),
                resource_frame(101, false, 30_100_000, 9001, &import),
                // A value seen off a read is not a read: the hour carries hour 100's.
                resource_frame(102, false, 1, 9001, &import),
                resource_frame(103, true, 30_100_000, 9001, &both),
                resource_frame(104, true, 31_000_000, 9001, &both),
                // Hour 105 is missing: 106 carries nothing until the next read.
                resource_frame(106, false, 31_000_000, 9001, &both),
                resource_frame(107, true, 31_000_000, 9001, &both),
                // Another country took the tag: its carried rows are not Germany's.
                resource_frame(108, false, 31_000_000, 9002, &both),
                resource_frame(109, true, 31_000_000, 9002, &[]),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.resources,
        [(1, "steel".to_owned()), (2, "oil".to_owned())]
    );
    assert_eq!(scanned.reads, [100, 103, 104, 107, 109]);
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.produced_raw.as_str(),
            )
        })
        .collect();
    assert_eq!(
        steel,
        [
            (100, 103, true, "30100000"),
            (104, 104, false, "31000000"),
            (107, 107, true, "31000000"),
            (109, 109, true, "31000000"),
        ]
    );
    let run = &scanned.series[&1][0];
    assert_eq!(
        (
            run.available_raw.as_str(),
            run.after_projects_raw.as_str(),
            run.to_export_raw.as_str(),
            run.production_raw.as_str()
        ),
        ("28400000", "28360000", "2500000", "-3000000")
    );
    // Italy's rows are not Germany's.
    assert_eq!(scanned.series.len(), 2);
    let trades: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| {
            (
                trade.direction,
                trade.partner.as_str(),
                trade.first_hour,
                trade.last_hour,
                trade.starts_segment,
            )
        })
        .collect();
    assert_eq!(
        trades,
        [
            (TradeDirection::Import, "ITA", 100, 104, true),
            (TradeDirection::Export, "SPR", 103, 104, true),
            (TradeDirection::Import, "ITA", 107, 107, true),
            (TradeDirection::Export, "SPR", 107, 107, true),
        ]
    );
    assert_eq!(scanned.trades[0].delivered_raw, "800000");
    assert_eq!(scanned.unnamed + scanned.unresolved, 0);
}

#[test]
fn resource_balance_replies_without_content_and_answers_again_from_the_cache() {
    use crate::products::{
        CountryParams, LoadContext, Product,
        resource_balance::{RESOURCE_DECIMALS, ResourceBalance},
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    resource_frame(100, true, 30_100_000, 9001, &[]),
                    resource_frame(101, false, 30_100_000, 9001, &[]),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "resources".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let load = |params: &CountryParams| {
        ResourceBalance::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load(&params);
    assert_eq!(reply.decimals, RESOURCE_DECIMALS);
    // Without content, resources keep their tokens and have no names or icons.
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (
                resource.token.as_str(),
                resource.name.is_none(),
                resource.icon.is_none()
            ))
            .collect::<Vec<_>>(),
        [("steel", true, true), ("oil", true, true)]
    );
    assert_eq!(reply.series[0].runs[0].last_hour, 101);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Resource names and icons are unavailable"))
    );
    assert_eq!(load(&params).series, reply.series);
    let other = load(&CountryParams {
        tag: "XXX".into(),
        ..params
    });
    assert!(other.reads.is_empty());
    assert!(
        other
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
}

#[test]
fn resource_pools_and_deals_keep_their_own_values_when_the_other_table_is_missing() {
    use crate::products::resource_balance::scan;
    let deal = [(0x33, 21, 9001, 800_000)];
    let missing = |mut frame: DecodedFrame, table: TableId| {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, true, 30_100_000, 9001, &deal),
                // The deals were not captured at this read: the pools still count.
                missing(
                    resource_frame(101, true, 30_100_000, 9001, &deal),
                    TableId::ResourceExchanges,
                ),
                resource_frame(102, true, 30_100_000, 9001, &deal),
                // The pools were not captured at this read: the deals still count.
                missing(
                    resource_frame(103, true, 30_100_000, 9001, &deal),
                    TableId::CountryResources,
                ),
                resource_frame(104, true, 30_100_000, 9001, &deal),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [100, 101, 102, 104]);
    let spans = |spans: &[crate::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    assert_eq!(spans(&scanned.deal_hours), [(100, 100), (102, 104)]);
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
        .collect();
    assert_eq!(steel, [(100, 102, true), (104, 104, true)]);
    let deals: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| (trade.first_hour, trade.last_hour, trade.starts_segment))
        .collect();
    assert_eq!(deals, [(100, 100, true), (102, 104, true)]);
}

#[test]
fn deals_missing_at_a_carried_hour_are_unknown_from_there_while_the_pools_go_on() {
    use crate::products::resource_balance::scan;
    let deal = [(0x33, 21, 9001, 800_000)];
    let mut missing = resource_frame(101, false, 30_100_000, 9001, &deal);
    *missing.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, true, 30_100_000, 9001, &deal),
                // Carried hours: the deal table fails at 101 and is back at 102, but
                // only a read can say what the deals are again.
                missing,
                resource_frame(102, false, 30_100_000, 9001, &deal),
                resource_frame(103, true, 30_100_000, 9001, &deal),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    // The pools of read 100 carry through; their deals are known only at 100.
    assert_eq!(scanned.reads, [100, 103]);
    assert_eq!(
        scanned.series[&1]
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103)]
    );
    assert_eq!(
        scanned
            .deal_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
    assert_eq!(
        scanned
            .trades
            .iter()
            .map(|trade| (trade.first_hour, trade.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
}

#[test]
fn a_repeated_resource_token_is_its_own_resource_and_never_blanks_the_others() {
    use crate::products::{
        CountryParams, LoadContext, Product,
        resource_balance::{ResourceBalance, ordinals},
    };
    use replay::rows::{CountryResourceRow, ResourceDefinitionRow};
    // Resource 3 repeats resource 1's token, as a second declaration in content would.
    let repeated = |hour| {
        let mut frame = resource_frame(hour, true, 30_100_000, 9001, &[]);
        if let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
            &mut *frame.tables[TableId::ResourceDefinitions.index()]
        {
            rows.push(ResourceDefinitionRow {
                id: 3,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            });
        }
        if let TableView::Complete(TableRows::CountryResources(rows)) =
            &mut *frame.tables[TableId::CountryResources.index()]
        {
            rows.insert(
                2,
                CountryResourceRow {
                    country: 11,
                    resource: 3,
                    produced: 700_000,
                    ..Default::default()
                },
            );
        }
        frame
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![repeated(100), repeated(101)])],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "repeated".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ResourceBalance::load(
        &source,
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (resource.id, resource.token.as_str(), resource.ordinal))
            .collect::<Vec<_>>(),
        [(1, "steel", 1), (2, "oil", 1), (3, "steel", 2)]
    );
    // Each keeps its own history; none overlaps another's.
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| (
                series.id,
                series.runs.len(),
                series.runs[0].produced_raw.as_str()
            ))
            .collect::<Vec<_>>(),
        [(1, 1, "30100000"), (2, 1, "0"), (3, 1, "700000")]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("steel is declared 2 times"))
    );
    assert_eq!(
        ordinals(&[
            (1, "a".into()),
            (2, "b".into()),
            (5, "a".into()),
            (9, "a".into())
        ]),
        [1, 1, 2, 3]
    );
}

/// A frame with Germany's fuel status (native id `germany`, linked through id 11
/// too) and Italy's, each owning a state. `read` times a walk of another table of
/// the fuel slot only, as a shared read times only its anchor. Fuel values are
/// (current, maximum, ring gain, ring consumed) raw.
fn fuel_frame(hour: i32, read: bool, germany: u32, fuel: (i64, i64, i64, i64)) -> DecodedFrame {
    use replay::rows::{CountryFuelRow, StateRow};
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(germany, 7),
        ]));
    let (current_raw, maximum_raw, ring_gain_raw, ring_consumed_raw) = fuel;
    *frame.tables[TableId::CountryFuel.index()] =
        TableView::Complete(TableRows::CountryFuel(vec![
            // Germany's row names it by its other link.
            CountryFuelRow {
                country: 11,
                current_raw,
                maximum_raw,
                ring_gain_raw,
                ring_consumed_raw,
                ring_other_raw: 3,
                lend_lease_gain_raw: 7,
                ..Default::default()
            },
            CountryFuelRow {
                country: 21,
                current_raw: 1,
                maximum_raw: 2,
                ring_gain_raw: 3,
                ring_consumed_raw: 4,
                ..Default::default()
            },
        ]));
    let state = |id, owner| StateRow {
        id,
        owner,
        controller: owner,
        ..Default::default()
    };
    // Germany's state names it by its other link.
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![state(1, 21), state(2, 11)]));
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    if read {
        frame.metrics.table_ns[TableId::CountryFuelConsumers.index()] = 1;
    }
    frame
}

/// The frame with Germany's links gone: Countries lists it, but CountryLinks
/// (otherwise complete) cannot resolve it.
fn unlinked(mut frame: DecodedFrame) -> DecodedFrame {
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        links.retain(|link| link.canonical_slot != 7);
    }
    frame
}

/// The frame with Germany's state taken by Italy, and another state owned by a
/// link id the links cannot resolve.
fn stateless_with_unknown_owner(frame: DecodedFrame) -> DecodedFrame {
    use replay::rows::StateRow;
    let mut frame = stateless(frame);
    if let TableView::Complete(TableRows::States(states)) =
        &mut *frame.tables[TableId::States.index()]
    {
        states.push(StateRow {
            id: 3,
            owner: 99,
            ..Default::default()
        });
    }
    frame
}

/// The frame with Germany's state taken by Italy.
fn stateless(mut frame: DecodedFrame) -> DecodedFrame {
    use replay::rows::StateRow;
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 21,
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 21,
            ..Default::default()
        },
    ]));
    frame
}

/// The frame recorded without a States table, so it cannot tell whether Germany
/// owned a state.
fn unowned_unknown(mut frame: DecodedFrame) -> DecodedFrame {
    *frame.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    frame
}

/// The fuel slot's hour on day `day`.
fn fuel_slot(day: i32) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(TableId::CountryFuel)
    else {
        panic!("CountryFuel is read in a daily slot");
    };
    day * replay::cadence::STAGGER_PERIOD + slot as i32
}

/// What a fuel test hour holds, relative to the fuel slot: a read, a sample
/// without a state or one recorded without States.
#[derive(Clone, Copy, PartialEq)]
enum FuelHour {
    Owning,
    Stateless,
    Unknown,
}

/// A frame for `hour` (relative to `slot`) as `kind` says.
fn fuel_hour(
    slot: i32,
    hour: i32,
    read: bool,
    germany: u32,
    fuel: (i64, i64, i64, i64),
    kind: FuelHour,
) -> DecodedFrame {
    let frame = fuel_frame(slot + hour, read, germany, fuel);
    match kind {
        FuelHour::Owning => frame,
        FuelHour::Stateless => stateless(frame),
        FuelHour::Unknown => unowned_unknown(frame),
    }
}

#[test]
fn fuel_reads_carry_until_the_next_keep_whole_days_once_and_never_cross_gaps() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    // Every hour from -25 to 336 is recorded but 25 and 311. Reads (relative to
    // the fuel slot): a branch start at -25, the slot at 0, a refresh at 2, then
    // daily slot reads, an off-slot read at 127 (and no slot reads at 120 or 144).
    let reads = [
        -25, 0, 2, 24, 48, 72, 96, 127, 168, 192, 216, 240, 264, 288, 312, 336,
    ];
    let missing = [25, 311];
    let kind = |hour| match hour {
        150 | 192 | 255 => FuelHour::Stateless,
        230 | 250 => FuelHour::Unknown,
        _ => FuelHour::Owning,
    };
    let fuel = |hour| match hour {
        ..0 => (32_768, 655_360_000, 100_000, 0),
        0 => (65_536, 655_360_000, 200_000, 32_768),
        // A value seen off a read is not a read: hour 1 carries the slot's read.
        1 => (1, 1, 1, 1),
        2..24 => (98_304, 655_360_000, 300_000, 0),
        _ => full,
    };
    // Another country takes the tag from hour 49.
    let country = |hour| if hour < 49 { 9001 } else { 9002 };
    let frames = (-25..=336)
        .filter(|hour| !missing.contains(hour))
        .map(|hour| {
            fuel_hour(
                slot,
                hour,
                reads.contains(&hour),
                country(hour),
                fuel(hour),
                kind(hour),
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reads: Vec<_> = scanned
        .reads
        .iter()
        .map(|read| {
            (
                read.first_hour - slot,
                read.last_hour - slot,
                read.starts_segment,
                read.fuel_raw.as_str(),
                read.day.is_some(),
                read.left_out,
            )
        })
        .collect();
    let full = "655360000";
    let (unknown, stale, overlaps) = (
        Some(DayLeftOut::OwnershipNotRecorded),
        Some(DayLeftOut::NotUpdated),
        Some(DayLeftOut::Overlaps),
    );
    assert_eq!(
        reads,
        [
            // The branch's first read: its day lies before the recording.
            (-25, -1, true, "32768", false, unknown),
            (0, 1, false, "65536", true, None),
            // A refresh two hours after the slot read: its day overlaps the slot read's.
            (2, 23, false, "98304", false, overlaps),
            // Hour 25 is missing: the read carries nothing past it.
            (24, 24, false, full, true, None),
            // The gap at 25 lies inside this read's day.
            (48, 48, true, full, false, unknown),
            // The first update of this day followed a sample of the old country.
            (72, 95, true, full, false, stale),
            (96, 126, false, full, true, None),
            // Off the slot, a day and more from every other read, it keeps its day.
            (127, 167, false, full, true, None),
            // Without a state at 150 the game did not update the ring.
            (168, 191, false, full, false, stale),
            // The read's own sample (without a state) governs the next update.
            (192, 215, false, full, true, None),
            // ...which is this day's first.
            (216, 239, false, full, false, stale),
            // An hour recorded without States cannot tell.
            (240, 263, false, full, false, unknown),
            // An hour without a state outweighs one that cannot tell.
            (264, 287, false, full, false, stale),
            (288, 310, false, full, true, None),
            // A gap at 311, the last sample of this day.
            (312, 335, true, full, false, unknown),
            // The same gap lies just outside this day.
            (336, 336, false, full, true, None),
        ]
    );
    assert_eq!(
        scanned.reads[3]
            .day
            .as_ref()
            .map(|day| (day.gained_raw.as_str(), day.burned_raw.as_str())),
        Some(("1000000", "65536"))
    );
    assert_eq!(scanned.reads[3].capacity_raw, "655360000");
    // The latest hour's lend-lease fuel, which the day's gain does not include.
    assert_eq!(scanned.reads[3].lend_lease_raw, "7");
}

#[test]
fn a_fuel_day_is_governed_by_all_24_samples_before_its_read() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    // The day read at 24 (relative to the slot), recorded from `from` with the
    // hours in `missing` left out and those in `stateless` holding no state.
    let day_read_at_24 = |from: i32, missing: &[i32], stateless: &[i32]| {
        let frames = (from..=24)
            .filter(|hour| !missing.contains(hour))
            .map(|hour| {
                let kind = if stateless.contains(&hour) {
                    FuelHour::Stateless
                } else {
                    FuelHour::Owning
                };
                fuel_hour(slot, hour, hour == 0 || hour == 24, 9001, full, kind)
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
        scanned.reads.last().unwrap().left_out
    };
    assert_eq!(day_read_at_24(0, &[], &[]), None);
    // The sample 24 hours before a read (without a state) governs the first
    // update the read holds, although the country owns one again an hour later.
    assert_eq!(day_read_at_24(0, &[], &[0]), Some(DayLeftOut::NotUpdated));
    // The read's own sample (without a state) governs the next update.
    assert_eq!(day_read_at_24(0, &[], &[24]), None);
    // Nor does a sample 25 hours before it.
    assert_eq!(day_read_at_24(-1, &[], &[-1]), None);
    // A gap inside the day: the country may have lost its states then, unseen.
    assert_eq!(
        day_read_at_24(0, &[12], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    assert_eq!(
        day_read_at_24(0, &[0], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // A gap just outside it (25 hours back) does not count.
    assert_eq!(day_read_at_24(-3, &[-1], &[]), None);
    // A sample without a state outweighs a gap.
    assert_eq!(day_read_at_24(0, &[12], &[5]), Some(DayLeftOut::NotUpdated));
    // With none of the 24 recorded (a branch's first read) there is no evidence.
    assert_eq!(
        day_read_at_24(24, &[], &[]),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
}

#[test]
fn a_fuel_day_is_not_known_where_the_links_cannot_tell_who_owns_what() {
    use crate::products::country_fuel::{DayLeftOut, scan};
    let slot = fuel_slot(1000);
    let full = (655_360_000, 655_360_000, 1_000_000, 65_536);
    let day_read_at_24 = |odd: fn(DecodedFrame) -> DecodedFrame| {
        let frames = (0..=24)
            .map(|hour| {
                let frame = fuel_frame(slot + hour, hour == 0 || hour == 24, 9001, full);
                if hour == 12 { odd(frame) } else { frame }
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
        scanned.reads.last().unwrap().left_out
    };
    // Countries lists the tag but its links are gone: whose it is, and whether it
    // owned a state, is unknown, not "no state".
    assert_eq!(
        day_read_at_24(unlinked),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // No state resolves to the country, but one names an owner the links cannot
    // resolve: that one could be its own.
    assert_eq!(
        day_read_at_24(stateless_with_unknown_owner),
        Some(DayLeftOut::OwnershipNotRecorded)
    );
    // Every owner resolves and none is the country's: it held no state.
    assert_eq!(day_read_at_24(stateless), Some(DayLeftOut::NotUpdated));
}

#[test]
fn a_tag_is_found_absent_or_unresolved() {
    use crate::metadata::{TaggedCountry, tagged_country};
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let countries = [country(21, 8, b"ITA"), country(9001, 7, b"GER")];
    let links = [
        CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ];
    assert_eq!(
        tagged_country(&countries, &links, "GER"),
        TaggedCountry::Found(&countries[1], 7)
    );
    assert_eq!(
        tagged_country(&countries, &links, "FRA"),
        TaggedCountry::Absent
    );
    assert_eq!(
        tagged_country(&countries, &links[..1], "GER"),
        TaggedCountry::Unresolved
    );
    assert_eq!(tagged_country(&countries, &links[..1], "GER").found(), None);
}

#[test]
fn fuel_replies_with_the_engine_scales_and_answers_again_from_the_cache() {
    use crate::products::{CountryParams, LoadContext, Product, country_fuel::CountryFuel};
    let slot = fuel_slot(1000);
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                // A whole recorded day before the slot read, then a read an hour later.
                (slot - 24..=slot + 1)
                    .map(|hour| {
                        fuel_frame(hour, hour >= slot, 9001, (65_536, 98_304, 200_000, 32_768))
                    })
                    .collect(),
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "fuel".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |tag: &str| {
        CountryFuel::load(
            &source,
            &CountryParams {
                branch_id: "1".into(),
                tag: tag.into(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load("GER");
    assert_eq!(
        (reply.quantity_scale, reply.gain_scale, reply.day_hours),
        (32_768, 100_000, 24)
    );
    assert_eq!(reply.reads.len(), 2);
    assert!(reply.reads[1].day.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Fuel reads less than a day from another: 1.")),
        "{:?}",
        reply.warnings
    );
    assert_eq!(load("GER").reads, reply.reads);
    let missing = load("XXX");
    assert!(missing.reads.is_empty());
    assert!(missing.warnings[0].starts_with("XXX:"));
}
