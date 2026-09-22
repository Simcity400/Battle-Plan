use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use replay::{
    branches::{BranchReason, BranchStart, Gap, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryRow, TableId, TableRows},
    spool::Record,
};

use crate::{
    Analytics, CountrySeriesRequest, MAX_POINTS, PointStatus,
    reduce::{Reducer, Values},
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
        frame.tables[table.index()] = TableView::Unavailable {
            reason: "independent table failure".into(),
        };
    }
    let histories =
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false)).unwrap();
    let slot = &histories["GER"].slots[0];
    assert_eq!(slot.entries[0].token, "tech_1");
    assert!(slot.states[0].details.as_ref().unwrap().bonuses.is_none());
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
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
    frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "fixture hourly research".into(),
    };
    frame.tables[TableId::ResearchSlots.index()] = TableView::Complete(TableRows::ResearchSlots(
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
    frame.tables[TableId::ResearchTechnologies.index()] =
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
    frame.tables[TableId::TechnologyDefinitions.index()] =
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
            &mut frame.tables[TableId::ResearchSlots.index()]
        else {
            panic!()
        };
        slots[0].research_speed_raw = if frame.hour == 102 { 200_000 } else { 150_000 };
        slots[0].applied_days_raw = 3_000_000;
        slots[0].banked_days_raw = i64::MAX;
        let TableView::Complete(TableRows::ResearchTechnologies(technologies)) =
            &mut frame.tables[TableId::ResearchTechnologies.index()]
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

#[test]
fn research_never_dates_through_gaps_or_country_reuse_and_cache_is_branch_scoped() {
    let mut missing = research_frame(1, 101, &[Some(1)], &[]);
    missing.tables[TableId::ResearchSlots.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let mut reused = research_frame(1, 104, &[Some(2)], &[1]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut reused.tables[TableId::Countries.index()]
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
        &mut frame.tables[TableId::Countries.index()]
    {
        let mut second = rows[0].clone();
        second.id = 9002;
        second.canonical_slot = 8;
        second.tag = b"ENG".to_vec();
        rows.push(second);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 9002,
            canonical_slot: 8,
        });
    }
    if let TableView::Complete(TableRows::ResearchInputs(rows)) =
        &mut frame.tables[TableId::ResearchInputs.index()]
    {
        let mut second = rows[0].clone();
        second.country = 9002;
        second.technology_handle = 2;
        second.token = b"tech_2".to_vec();
        rows.push(second);
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
        frame.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic unavailable".into(),
        };
        assert!(
            crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
                .unwrap()
                .is_empty()
        );
    }
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(Vec::new()));
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut frame.tables[TableId::CountryLinks.index()]
    {
        links.retain(|link| link.id != 11);
    }
    assert!(
        crate::research_branch(&research_replay(vec![frame]), 1, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let mut frame = research_frame(1, 100, &[Some(1)], &[]);
    frame.tables[TableId::ResearchTechnologies.index()] = TableView::Truncated {
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
        &mut first.tables[TableId::ResearchTechnologies.index()]
    {
        rows[0].classifier = engine::ctechnology::CLASSIFIER_ACTIVE;
    }
    let mut second = first.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut second.tables[TableId::ResearchTechnologies.index()]
    {
        rows[0].level = 2;
    }
    let mut third = second.clone();
    third.hour = 102;
    if let TableView::Complete(TableRows::ResearchSlots(rows)) =
        &mut third.tables[TableId::ResearchSlots.index()]
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
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
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
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [
            (11, german_slot),
            (44, 9),
            (german_id, german_slot),
            (10001, 9),
        ]
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .to_vec(),
    ));
    frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        [(11, german_tokens), (44, france)]
            .map(|(country, tokens)| FocusRow {
                country,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            })
            .to_vec(),
    ));
    frame.tables[TableId::CompletedFocuses.index()] =
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
    unavailable.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
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
    let projection = crate::focus_branch(&replay, 1, &cancel).unwrap();
    assert_eq!(projection.countries.len(), 2);
    let mut cache = crate::FocusCache::default();
    for tag in ["GER", "FRA", "GER", "FRA"] {
        let history = cache.get(&replay, 1, tag, &cancel).unwrap();
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
        cache.get(&replay, 1, "GER", &cancel).unwrap(),
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
        frame.tables[TableId::Focus.index()] =
            TableView::Complete(TableRows::Focus(vec![FocusRow {
                country: 11,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            }]));
        frame.tables[TableId::CompletedFocuses.index()] =
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
    frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![FocusRow {
        country: 11,
        current: Some(b"next".to_vec()),
        completed_count: 1,
        ..FocusRow::default()
    }]));
    frame.tables[TableId::CompletedFocuses.index()] =
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
    frame.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    assert!(crate::focus_order(&read(vec![frame]), 1, 100, "GER", &cancel).is_err());
}

