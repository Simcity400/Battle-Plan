use super::{
    Course, RaidEndedSource, RaidLog, RaidLogParams, RaidPreparationRun, RaidVictimSource,
    StretchRun, coalesce_spans, date_bounds, disappearance, exact_at, outcome_label,
    outcome_missing, phase_label, province_of, scope_of, victim_source,
};
use dashboard_base::{
    Source, SourceKey,
    products::{LoadContext, Product},
    test_fixtures::{CAMPAIGN, records, source as base_frame},
};
use engine_native_air::craid_instance as native;
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
use replay_data::rows::{
    CountryLinkRow, CountryRow, LexicalTokenRow, ProvinceRow, RaidRow, RaidScope, RaidTargetRow,
    StateRow, TableId, TableRows,
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, TableView},
};
use replay_lanes::lanes::{Cell, Step};
use std::sync::{Arc, atomic::AtomicBool};

fn step(hour: i32, phase: i32, outcome: i32, travelled: Option<i64>, preparation: i64) -> Step {
    let travelled = travelled.map_or(Cell::Missing, Cell::Number);
    Step { hour, cells: Some(Box::new([Cell::Number(7), Cell::Number(i64::from(phase)), Cell::Number(i64::from(outcome)), travelled, Cell::Number(preparation)])) }
}

fn gap(hour: i32) -> Step {
    Step { hour, cells: None }
}

fn srun(stretch: usize, first: i32, last: i32, phase: i32, outcome: i32) -> StretchRun {
    StretchRun { stretch, first, last, phase, outcome }
}

fn raid_row(id: u32, raider_slot: u32, phase: i32, outcome: i32, travelled: Option<i64>) -> RaidRow {
    RaidRow {
        reference_type: 84,
        id,
        parent_slot: raider_slot,
        ordinal: 0,
        phase,
        outcome,
        travelled,
        preparation_hours: 1,
        ..Default::default()
    }
}

fn target_row(id: u32, scope: RaidScope, raid_type: u32) -> RaidTargetRow {
    RaidTargetRow { reference_type: 84, id, scope, raid_type, detected: 0 }
}

fn make_frame(
    branch: u64,
    hour: i32,
    countries: Vec<(u32, u32, &[u8])>,
    province_controller: u32,
    raids_view: TableView,
    targets: Vec<RaidTargetRow>,
) -> DecodedFrame {
    let mut frame = base_frame(branch, hour, Some(1));
    let country_rows: Vec<CountryRow> = countries
        .iter()
        .map(|&(id, slot, tag)| CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..Default::default() })
        .collect();
    let link_rows: Vec<CountryLinkRow> = countries
        .iter()
        .map(|&(id, slot, _)| CountryLinkRow { id, canonical_slot: slot })
        .collect();
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(country_rows));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(link_rows));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
        id: 10,
        state: 5,
        owner: province_controller,
        controller: province_controller,
        ..Default::default()
    }]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 5,
        owner: province_controller,
        controller: province_controller,
        ..Default::default()
    }]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow { token: 40, literal: b"air_raid".to_vec() },
        LexicalTokenRow { token: 41, literal: b"dam_raid".to_vec() },
    ]));
    if matches!(raids_view, TableView::Complete(_) | TableView::Truncated { .. }) {
        frame.metrics.table_ns[TableId::Raids.index()] = 1;
    }
    *frame.tables[TableId::Raids.index()] = raids_view;
    *frame.tables[TableId::RaidTargets.index()] = TableView::Complete(TableRows::RaidTargets(targets));
    frame
}