fn ownership_frame(branch: u64, hour: i32, owner: u32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut frame.tables[TableId::Countries.index()]
    {
        for (id, slot, tag) in [(44, 8, b"FRA"), (55, 9, b"CTL"), (66, 10, b"MOD")] {
            rows.push(CountryRow {
                id,
                canonical_slot: slot,
                tag: tag.to_vec(),
                name: tag.to_vec(),
                ..CountryRow::default()
            });
        }
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend(
            [(44, 8), (55, 9), (66, 10)]
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }),
        );
        rows.sort_by_key(|row| row.id);
    }
    frame.tables[TableId::States.index()] =
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
    let result = crate::land_owners(&replay, &AtomicBool::new(false)).unwrap();
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
fn land_owners_report_dangling_owners_and_missing_country_identity() {
    for missing in ["link", "row", "tag"] {
        let mut frame = ownership_frame(1, 101, 44);
        match missing {
            "link" => {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut frame.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            "row" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            _ => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut frame.tables[TableId::Countries.index()]
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
        let result = crate::land_owners(&replay, &AtomicBool::new(false)).unwrap();
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
    let result = crate::land_owners(&replay, &AtomicBool::new(false)).unwrap();
    assert!(result.countries.is_empty());
    assert!(result.warnings.is_empty());
}

#[test]
fn land_owners_keep_partial_positive_evidence_and_do_not_reuse_lost_baselines() {
    let mut partial = ownership_frame(1, 102, 66);
    partial.tables[TableId::States.index()] = TableView::Truncated {
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
    let result = crate::land_owners(&replay, &AtomicBool::new(false)).unwrap();
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
        crate::land_owners(&replay, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    replay.index[0].raw_length = crate::QueryLimits::default().max_block_raw_bytes as u64 + 1;
    assert!(
        crate::land_owners(&replay, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("memory budget")
    );
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
            name: b"Germany".to_vec(),
            political_power_raw: power,
            command_power_raw: i64::MAX,
            stability_raw: i64::MIN,
            war_support_raw: 70_001,
            convoys_raw: Some(99_001),
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
        tables,
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
                if matches!(&frame.tables[TableId::ResearchInputs.index()], TableView::Unavailable { reason } if reason == "fixture hourly research") {
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
    frame.tables[TableId::ResearchInputs.index()] = input.map_or_else(
        || TableView::Unavailable {
            reason: "fixture research unavailable".into(),
        },
        |rows| TableView::Complete(TableRows::ResearchInputs(rows)),
    );
    frame.tables[TableId::ResearchAppliedBonuses.index()] =
        TableView::Complete(TableRows::ResearchAppliedBonuses(vec![]));
}

fn open(segments: Vec<Vec<Record>>) -> Analytics {
    Analytics::from_replay(
        Replay::from_records(CAMPAIGN.into(), &segments).unwrap(),
        PathBuf::from("memory.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn request(branch: u64, max_points: Option<u32>) -> CountrySeriesRequest {
    CountrySeriesRequest {
        branch_id: branch.to_string(),
        country_id: "7".into(),
        max_points,
        first_hour: None,
        last_hour: None,
    }
}

#[test]
fn canonical_country_aliases_and_full_width_native_values_survive_json() {
    let branch = u64::MAX;
    let mut analytics = open(vec![records(
        branch,
        vec![source(branch, 100, Some(i64::MAX))],
    )]);
    let result = analytics
        .country_series(&request(branch, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.branch_id, "18446744073709551615");
    assert_eq!(result.session_id, result.branch_id);
    assert_eq!(result.country.as_ref().unwrap().id, "7");
    assert_eq!(analytics.metadata().countries[0].id, "7");
    let values = &result.points[0].first.as_ref().unwrap().values;
    assert_eq!(
        values.political_power.as_deref(),
        Some("9223372036854775807")
    );
    assert_eq!(values.stability.as_deref(), Some("-9223372036854775808"));
    assert_eq!(values.convoys.as_deref(), Some("99001"));
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"politicalPower\":\"9223372036854775807\""));
    assert_eq!(
        serde_json::from_str::<crate::CountrySeries>(&json).unwrap(),
        result
    );
    let mut raw_reference = request(branch, None);
    raw_reference.country_id = "9001".into();
    let absent = analytics
        .country_series(&raw_reference, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(absent.points[0].status, PointStatus::Absent);
}

#[test]
fn gaps_partial_tables_absence_and_optional_nulls_are_distinct() {
    let mut partial = source(1, 103, Some(3));
    partial.tables[TableId::Countries.index()] = TableView::Truncated {
        rows: TableRows::Countries(vec![]),
        missing: 1,
    };
    let mut absent = source(1, 104, Some(4));
    absent.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![]));
    let mut input = records(
        1,
        vec![
            source(1, 100, Some(1)),
            source(1, 101, None),
            partial,
            absent,
            source(1, 105, Some(5)),
        ],
    );
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 102,
        last_hour: 102,
        reason: "queue full".into(),
    }));
    let mut analytics = open(vec![input]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        result
            .points
            .iter()
            .map(|point| point.status)
            .collect::<Vec<_>>(),
        vec![
            PointStatus::Recorded,
            PointStatus::Recorded,
            PointStatus::Gap,
            PointStatus::Unavailable,
            PointStatus::Absent,
            PointStatus::Recorded,
        ]
    );
    assert_eq!(result.coverage.recorded_hours, 3);
    assert_eq!(result.coverage.gap_hours, 1);
    assert_eq!(result.coverage.unavailable_hours, 1);
    assert_eq!(result.coverage.absent_hours, 1);
    assert_eq!(result.points[1].missing_metric_hours.political_power, 1);
    assert!(
        result.points[1]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .is_none()
    );
    assert!(
        result.points[2]
            .reason
            .as_ref()
            .unwrap()
            .contains("queue full")
    );
    assert!(
        result.points[3]
            .reason
            .as_ref()
            .unwrap()
            .contains("truncated")
    );
}

#[test]
fn downsampling_keeps_exact_signed_extrema_and_every_loss_count() {
    let mut reducer = Reducer::new(0, 19_999, MAX_POINTS);
    for hour in 0..20_000 {
        if hour % 3 == 1 {
            reducer.missing(hour, hour, PointStatus::Gap, "lost hour");
        } else {
            let value = if hour == 777 {
                i64::MIN
            } else if hour == 9000 {
                i64::MAX
            } else {
                i64::from(hour)
            };
            reducer.record(
                hour,
                Values([Some(value), None, Some(value), Some(0), None]),
            );
        }
    }
    let (points, coverage) = reducer.finish();
    assert!(points.len() <= MAX_POINTS as usize);
    assert_eq!(coverage.recorded_hours + coverage.gap_hours, 20_000);
    assert_eq!(coverage.gap_hours, 6667);
    assert!(
        points
            .iter()
            .any(|point| point.min.political_power.as_deref() == Some("-9223372036854775808"))
    );
    assert!(
        points
            .iter()
            .any(|point| point.max.political_power.as_deref() == Some("9223372036854775807"))
    );
    assert!(
        points
            .iter()
            .all(|point| point.status == PointStatus::Mixed)
    );
    assert!(
        points
            .iter()
            .all(|point| point.missing_metric_hours.command_power == point.recorded_hours)
    );
}

#[test]
fn a_full_signed_hour_range_is_bounded_without_per_hour_expansion() {
    let mut reducer = Reducer::new(i32::MIN, i32::MAX, 1);
    reducer.missing(i32::MIN, i32::MAX, PointStatus::Gap, "no data");
    let (points, coverage) = reducer.finish();
    assert_eq!(points.len(), 1);
    assert_eq!(coverage.gap_hours, 4_294_967_296);
    assert_eq!(points[0].status, PointStatus::Gap);
    assert!(points[0].first.is_none());
}

#[test]
fn branch_requests_never_join_overlapping_dates_or_probable_parents() {
    let first = records(1, vec![source(1, 100, Some(10)), source(1, 101, Some(11))]);
    let mut second = records(2, vec![source(2, 101, Some(20)), source(2, 102, Some(21))]);
    if let Record::BranchStart(branch) = &mut second[1] {
        branch.loaded_hour = Some(100);
        branch.parent = Parentage::Probable {
            branch: 1,
            hour: 100,
        };
    }
    let mut analytics = open(vec![first, second]);
    assert_eq!(
        analytics.metadata().branches[1].parent_id.as_deref(),
        Some("1")
    );
    let result = analytics
        .country_series(&request(2, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!((result.first_hour, result.last_hour), (101, 102));
    assert_eq!(
        result.points[0]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("20")
    );
    assert_eq!(result.coverage.recorded_hours, 2);
}

#[test]
fn recovered_frames_override_overlapping_loss_diagnostics() {
    let mut input = records(1, vec![source(1, 100, Some(1)), source(1, 101, Some(2))]);
    if let Record::BranchStart(branch) = &mut input[1] {
        branch.first_hour = 98;
    }
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 98,
        last_hour: 103,
        reason: "failed flush".into(),
    }));
    let mut analytics = open(vec![input]);
    assert_eq!(analytics.metadata().branches[0].first_hour, 98);
    assert_eq!(analytics.metadata().branches[0].last_hour, 103);
    assert_eq!(
        analytics.metadata().branches[0].gaps,
        vec![crate::RecordingGap {
            first_hour: 98,
            last_hour: 103,
            reason: "failed flush".into(),
        }]
    );
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.recorded_hours, 2);
    assert_eq!(result.coverage.gap_hours, 4);
    assert_eq!(result.points[2].status, PointStatus::Recorded);
    assert_eq!(result.points[3].status, PointStatus::Recorded);
}

#[test]
fn unreadable_block_is_a_gap_and_the_next_keyframe_recovers() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![source(1, 100, Some(1)), source(1, 820, Some(2))],
        )],
    )
    .unwrap();
    let block = &replay.index[0];
    let mut bytes = replay.bytes().to_vec();
    bytes[(block.offset + block.length - 1) as usize] ^= 0x40;
    let mut analytics = Analytics::from_replay(
        Replay::open(bytes).unwrap(),
        "memory.battleplan".into(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 720);
    assert_eq!(result.coverage.recorded_hours, 1);
    assert_eq!(result.points[0].status, PointStatus::Gap);
    assert!(
        result.points[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("unreadable block")
    );
    assert_eq!(result.points.last().unwrap().status, PointStatus::Recorded);
}

#[test]
fn a_campaign_mismatch_cannot_lend_country_values_to_a_chart() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![source(1, 100, Some(1)), source(1, 101, Some(2))],
        )],
    )
    .unwrap();
    replay.campaign_id = b"different campaign".to_vec();
    let mut analytics =
        Analytics::from_replay(replay, "memory.battleplan".into(), &AtomicBool::new(false))
            .unwrap();
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 2);
    assert!(result.points.iter().all(|point| point.first.is_none()));
    assert!(
        result.points[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("campaign identity")
    );
}

#[test]
fn disagreeing_country_links_are_unavailable_rather_than_absent_or_guessed() {
    let mut frame = source(1, 100, Some(42));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut frame.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 8;
    }
    let mut analytics = open(vec![records(1, vec![frame])]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.points[0].status, PointStatus::Unavailable);
    assert!(result.points[0].first.is_none());
}

#[test]
fn cancelled_queries_do_not_replace_the_last_completed_cache() {
    let mut analytics = open(vec![records(1, vec![source(1, 100, Some(42))])]);
    let cancel = AtomicBool::new(false);
    let first = analytics
        .country_series(&request(1, None), &cancel)
        .unwrap();
    assert_eq!(analytics.scans, 1);
    let repeat = analytics
        .country_series(&request(1, Some(MAX_POINTS)), &cancel)
        .unwrap();
    assert_eq!(first, repeat);
    assert_eq!(analytics.scans, 1);
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        analytics
            .country_series(&request(1, Some(1)), &cancel)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(
        analytics
            .country_series(&request(1, None), &cancel)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(
        analytics
            .country_series(&request(1, None), &cancel)
            .unwrap(),
        first
    );
    assert_eq!(analytics.scans, 1);
}

#[test]
fn invalid_identifiers_and_point_budgets_fail_before_scanning() {
    let mut analytics = open(vec![records(1, vec![source(1, 100, Some(42))])]);
    let cancel = AtomicBool::new(false);
    for branch in ["01", "1 ", "+1", "-1", "18446744073709551616", "999"] {
        let mut input = request(1, None);
        input.branch_id = branch.into();
        assert!(analytics.country_series(&input, &cancel).is_err());
    }
    for count in [0, 1201, u32::MAX] {
        assert!(
            analytics
                .country_series(&request(1, Some(count)), &cancel)
                .is_err()
        );
    }
    assert_eq!(analytics.scans, 0);
}

#[test]
fn a_lost_country_baseline_never_reuses_stale_values() {
    let mut input = records(1, vec![source(1, 100, Some(1)), source(1, 101, Some(2))]);
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::Countries.index()] = TableUpdate::Unchanged { baseline: 99 };
    }
    let mut analytics = open(vec![input]);
    let result = analytics
        .country_series(&request(1, None), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.points[0].status, PointStatus::Recorded);
    assert_eq!(result.points[1].status, PointStatus::Unavailable);
    assert!(result.points[1].first.is_none());
}