fn load_product(segments: &[Vec<Record>], branch: &str, from: Option<i32>, to: Option<i32>) -> super::RaidLogReply {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), segments)
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        SourceKey { source_id: "raid-test".into(), revision: "1".into() },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    RaidLog::load(
        &source,
        &RaidLogParams { branch_id: branch.into(), from, to, countries: Vec::new(), catalogue: false },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn loaded_records(branch: u64, parent_branch: u64, parent_hour: i32, frames: Vec<DecodedFrame>) -> Vec<Record> {
    let mut out = records(branch, frames);
    if let Record::BranchStart(start) = &mut out[1] {
        *start = BranchStart {
            id: branch,
            session: branch,
            loaded_hour: Some(parent_hour),
            first_hour: start.first_hour,
            parent: Parentage::Probable { branch: parent_branch, hour: parent_hour },
            reason: BranchReason::LoadSignal,
        };
    }
    out
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
fn phase_dates_require_same_stretch_complete_reads_and_lower_bound_from_the_raids_own_prior_read() {
    let runs = [srun(0, 10, 14, 1, 0), srun(0, 15, 24, 2, 0), srun(0, 25, 30, 5, 3)];
    let complete = vec![coalesce_spans(vec![(5, 19), (20, 35)])];
    let exact = date_bounds(&runs, 10, &complete);
    assert_eq!(exact, vec![(true, Some(9)), (true, Some(14)), (true, Some(24))]);
    assert!(exact_at(25, 0, 10, true, &runs, &complete));

    let cross_load = [srun(0, 10, 19, 2, 0), srun(1, 20, 30, 5, 3)];
    let split_complete = vec![vec![(5, 19)], vec![(20, 35)]];
    let across = date_bounds(&cross_load, 10, &split_complete);
    assert_eq!(across[1], (false, Some(19)));
    assert!(!exact_at(20, 1, 10, true, &cross_load, &split_complete));

    let omitted_at_19 = [srun(0, 10, 10, 2, 0), srun(0, 20, 25, 5, 3)];
    let truncated_complete = vec![vec![(10, 10), (20, 25)]];
    let bounded = date_bounds(&omitted_at_19, 10, &truncated_complete);
    assert_eq!(bounded, vec![(false, None), (false, Some(10))]);
}

#[test]
fn disappearance_requires_same_stretch_complete_reads() {
    let complete = vec![vec![(10, 20), (25, 30)]];
    assert_eq!(disappearance(true, 19, 0, None, &complete), (None, false));
    assert_eq!(disappearance(false, 19, 0, Some((20, 0)), &complete), (Some(20), true));
    assert_eq!(disappearance(false, 20, 0, None, &complete), (Some(25), false));
    assert_eq!(disappearance(false, 30, 0, None, &complete), (None, false));
    let cross_load = vec![vec![(10, 19)], vec![(20, 25)]];
    assert_eq!(disappearance(false, 19, 0, None, &cross_load), (Some(20), false));
}

#[test]
fn victim_source_reserves_fixed_target_for_encoded_country_and_labels_leader_captor_by_observation() {
    let fixed_building = RaidScope { building_type: 9, building_country: 4, ..RaidScope::default() };
    let leader_captor = RaidScope { leader_reference: 11, leader_country: Some(7), ..RaidScope::default() };
    let mutable_province = RaidScope { province: 44, ..RaidScope::default() };
    let mutable_building = RaidScope { building_type: 9, building_location: 44, building_flags: 1, ..RaidScope::default() };
    assert_eq!(victim_source(&fixed_building, true), RaidVictimSource::FixedTarget);
    assert_eq!(victim_source(&leader_captor, false), RaidVictimSource::CaptorAtObservedRead);
    assert_eq!(victim_source(&leader_captor, true), RaidVictimSource::CaptorAtFirstEndedRead);
    assert_eq!(victim_source(&mutable_province, true), RaidVictimSource::ControllerAtFirstEndedRead);
    assert_eq!(victim_source(&mutable_building, false), RaidVictimSource::ControllerAtObservedRead);
}

#[test]
fn product_splits_reused_key_across_witnessed_absence_and_load_while_merging_continuous_identity() {
    let countries = vec![(10, 1, b"ENG".as_slice()), (20, 2, b"GER".as_slice()), (30, 3, b"FRA".as_slice())];
    let scope_a = RaidScope { province: 10, ..Default::default() };
    let branch1 = records(1, vec![
        make_frame(1, 10, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(7, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(7, scope_a.clone(), 40)]),
        make_frame(1, 11, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![])), vec![]),
    ]);
    let branch2 = loaded_records(2, 1, 11, vec![
        make_frame(2, 12, countries.clone(), 30, TableView::Complete(TableRows::Raids(vec![raid_row(7, 2, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(200))])), vec![target_row(7, scope_a.clone(), 41)]),
    ]);
    let reply = load_product(&[branch1, branch2], "1", None, None);
    assert_eq!(reply.episodes.len(), 2, "key 7 reused after witnessed absence and load must stay two episodes");
    assert_eq!(reply.episodes[0].raider.as_deref(), Some("ENG"));
    assert_eq!(reply.episodes[1].raider.as_deref(), Some("GER"));
    assert_ne!(reply.episodes[0].id, reply.episodes[1].id);

    let same_b1 = records(1, vec![
        make_frame(1, 10, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(9, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(9, scope_a.clone(), 40)]),
    ]);
    let same_b2 = loaded_records(2, 1, 10, vec![
        make_frame(2, 11, countries, 20, TableView::Complete(TableRows::Raids(vec![raid_row(9, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(9, scope_a, 40)]),
    ]);
    let merged = load_product(&[same_b1, same_b2], "1", None, None);
    assert_eq!(merged.episodes.len(), 1, "same key across load with matching raider, type, target and monotonic phase merges");
    assert!(!merged.episodes[0].ended_exact, "ended across a load must never be exact");
    assert_eq!(merged.episodes[0].runs[1].after_hour, Some(10));
}

#[test]
fn product_rejects_exactness_from_truncated_reads_and_keeps_exactness_at_range_start() {
    let countries = vec![(10, 1, b"ENG".as_slice()), (20, 2, b"GER".as_slice())];
    let scope = RaidScope { province: 10, ..Default::default() };
    let truncated_run = records(1, vec![
        make_frame(1, 10, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(1, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(1, scope.clone(), 40)]),
        make_frame(1, 11, countries.clone(), 20, TableView::Truncated { rows: TableRows::Raids(vec![]), missing: 1 }, vec![]),
        make_frame(1, 12, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(1, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(1, scope.clone(), 40)]),
    ]);
    let reply = load_product(&[truncated_run], "1", None, None);
    assert_eq!(reply.episodes.len(), 1);
    assert!(!reply.episodes[0].ended_exact);
    assert_eq!(reply.episodes[0].runs[1].first_exact, false);
    assert_eq!(reply.episodes[0].runs[1].after_hour, Some(10), "lower bound is the raid's last prior read, not the truncated hour");
    assert!(
        reply.missing.iter().any(|span| (span.first_hour, span.last_hour) == (11, 11) && span.reason.contains("read only in part")),
        "an hour whose raids were not read whole is missing, never shown as no raids: {:?}",
        reply.missing
    );

    let range_run = records(1, vec![
        make_frame(1, 19, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(2, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(2, scope.clone(), 40)]),
        make_frame(1, 20, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(2, 1, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE, Some(10))])), vec![target_row(2, scope.clone(), 40)]),
        make_frame(1, 21, countries, 20, TableView::Complete(TableRows::Raids(vec![raid_row(2, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(2, scope, 40)]),
    ]);
    let ranged = load_product(&[range_run], "1", Some(20), Some(21));
    assert_eq!(ranged.episodes.len(), 1);
    assert_eq!(ranged.episodes[0].launched_hour, Some(20));
    assert!(ranged.episodes[0].launched_exact, "preceding hour 19 proves launch at range start 20 is exact");
    assert_eq!(ranged.episodes[0].runs[0].first_hour, 20);
    assert!(ranged.episodes[0].runs[0].first_exact);
    assert_eq!(ranged.episodes[0].runs[0].after_hour, Some(19));
}

#[test]
fn product_reads_baseline_ended_controller_at_first_ended_hour_and_resolves_raider_tag_per_event() {
    let scope = RaidScope { province: 10, ..Default::default() };
    let baseline_ended = records(1, vec![
        make_frame(1, 10, vec![(10, 1, b"ENG"), (20, 2, b"FRA"), (30, 3, b"GER")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(5, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(5, scope.clone(), 40)]),
        make_frame(1, 11, vec![(10, 1, b"ENG"), (20, 2, b"FRA"), (30, 3, b"GER")], 30, TableView::Complete(TableRows::Raids(vec![raid_row(5, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(5, scope.clone(), 40)]),
        make_frame(1, 12, vec![(10, 1, b"ENG"), (20, 2, b"FRA"), (30, 3, b"GER")], 30, TableView::Complete(TableRows::Raids(vec![raid_row(5, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(5, scope.clone(), 40)]),
    ]);
    let reply = load_product(std::slice::from_ref(&baseline_ended), "1", None, None);
    assert_eq!(reply.episodes.len(), 1);
    assert_eq!(reply.episodes[0].victim.as_deref(), Some("FRA"), "baseline-ended raid must read controller at first ended hour 10, not later hour 11");
    assert_eq!(reply.episodes[0].victim_source, Some(RaidVictimSource::ControllerAtFirstEndedRead));
    let ranged = load_product(std::slice::from_ref(&baseline_ended), "1", Some(12), Some(12));
    assert_eq!(ranged.episodes.len(), 1);
    assert_eq!(ranged.episodes[0].victim.as_deref(), Some("FRA"), "a ranged query reads the raid's own first ended observation, not the range's baseline hour");

    let tag_change = records(1, vec![
        make_frame(1, 10, vec![(10, 1, b"GER"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(6, scope.clone(), 40)]),
        make_frame(1, 11, vec![(10, 1, b"DDR"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE, Some(50))])), vec![target_row(6, scope.clone(), 40)]),
        make_frame(1, 12, vec![(10, 1, b"DDR"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(6, scope, 40)]),
    ]);
    let changed = load_product(&[tag_change], "1", None, None);
    assert_eq!(changed.episodes.len(), 1);
    assert_eq!(changed.episodes[0].raider.as_deref(), Some("GER"));
    assert_eq!(changed.episodes[0].launched_raider.as_deref(), Some("DDR"));
    assert_eq!(changed.episodes[0].ended_raider.as_deref(), Some("DDR"));
    assert_eq!(changed.episodes[0].runs[0].raider.as_deref(), Some("GER"));
    assert_eq!(changed.episodes[0].runs[1].raider.as_deref(), Some("DDR"));
}

#[test]
fn product_prefers_native_end_hour_and_completion_country_and_distinguishes_known_absent_from_withheld() {
    let countries = vec![(10, 1, b"ENG".as_slice()), (20, 2, b"POL".as_slice()), (30, 3, b"GER".as_slice())];
    let scope = RaidScope { province: 10, ..Default::default() };
    let recorded_both = RaidRow::END_HOUR_RECORDED | RaidRow::COMPLETION_COUNTRY_RECORDED;

    let mut ended_row = raid_row(1, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100));
    ended_row.end_hour = Some(5);
    ended_row.completion_country = Some(20);
    ended_row.completion_recorded = recorded_both;
    let mut absent_row = raid_row(2, 1, native::PHASE_ENDED, native::OUTCOME_CANCELED, None);
    absent_row.end_hour = Some(7);
    absent_row.completion_country = None;
    absent_row.completion_recorded = recorded_both;
    let mut withheld_row = raid_row(3, 1, native::PHASE_ENDED, native::OUTCOME_FAILURE, Some(50));
    withheld_row.end_hour = None;
    withheld_row.completion_country = None;
    withheld_row.completion_recorded = 0;

    let run = records(1, vec![
        make_frame(
            1,
            20,
            countries,
            30,
            TableView::Complete(TableRows::Raids(vec![ended_row, absent_row, withheld_row])),
            vec![
                target_row(1, scope.clone(), 40),
                target_row(2, scope.clone(), 40),
                target_row(3, scope, 40),
            ],
        ),
    ]);
    let reply = load_product(&[run], "1", None, None);
    assert_eq!(reply.episodes.len(), 3);

    let native_ep = &reply.episodes[0];
    assert_eq!(native_ep.ended_hour, Some(5));
    assert_eq!(native_ep.ended_seen_hour, Some(20), "the native end date is first seen at the read that found it ended");
    assert!(native_ep.ended_exact, "native end_hour is exact even on a baseline-ended raid");
    assert_eq!(native_ep.ended_source, Some(RaidEndedSource::NativeEndDate));
    assert_eq!(native_ep.victim.as_deref(), Some("POL"), "native completion_country beats current controller GER");
    assert_eq!(native_ep.victim_source, Some(RaidVictimSource::CompletionCountry));

    let absent_ep = &reply.episodes[1];
    assert_eq!(absent_ep.ended_hour, Some(7));
    assert!(absent_ep.ended_exact);
    assert_eq!(absent_ep.victim, None, "known-absent completion_country must not fall back to controller");
    assert_eq!(absent_ep.victim_missing.as_deref(), Some("The raid's target resolved to no country at completion"));
    assert_eq!(absent_ep.victim_source, None);

    let withheld_ep = &reply.episodes[2];
    assert_eq!(withheld_ep.ended_hour, Some(20));
    assert!(!withheld_ep.ended_exact);
    assert_eq!(withheld_ep.ended_source, Some(RaidEndedSource::ObservedEndedRead));
    assert_eq!(withheld_ep.victim.as_deref(), Some("GER"));
    assert_eq!(withheld_ep.victim_source, Some(RaidVictimSource::ControllerAtFirstEndedRead));
}