#[test]
fn an_initial_failed_census_is_not_a_complete_empty_country_catalogue() {
    let mut failed = source(1, 100, Some(1));
    failed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "initial capacity".into(),
    };
    let analytics = open(vec![records(1, vec![failed, source(1, 101, Some(2))])]);
    assert_eq!(analytics.metadata().countries[0].observed_hour, 101);
}

#[test]
fn ranged_queries_decode_prefixes_but_return_only_requested_hours() {
    let mut analytics = open(vec![records(
        1,
        vec![
            source(1, 100, Some(10)),
            source(1, 103, Some(30)),
            source(1, 104, Some(40)),
        ],
    )]);
    let mut range = request(1, None);
    range.first_hour = Some(101);
    range.last_hour = Some(103);
    let result = analytics
        .country_series(&range, &AtomicBool::new(false))
        .unwrap();
    assert_eq!((result.first_hour, result.last_hour), (101, 103));
    assert_eq!(result.coverage.gap_hours, 2);
    assert_eq!(result.coverage.recorded_hours, 1);
    assert_eq!(
        result
            .points
            .last()
            .unwrap()
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("30")
    );
    range.last_hour = Some(102);
    let result = analytics
        .country_series(&range, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.coverage.gap_hours, 2);
    assert_eq!(result.coverage.recorded_hours, 0);
    assert_eq!(analytics.scans, 2);
    range.first_hour = Some(99);
    assert!(
        analytics
            .country_series(&range, &AtomicBool::new(false))
            .is_err()
    );
    range.first_hour = Some(104);
    assert!(
        analytics
            .country_series(&range, &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(analytics.scans, 2);
}

fn dashboard_request(first: i32, last: i32) -> crate::QueryRequest {
    crate::QueryRequest {
        scope_id: "current".into(),
        generation: "1".into(),
        source_id: "source".into(),
        revision: "snapshot".into(),
        branch_id: "1".into(),
        country_ids: vec!["7".into()],
        first_hour: first,
        last_hour: last,
        kind: crate::QueryKind::ExactWindow,
        max_points: None,
        priority: crate::QueryPriority::Current,
    }
}

fn query_replay(frames: Vec<DecodedFrame>) -> std::sync::Arc<Replay> {
    std::sync::Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap())
}

fn finish_scan(scan: &mut crate::CountryScan) -> crate::QueryResult {
    loop {
        match scan
            .advance(
                std::num::NonZeroUsize::new(1).unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap()
        {
            crate::ScanStep::Yielded => {}
            crate::ScanStep::Ready(result) => return result,
        }
    }
}

#[test]
fn projected_chunks_reuse_decoding_across_countries_ranges_and_plot_density() {
    let replay = query_replay(vec![
        source(1, 100, Some(10)),
        source(1, 102, Some(90)),
        source(1, 103, Some(20)),
    ]);
    let runtime = crate::AnalyticsRuntime::new(crate::RuntimeLimits::default()).unwrap();
    let mut request = dashboard_request(100, 103);
    let source = runtime
        .register_source(
            request.source_key(),
            "synthetic.battleplan".into(),
            std::sync::Arc::clone(&replay),
        )
        .unwrap();
    request.kind = crate::QueryKind::History;
    request.max_points = Some(1);
    let history = runtime.request(request.clone()).unwrap().recv().unwrap();
    assert_eq!(
        history.series[0].points[0].max.political_power.as_deref(),
        Some("90")
    );
    assert_eq!(runtime.stats().block_reads, 1);

    request = dashboard_request(100, 101);
    request.country_ids.push("99".into());
    let exact = runtime.request(request.clone()).unwrap().recv().unwrap();
    let expected = finish_scan(
        &mut crate::CountryScan::start(
            std::sync::Arc::clone(&replay),
            &request,
            crate::QueryLimits::default(),
        )
        .unwrap(),
    );
    assert_eq!(*exact, expected);
    assert_eq!(
        exact.series[0].points[0].max.political_power.as_deref(),
        Some("10")
    );
    assert_eq!(exact.series[0].points[1].status, PointStatus::Gap);
    assert_eq!(exact.series[1].points[0].status, PointStatus::Absent);
    assert_eq!(runtime.stats().block_reads, 1);
    assert_eq!(runtime.stats().projection_hits, 1);
    assert!(runtime.stats().projection_bytes > 0);
    runtime.close_source(source.key());
    assert_eq!(runtime.stats().projection_bytes, 0);
}

#[test]
fn projected_chunks_preserve_unavailable_tables_and_never_cross_source_revisions() {
    let first = source(1, 100, Some(0));
    let mut failed = source(1, 101, Some(42));
    failed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic table failure".into(),
    };
    let replay = query_replay(vec![first, failed, source(1, 102, None)]);
    let runtime = crate::AnalyticsRuntime::new(crate::RuntimeLimits::default()).unwrap();
    let mut request = dashboard_request(100, 102);
    let original = runtime
        .register_source(
            request.source_key(),
            "one.battleplan".into(),
            std::sync::Arc::clone(&replay),
        )
        .unwrap();
    runtime.request(request.clone()).unwrap().recv().unwrap();
    request.first_hour = 101;
    let cached = runtime.request(request.clone()).unwrap().recv().unwrap();
    let cold = finish_scan(
        &mut crate::CountryScan::start(
            std::sync::Arc::clone(&replay),
            &request,
            crate::QueryLimits::default(),
        )
        .unwrap(),
    );
    assert_eq!(*cached, cold);
    assert_eq!(cached.series[0].points[0].status, PointStatus::Unavailable);
    assert_eq!(runtime.stats().block_reads, 1);
    request.revision = "replacement".into();
    let replacement = runtime
        .register_source(request.source_key(), "two.battleplan".into(), replay)
        .unwrap();
    runtime.request(request).unwrap().recv().unwrap();
    assert_eq!(runtime.stats().block_reads, 2);
    drop((original, replacement));
}

#[test]
fn projection_pressure_falls_back_without_changing_exact_results() {
    let replay = query_replay(vec![source(1, 100, Some(i64::MIN)), source(1, 101, None)]);
    let limits = crate::RuntimeLimits {
        projection_bytes: 1,
        ..Default::default()
    };
    let runtime = crate::AnalyticsRuntime::new(limits).unwrap();
    let request = dashboard_request(100, 101);
    let _source = runtime
        .register_source(
            request.source_key(),
            "synthetic.battleplan".into(),
            std::sync::Arc::clone(&replay),
        )
        .unwrap();
    let result = runtime.request(request.clone()).unwrap().recv().unwrap();
    let cold = finish_scan(
        &mut crate::CountryScan::start(replay, &request, crate::QueryLimits::default()).unwrap(),
    );
    assert_eq!(*result, cold);
    assert_eq!(runtime.stats().projection_bytes, 0);
}

#[test]
fn evicting_a_projection_does_not_release_a_scans_live_byte_charge() {
    use crate::projection::{ProjectedBlock, ProjectionCache};
    let cache = ProjectionCache::new(8192);
    let replay = query_replay(vec![source(1, 100, Some(10))]);
    let key = dashboard_request(100, 100).source_key();
    let frame = cache.capture(&source(1, 100, Some(10))).unwrap();
    let bytes = cache.bytes();
    cache.insert(
        key.clone(),
        &replay,
        0,
        ProjectedBlock {
            frames: vec![frame],
            through: 100,
        },
    );
    let held = cache.get(&key, &replay, 0, 100).unwrap();
    cache.retain_sources(|_| false);
    assert_eq!(cache.bytes(), bytes);
    assert!(cache.get(&key, &replay, 0, 100).is_none());
    drop(held);
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn multi_country_exact_windows_share_gap_boundaries_and_preserve_native_values() {
    let mut frames: Vec<_> = [100, 101, 103]
        .into_iter()
        .map(|hour| source(1, hour, Some(i64::from(hour))))
        .collect();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut frame.tables[TableId::Countries.index()]
        {
            let mut second = rows[0].clone();
            second.id = 10001;
            second.canonical_slot = 9;
            second.tag = b"SOV".to_vec();
            second.name = b"Soviet Union".to_vec();
            second.political_power_raw = Some(i64::MIN);
            rows.push(second);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 10001,
                canonical_slot: 9,
            });
        }
    }
    let replay = query_replay(frames);
    let mut request = dashboard_request(100, 103);
    request.country_ids = vec!["9".into(), "7".into(), "9".into()];
    let mut scan =
        crate::CountryScan::start(replay, &request, crate::QueryLimits::default()).unwrap();
    let result = finish_scan(&mut scan);
    assert_eq!(result.country_ids, ["7", "9"]);
    assert_eq!(result.series.len(), 2);
    assert_eq!(result.sample_provenance, crate::SampleProvenance::Unknown);
    for series in &result.series {
        assert!(!series.downsampled);
        assert_eq!(series.coverage.recorded_hours, 3);
        assert_eq!(series.coverage.gap_hours, 1);
        assert_eq!(series.points[2].status, PointStatus::Gap);
        assert!(series.points[2].first.is_none());
        for point in &series.points {
            assert_eq!(point.first_hour, point.last_hour);
        }
    }
    assert_eq!(
        result.series[1].points[0]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("-9223372036854775808")
    );
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::QueryResult>(&json).unwrap(),
        result
    );
}

#[test]
fn exact_windows_do_not_use_history_envelope_endpoints_for_interior_values() {
    let replay = query_replay(vec![
        source(1, 100, Some(10)),
        source(1, 101, Some(999)),
        source(1, 102, Some(20)),
    ]);
    let mut request = dashboard_request(100, 102);
    request.kind = crate::QueryKind::History;
    request.max_points = Some(1);
    let mut history = crate::CountryScan::start(
        std::sync::Arc::clone(&replay),
        &request,
        crate::QueryLimits::default(),
    )
    .unwrap();
    let result = finish_scan(&mut history);
    let point = &result.series[0].points[0];
    assert_eq!(
        point
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("10")
    );
    assert_eq!(
        point
            .last
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("20")
    );
    assert_eq!(point.max.political_power.as_deref(), Some("999"));
    let request = dashboard_request(101, 101);
    let mut exact =
        crate::CountryScan::start(replay, &request, crate::QueryLimits::default()).unwrap();
    let result = finish_scan(&mut exact);
    assert_eq!(result.series[0].points[0].first.as_ref().unwrap().hour, 101);
    assert_eq!(
        result.series[0].points[0]
            .first
            .as_ref()
            .unwrap()
            .values
            .political_power
            .as_deref(),
        Some("999")
    );
}

#[test]
fn cooperative_query_cancellation_does_not_publish_or_reset_partial_progress() {
    let replay = query_replay(vec![
        source(1, 100, Some(1)),
        source(1, 101, Some(2)),
        source(1, 102, Some(3)),
    ]);
    let mut scan = crate::CountryScan::start(
        replay,
        &dashboard_request(100, 102),
        crate::QueryLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        scan.advance(
            std::num::NonZeroUsize::new(1).unwrap(),
            &AtomicBool::new(false)
        ),
        Ok(crate::ScanStep::Yielded)
    ));
    let error = scan
        .advance(
            std::num::NonZeroUsize::new(1).unwrap(),
            &AtomicBool::new(true),
        )
        .err()
        .unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::Cancelled);
    let result = finish_scan(&mut scan);
    assert_eq!(result.series[0].coverage.recorded_hours, 3);
    assert_eq!(result.series[0].points.len(), 3);
}

#[test]
fn oversized_exact_requests_and_small_output_or_block_budgets_fail_explicitly() {
    let replay = query_replay(vec![source(1, 100, Some(1)), source(1, 1300, Some(2))]);
    let error = crate::CountryScan::start(
        std::sync::Arc::clone(&replay),
        &dashboard_request(100, 1300),
        crate::QueryLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::ResourceLimit);
    let mut limits = crate::QueryLimits {
        max_result_bytes: 1,
        ..Default::default()
    };
    let error = crate::CountryScan::start(
        std::sync::Arc::clone(&replay),
        &dashboard_request(100, 100),
        limits.clone(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::ResourceLimit);
    limits.max_result_bytes = crate::QueryLimits::default().max_result_bytes;
    limits.max_block_raw_bytes = 1;
    let mut scan = crate::CountryScan::start(replay, &dashboard_request(100, 100), limits).unwrap();
    let error = scan
        .advance(
            std::num::NonZeroUsize::new(1).unwrap(),
            &AtomicBool::new(false),
        )
        .err()
        .unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::ResourceLimit);
}

#[test]
fn shared_runtime_subscriptions_survive_other_scopes_and_obsolete_cancellation() {
    let replay = query_replay(vec![source(1, 100, Some(1)), source(1, 101, Some(2))]);
    let runtime = crate::AnalyticsRuntime::new(crate::RuntimeLimits::default()).unwrap();
    let first = dashboard_request(100, 101);
    let source = runtime
        .register_source(first.source_key(), "memory.battleplan".into(), replay)
        .unwrap();
    let old = runtime.request(first.clone()).unwrap();
    let mut second = first.clone();
    second.scope_id = "other-widget".into();
    let second = runtime.request(second).unwrap();
    let mut replacement = first.clone();
    replacement.generation = "2".into();
    let replacement = runtime.request(replacement).unwrap();
    runtime.cancel(&first.ticket());
    drop(old);
    let a = second.recv().unwrap();
    let b = replacement.recv().unwrap();
    assert!(a.shares_result(&b));
    assert_eq!(a.series[0].coverage.recorded_hours, 2);
    assert_eq!(runtime.stats().scans_started, 1);
    assert_eq!(runtime.stats().completed_scans, 1);
    assert_eq!(runtime.stats().subscriptions, 0);
    assert_eq!(source.metadata().countries[0].id, "7");
    runtime.shutdown();
}

#[test]
fn cache_eviction_does_not_uncharge_results_pinned_by_their_consumers() {
    let limits = crate::RuntimeLimits {
        output_bytes: 64 * 1024,
        query: crate::QueryLimits {
            max_result_bytes: 64 * 1024,
            ..Default::default()
        },
        ..Default::default()
    };
    let runtime = crate::AnalyticsRuntime::new(limits).unwrap();
    let request = dashboard_request(100, 100);
    let source = runtime
        .register_source(
            request.source_key(),
            "memory.battleplan".into(),
            query_replay(vec![source(1, 100, Some(1)), source(1, 101, Some(2))]),
        )
        .unwrap();
    let held = runtime.request(request.clone()).unwrap().recv().unwrap();
    let bytes = held.retained_bytes();
    assert_eq!(runtime.stats().output_bytes, bytes);
    let mut next = dashboard_request(101, 101);
    next.scope_id = "new-query".into();
    let error = runtime.request(next).unwrap().recv().err().unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::ResourceLimit);
    assert_eq!(runtime.stats().cache_entries, 0);
    assert_eq!(runtime.stats().output_bytes, bytes);
    runtime.close_source(source.key());
    assert_eq!(runtime.stats().output_bytes, bytes);
    drop(held);
    assert_eq!(runtime.stats().output_bytes, 0);
}

#[test]
fn source_handles_share_backing_and_keep_closed_map_pins_accounted() {
    let runtime = crate::AnalyticsRuntime::new(crate::RuntimeLimits::default()).unwrap();
    let replay = query_replay(vec![source(1, 100, Some(1))]);
    let key = dashboard_request(100, 100).source_key();
    let handle = runtime
        .register_source(
            key.clone(),
            "memory.battleplan".into(),
            std::sync::Arc::clone(&replay),
        )
        .unwrap();
    let map = handle.replay();
    assert!(std::sync::Arc::ptr_eq(&replay, &map));
    let bytes = runtime.stats().source_bytes;
    runtime.close_source(&key);
    assert!(runtime.source(&key).is_none());
    drop(handle);
    drop(replay);
    assert_eq!(runtime.stats().retired_sources, 1);
    assert_eq!(runtime.stats().source_bytes, bytes);
    drop(map);
    assert_eq!(runtime.stats().retired_sources, 0);
    assert_eq!(runtime.stats().source_bytes, 0);
}

#[test]
fn source_cancellation_is_checked_before_io_and_between_catalogue_frames() {
    let runtime = crate::AnalyticsRuntime::new(crate::RuntimeLimits::default()).unwrap();
    let key = dashboard_request(100, 100).source_key();
    let error = runtime
        .open_source_cancellable(
            key,
            "unused-nonexistent.battleplan".into(),
            &AtomicBool::new(true),
        )
        .err()
        .unwrap();
    assert_eq!(error.code, crate::QueryErrorCode::Cancelled);
    assert_eq!(runtime.stats().opening_bytes, 0);
    let replay = query_replay(vec![source(1, 100, Some(1))]);
    let checks = std::cell::Cell::new(0);
    let result = Analytics::from_shared_checked(replay, "memory.battleplan".into(), 100, || {
        checks.set(checks.get() + 1);
        if checks.get() == 3 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled at frame",
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(
        result.err().unwrap().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(checks.get(), 3);
}
