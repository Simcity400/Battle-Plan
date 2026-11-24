use super::{
    RaidEpisode, episode_at, pieces, prepare,
    RaidEndedSource, RaidLog, RaidLogParams, RaidPreparationRun, RaidVictimSource,
    StretchRun, coalesce_spans, date_bounds, disappearance, exact_at, outcome_label,
    outcome_missing, phase_label, province_of, scope_of, victim_source,
};
use dashboard_base::products::Country;
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
    load_with(segments, RaidLogParams { branch_id: branch.into(), from, to, countries: Vec::new(), catalogue: false })
}

fn load_with(segments: &[Vec<Record>], params: RaidLogParams) -> super::RaidLogReply {
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
        &params,
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

fn shown(seen: &[super::Piece]) -> Vec<(i32, i32, Option<(i32, i32, Option<u32>, bool)>)> {
    seen.iter().map(|piece| (piece.first, piece.last, piece.unit.as_ref().map(|unit| (unit.phase, unit.outcome, unit.raider, unit.launched)))).collect()
}

fn prepared(steps: &[Step], last: i32) -> Vec<RaidPreparationRun> {
    let mut runs = Vec::new();
    for piece in pieces(steps, last, &[]) {
        if let Some(hours) = piece.unit.and_then(|unit| unit.preparation) {
            prepare(&mut runs, piece.first, piece.last, hours);
        }
    }
    runs
}

#[test]
fn a_raids_rows_are_pieces_with_its_phase_raider_and_launch() {
    let steps = [
        step(10, native::PHASE_ASSEMBLING, native::OUTCOME_NONE, None, 0),
        step(20, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE, Some(0), 0),
        step(30, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(500), 0),
    ];
    assert_eq!(shown(&pieces(&steps, 50, &[])), vec![(10, 19, Some((1, 0, Some(7), false))), (20, 29, Some((4, 0, Some(7), true))), (30, 50, Some((5, 3, Some(7), true)))]);
    assert_eq!(shown(&pieces(&steps, 25, &[])).last(), Some(&(20, 25, Some((4, 0, Some(7), true)))));
    let flew = pieces(&[step(10, native::PHASE_PREPARED, 0, Some(0), 0), step(15, native::PHASE_PREPARED, 0, Some(250), 0)], 29, &[]);
    assert_eq!(flew.iter().map(|piece| piece.unit.as_ref().is_some_and(|unit| unit.launched)).collect::<Vec<_>>(), [false, true]);
}

#[test]
fn hours_the_row_is_not_shown_are_no_piece_and_a_target_change_cuts_one() {
    let seen = pieces(&[step(10, native::PHASE_PREPARING, 0, None, 3), gap(15), step(18, native::PHASE_PREPARING, 0, None, 9)], 20, &[12, 19]);
    assert_eq!(seen.iter().map(|piece| (piece.first, piece.last)).collect::<Vec<_>>(), [(10, 11), (12, 14), (18, 18), (19, 20)]);
    assert!(pieces(&[gap(10)], 20, &[]).is_empty());
    let blind = Step { hour: 10, cells: Some(Box::new([Cell::Number(7), Cell::Missing, Cell::Number(0)])) };
    assert_eq!(shown(&pieces(&[blind], 12, &[])), [(10, 12, None)], "a row whose phase is not recorded is shown but says nothing");
}

#[test]
fn preparation_hours_are_runs_of_a_steady_change_per_hour_and_never_span_a_gap() {
    let steps: Vec<Step> = (10..=13).map(|hour| step(hour, native::PHASE_PREPARING, 0, None, i64::from(hour - 5))).chain([gap(14), step(16, native::PHASE_PREPARING, 0, None, 30)]).collect();
    assert_eq!(prepared(&steps, 19), vec![RaidPreparationRun(10, 13, 5, 1), RaidPreparationRun(16, 19, 30, 0)]);
    assert_eq!(prepared(&[step(10, native::PHASE_PREPARING, 0, None, 4), step(13, native::PHASE_PREPARING, 0, None, 9)], 13), vec![RaidPreparationRun(10, 12, 4, 0), RaidPreparationRun(13, 13, 9, 0)]);
    assert!(prepared(&[step(10, native::PHASE_IN_PROGRESS, 0, None, 40)], 12).is_empty());
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
fn disappearance_is_the_next_whole_read_without_the_key_and_exact_only_the_hour_after_it_was_seen() {
    let complete = vec![vec![(10, 20), (25, 30)]];
    assert_eq!(disappearance(19, 0, None, &complete), Some((20, true)));
    assert_eq!(disappearance(20, 0, None, &complete), Some((25, false)));
    assert_eq!(disappearance(20, 0, Some(25), &complete), None, "the key read again at 25 under another raid proves no absence");
    assert_eq!(disappearance(30, 0, None, &complete), None);
    let cross_load = vec![vec![(10, 19)], vec![(20, 25)]];
    assert_eq!(disappearance(19, 0, None, &cross_load), Some((20, false)));
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
fn product_rejects_exactness_from_truncated_reads_and_keeps_exactness_at_range_start() {
    let countries = vec![(10, 1, b"ENG".as_slice()), (20, 2, b"GER".as_slice())];
    let scope = RaidScope { province: 10, ..Default::default() };
    let truncated_run = records(1, vec![
        make_frame(1, 10, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(1, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(1, scope.clone(), 40)]),
        make_frame(1, 11, countries.clone(), 20, TableView::Truncated { rows: TableRows::Raids(vec![]), missing: 1 }, vec![]),
        make_frame(1, 12, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid_row(1, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(1, scope.clone(), 40)]),
    ]);
    let reply = load_product(std::slice::from_ref(&truncated_run), "1", None, None);
    assert_eq!(reply.episodes.len(), 1);
    assert!(!reply.episodes[0].ends[0].exact);
    assert_eq!(reply.episodes[0].runs[1].first_exact, false);
    assert_eq!(reply.episodes[0].runs[1].after_hour, Some(10), "lower bound is the raid's last prior read, not the truncated hour");
    assert!(
        reply.missing.iter().any(|span| (span.first_hour, span.last_hour) == (11, 11) && span.reason.contains("read only in part")),
        "an hour whose raids were not read whole is missing, never shown as no raids: {:?}",
        reply.missing
    );
    let catalogue = load_with(std::slice::from_ref(&truncated_run), RaidLogParams { branch_id: "1".into(), from: None, to: None, countries: Vec::new(), catalogue: true });
    assert!(catalogue.episodes.is_empty() && catalogue.missing.is_empty(), "a catalogue request reads no raid and no raid coverage: {:?}", catalogue.missing);

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
fn product_resolves_raider_tag_per_event() {
    let scope = RaidScope { province: 10, ..Default::default() };
    let tag_change = records(1, vec![
        make_frame(1, 10, vec![(10, 1, b"GER"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_PREPARING, native::OUTCOME_NONE, None)])), vec![target_row(6, scope.clone(), 40)]),
        make_frame(1, 11, vec![(10, 1, b"DDR"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE, Some(50))])), vec![target_row(6, scope.clone(), 40)]),
        make_frame(1, 12, vec![(10, 1, b"DDR"), (20, 2, b"FRA")], 20, TableView::Complete(TableRows::Raids(vec![raid_row(6, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS, Some(100))])), vec![target_row(6, scope, 40)]),
    ]);
    let changed = load_product(&[tag_change], "1", None, None);
    assert_eq!(changed.episodes.len(), 1);
    assert_eq!(changed.episodes[0].raider.as_deref(), Some("GER"));
    assert_eq!(changed.episodes[0].launched_raider.as_deref(), Some("DDR"));
    assert_eq!(changed.episodes[0].ends[0].raider.as_deref(), Some("DDR"));
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

    let mut frames: Vec<DecodedFrame> = (4..20).map(|hour| make_frame(1, hour, countries.clone(), 30, TableView::Complete(TableRows::Raids(vec![])), vec![])).collect();
    frames.push(
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
    );
    let reply = load_product(&[records(1, frames)], "1", None, None);
    assert_eq!(reply.episodes.len(), 3);

    let native_ep = &reply.episodes[0];
    assert_eq!(native_ep.ends.iter().map(|end| (end.hour, end.ended_hour, end.exact, end.source)).collect::<Vec<_>>(), [(20, 5, true, RaidEndedSource::NativeEndDate)], "a native end date is exact, and known from the read that holds it");
    assert_eq!(native_ep.succeeded_hour, Some(20), "the success is first seen at the read that found it ended");
    assert_eq!(native_ep.victim.as_deref(), Some("POL"), "native completion_country beats current controller GER");
    assert_eq!(native_ep.victim_source, Some(RaidVictimSource::CompletionCountry));

    let absent_ep = &reply.episodes[1];
    assert_eq!(absent_ep.ends.iter().map(|end| (end.ended_hour, end.exact)).collect::<Vec<_>>(), [(7, true)]);
    assert_eq!(absent_ep.victim, None, "known-absent completion_country must not fall back to controller");
    assert_eq!(absent_ep.victim_missing.as_deref(), Some("The raid's target resolved to no country at completion"));
    assert_eq!(absent_ep.victim_source, None);

    let withheld_ep = &reply.episodes[2];
    assert_eq!(withheld_ep.ends.iter().map(|end| (end.ended_hour, end.exact, end.source)).collect::<Vec<_>>(), [(20, true, RaidEndedSource::ObservedEndedRead)], "absent from the whole read at 19, seen ended at 20");
    assert_eq!(withheld_ep.victim.as_deref(), Some("GER"));
    assert_eq!(withheld_ep.victim_source, Some(RaidVictimSource::ControllerAtFirstEndedRead));
}

enum Raids {
    Whole(Vec<RaidRow>),
    Part(Vec<RaidRow>),
    Unread(Vec<RaidRow>),
}

struct At {
    hour: i32,
    raids: Raids,
    controller: u32,
    spelled: bool,
    targeted: bool,
    provinces_part: bool,
    slot1: &'static [u8],
}

fn at(hour: i32, raids: Raids, controller: u32) -> At {
    At { hour, raids, controller, spelled: true, targeted: true, provinces_part: false, slot1: b"ENG" }
}

fn raid(id: u32, slot: u32, phase: i32, outcome: i32) -> RaidRow {
    raid_row(id, slot, phase, outcome, (phase >= native::PHASE_IN_PROGRESS).then_some(100))
}

fn frame_at(branch: u64, at: &At) -> DecodedFrame {
    let countries = vec![(10, 1, at.slot1), (20, 2, b"GER".as_slice()), (30, 3, b"FRA".as_slice())];
    let (view, rows) = match &at.raids {
        Raids::Whole(rows) => (TableView::Complete(TableRows::Raids(rows.clone())), rows),
        Raids::Part(rows) => (TableView::Truncated { rows: TableRows::Raids(rows.clone()), missing: 1 }, rows),
        Raids::Unread(rows) => (TableView::Unavailable { reason: "not read this hour".into() }, rows),
    };
    let targets = if at.targeted { rows.iter().map(|row| target_row(row.id, RaidScope { province: 10, ..Default::default() }, 40)).collect() } else { Vec::new() };
    let mut frame = make_frame(branch, at.hour, countries, at.controller, view, targets);
    if !at.spelled {
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 41, literal: b"dam_raid".to_vec() }]));
    }
    if at.provinces_part
        && let TableView::Complete(rows) = std::mem::replace(&mut *frame.tables[TableId::Provinces.index()], TableView::Unavailable { reason: String::new() })
    {
        *frame.tables[TableId::Provinces.index()] = TableView::Truncated { rows, missing: 1 };
    }
    frame
}

fn recording(stretches: &[Vec<At>]) -> Vec<Vec<Record>> {
    stretches
        .iter()
        .enumerate()
        .map(|(index, stretch)| {
            let branch = index as u64 + 1;
            let frames = stretch.iter().map(|at| frame_at(branch, at)).collect();
            if index == 0 { records(branch, frames) } else { loaded_records(branch, branch - 1, stretches[index - 1].last().unwrap().hour, frames) }
        })
        .collect()
}

type Victim = (i32, Option<String>, Option<RaidVictimSource>);
type EndSeen = (i32, i32, bool, Option<String>);

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    id: String,
    first: i32,
    slot: Option<u32>,
    victims: Vec<Victim>,
    type_hour: Option<i32>,
    launched: Option<i32>,
    ends: Vec<EndSeen>,
    succeeded: Option<i32>,
    disappeared: Option<(i32, bool)>,
    outcome_known: bool,
}

fn seen(episode: &RaidEpisode) -> Seen {
    Seen {
        id: episode.id.clone(),
        first: episode.first_hour,
        slot: episode.raider_slot,
        victims: episode.victims.iter().map(|v| (v.hour, v.tag.clone(), v.source)).collect(),
        type_hour: episode.raid_type_hour,
        launched: episode.launched_hour,
        ends: episode.ends.iter().map(|end| (end.hour, end.ended_hour, end.source == RaidEndedSource::NativeEndDate, end.raider.clone())).collect(),
        succeeded: episode.succeeded_hour,
        disappeared: episode.disappeared_hour.map(|hour| (hour, episode.disappeared_exact)),
        outcome_known: episode.outcome_missing.is_none(),
    }
}

fn ep(raid: u32, nth: u32, slot: u32, victims: &[(i32, &str, RaidVictimSource)]) -> Seen {
    let key = (u128::from(raid) << 32 | 84).to_string();
    Seen {
        id: if nth == 1 { key } else { format!("{key}#{nth}") },
        first: victims[0].0,
        slot: Some(slot),
        victims: victims.iter().map(|&(hour, tag, source)| (hour, Some(tag.to_owned()), Some(source))).collect(),
        type_hour: Some(victims[0].0),
        launched: None,
        ends: Vec::new(),
        succeeded: None,
        disappeared: None,
        outcome_known: true,
    }
}

impl Seen {
    fn done(mut self, hour: i32, tag: &str) -> Self {
        self.launched = Some(hour);
        self.ends.push((hour, hour, false, Some(tag.to_owned())));
        self
    }

    fn native(mut self, hour: i32, ended: i32, tag: &str) -> Self {
        self.ends.push((hour, ended, true, Some(tag.to_owned())));
        self
    }

    fn won(mut self, hour: i32) -> Self {
        self.succeeded = Some(hour);
        self
    }

    fn launched(mut self, hour: i32) -> Self {
        self.launched = Some(hour);
        self
    }

    fn typed(mut self, hour: Option<i32>) -> Self {
        self.type_hour = hour;
        self
    }

    fn untargeted(mut self) -> Self {
        let hour = self.first;
        self.victims = vec![(hour, None, None)];
        self
    }

    fn unknown_outcome(mut self) -> Self {
        self.outcome_known = false;
        self
    }

    fn gone(mut self, hour: i32, exact: bool) -> Self {
        self.disappeared = Some((hour, exact));
        self.unknown_outcome()
    }
}

fn malformed(reply: &super::RaidLogReply) -> Vec<String> {
    let mut out = Vec::new();
    for e in &reply.episodes {
        let mut bad = |rule: &str| out.push(format!("{}: {rule}", e.id));
        let increasing = |hours: Vec<i32>| hours.windows(2).all(|pair| pair[0] < pair[1]);
        if e.first_hour > e.last_hour {
            bad("first hour after the last");
        }
        if e.victim.is_none() == e.victim_missing.is_none() || (e.victim.is_none() && e.victim_source.is_some()) {
            bad("victim");
        }
        if e.victims.is_empty() || !increasing(e.victims.iter().map(|v| v.hour).collect()) || e.victims.iter().any(|v| v.tag.is_none() == v.missing.is_none() || (v.tag.is_none() && v.source.is_some())) {
            bad("victim witnesses");
        }
        if !increasing(e.ends.iter().map(|end| end.hour).collect()) {
            bad("end witnesses not strictly increasing");
        }
        if e.runs.iter().any(|run| run.first_hour > run.last_hour) || e.runs.windows(2).any(|pair| pair[0].last_hour >= pair[1].first_hour) {
            bad("runs");
        }
        if e.preparation.iter().any(|run| run.0 > run.1) || e.preparation.windows(2).any(|pair| pair[0].1 >= pair[1].0) {
            bad("preparation");
        }
        if (e.launched_hour.is_some() && !e.launched) || (e.launched_exact && e.launched_hour.is_none()) {
            bad("launch");
        }
        if e.disappeared_exact && e.disappeared_hour.is_none() {
            bad("disappearance");
        }
        if e.succeeded_hour.is_some() && e.ends.is_empty() {
            bad("a success without an end");
        }
    }
    out
}

#[test]
fn every_raid_episode_case_gives_the_full_reply_cut_at_each_hour_as_the_reply_through_that_hour() {
    use RaidVictimSource::{CompletionCountry as DONE, ControllerAtFirstEndedRead as ENDED, ControllerAtObservedRead as SEEN};
    use native::{OUTCOME_CRITICAL_SUCCESS as CRITICAL, OUTCOME_FAILURE as LOST, OUTCOME_NONE as NONE, OUTCOME_SUCCESS as WON, PHASE_ENDED as END, PHASE_IN_PROGRESS as FLYING, PHASE_PREPARING as PREP};
    let (ger, fra) = (20, 30);
    let whole = |rows: Vec<RaidRow>| Raids::Whole(rows);
    let won = |id, slot| raid(id, slot, END, WON);
    let prep = |id, slot| raid(id, slot, PREP, NONE);
    let dated = |id, end_hour| {
        let mut row = won(id, 1);
        row.end_hour = Some(end_hour);
        row.completion_recorded = RaidRow::END_HOUR_RECORDED;
        row
    };
    let mut completed = raid(5, 1, END, LOST);
    completed.completion_country = Some(fra);
    completed.completion_recorded = RaidRow::COMPLETION_COUNTRY_RECORDED;
    let cases: Vec<(&str, Vec<Vec<At>>, Vec<Seen>)> = vec![
        (
            "R2-S1: a complete absence ends a raid; the key's return is another",
            vec![vec![at(10, whole(vec![won(7, 1)]), fra), at(11, whole(vec![]), fra), at(12, whole(vec![won(7, 1)]), ger), at(13, whole(vec![won(7, 1)]), ger), at(14, whole(vec![won(7, 1)]), ger)]],
            vec![ep(7, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10), ep(7, 2, 1, &[(12, "GER", ENDED)]).done(12, "ENG").won(12)],
        ),
        (
            "rv5-6: a raid already ended at the first hour names the controller of that hour",
            vec![vec![at(10, whole(vec![won(5, 1)]), fra), at(11, whole(vec![won(5, 1)]), ger), at(12, whole(vec![won(5, 1)]), ger)]],
            vec![ep(5, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10)],
        ),
        (
            "rv5-6: a raid carried over a load with the same identity keeps its controller",
            vec![(10..=12).map(|h| at(h, whole(vec![won(9, 1)]), fra)).collect(), (13..=16).map(|h| at(h, whole(vec![won(9, 1)]), ger)).collect()],
            vec![ep(9, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10)],
        ),
        (
            "rv5-3a: a success counts from the read that first shows it",
            vec![(10..=21).map(|h| at(h, whole(vec![raid(5, 1, END, if h < 20 { NONE } else { WON })]), fra)).collect()],
            vec![ep(5, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(20)],
        ),
        (
            "R2-S2: the owner is the slot's tag at the native end, the success where it was read",
            vec![(4..=20).map(|h| At { slot1: if h < 20 { b"GER" } else { b"DDR" }, ..at(h, whole(if h < 20 { vec![] } else { vec![dated(3, 5)] }), fra) }).collect()],
            vec![ep(3, 1, 1, &[(20, "FRA", ENDED)]).launched(20).native(20, 5, "GER").won(20)],
        ),
        (
            "a raid joins over a load when raider, target and type match at the join hour",
            vec![vec![at(10, whole(vec![prep(9, 1)]), ger)], vec![at(11, whole(vec![won(9, 1)]), ger)]],
            vec![ep(9, 1, 1, &[(10, "GER", SEEN), (11, "GER", ENDED)]).done(11, "ENG").won(11)],
        ),
        (
            "a truncated read proves no absence, and the raid resumes on a matching identity",
            vec![vec![at(10, whole(vec![prep(1, 1)]), ger), at(11, Raids::Part(vec![]), ger), at(12, whole(vec![won(1, 1)]), ger)]],
            vec![ep(1, 1, 1, &[(10, "GER", SEEN), (12, "GER", ENDED)]).done(12, "ENG").won(12)],
        ),
        (
            "an unread hour with the same identity on both sides keeps one raid",
            vec![vec![at(10, whole(vec![prep(7, 1)]), ger), at(11, Raids::Unread(vec![prep(7, 1)]), ger), at(12, whole(vec![won(7, 1)]), ger)]],
            vec![ep(7, 1, 1, &[(10, "GER", SEEN), (12, "GER", ENDED)]).done(12, "ENG").won(12)],
        ),
        (
            "R3-S1, R4-S3: a type not spelled at the join hour is no match, and is shown only from the hour it is spelled",
            vec![vec![at(10, whole(vec![prep(7, 1)]), ger)], vec![At { spelled: false, ..at(11, whole(vec![prep(7, 1)]), ger) }, at(12, whole(vec![prep(7, 1)]), ger)]],
            vec![ep(7, 1, 1, &[(10, "GER", SEEN)]), ep(7, 2, 1, &[(11, "GER", SEEN)]).typed(Some(12))],
        ),
        (
            "R3-S2: a raid by another raider after a complete absence is another raid, and its absence read whole is its disappearance",
            vec![vec![at(10, whole(vec![won(7, 1)]), fra), at(11, whole(vec![]), fra), at(12, whole(vec![prep(7, 2)]), ger), at(13, whole(vec![]), ger)]],
            vec![ep(7, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10), ep(7, 2, 2, &[(12, "GER", SEEN)]).gone(13, true)],
        ),
        (
            "R3-S3: another raider and a phase going back split across an unread hour",
            vec![vec![at(10, whole(vec![won(7, 1)]), fra), at(11, Raids::Unread(vec![]), fra), at(12, whole(vec![prep(7, 2)]), ger)]],
            vec![ep(7, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10), ep(7, 2, 2, &[(12, "GER", SEEN)])],
        ),
        (
            "R3-S4: an unknown outcome becomes known over a load and keeps the first victim",
            vec![vec![at(10, whole(vec![prep(5, 1)]), fra), at(11, whole(vec![raid(5, 1, END, NONE)]), fra)], vec![at(12, whole(vec![won(5, 1)]), ger)]],
            vec![ep(5, 1, 1, &[(10, "FRA", SEEN), (11, "FRA", ENDED)]).done(11, "ENG").won(12)],
        ),
        (
            "R3-S5: an end read with no outcome is filled by the success read later",
            vec![std::iter::once(at(9, whole(vec![prep(5, 1)]), fra)).chain((10..=20).map(|h| at(h, whole(vec![raid(5, 1, END, if h < 20 { NONE } else { WON })]), fra))).collect()],
            vec![ep(5, 1, 1, &[(9, "FRA", SEEN), (10, "FRA", ENDED)]).done(10, "ENG").won(20)],
        ),
        (
            "R3-S6: a completion country first read later is a victim from that hour",
            vec![(10..=20).map(|h| at(h, whole(vec![if h < 20 { raid(5, 1, END, LOST) } else { completed.clone() }]), ger)).collect()],
            vec![ep(5, 1, 1, &[(10, "GER", ENDED), (20, "FRA", DONE)]).done(10, "ENG")],
        ),
        (
            "a phase going back in place is another raid",
            vec![vec![at(10, whole(vec![won(7, 1)]), fra), at(11, whole(vec![prep(7, 1)]), fra)]],
            vec![ep(7, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10), ep(7, 2, 1, &[(11, "FRA", SEEN)])],
        ),
        (
            "R4-S1: after an unread hour a raid resumes only on a matching identity; an unread target is no match",
            vec![vec![at(10, whole(vec![raid(5, 1, END, NONE)]), fra), at(11, Raids::Unread(vec![raid(5, 1, END, NONE)]), fra), At { targeted: false, ..at(12, whole(vec![won(5, 1)]), fra) }]],
            vec![ep(5, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").unknown_outcome(), ep(5, 2, 1, &[(12, "-", ENDED)]).untargeted().typed(None).done(12, "ENG").won(12)],
        ),
        (
            "R4-S2: a launch belongs to the raid that was seen launched, never to the key's next raid",
            vec![vec![at(10, whole(vec![raid(7, 1, FLYING, NONE)]), fra), at(11, whole(vec![prep(7, 2)]), fra), at(12, whole(vec![won(7, 2)]), fra)]],
            vec![ep(7, 1, 1, &[(10, "FRA", SEEN)]).launched(10), ep(7, 2, 2, &[(11, "FRA", SEEN), (12, "FRA", ENDED)]).done(12, "GER").won(12)],
        ),
        (
            "R4-S4: the end tag is the observed end's until the native end date is read",
            vec![(4..=20).map(|h| At { slot1: if h < 10 { b"GER" } else { b"DDR" }, ..at(h, whole(match h { 4..=9 => vec![], 10..=19 => vec![won(5, 1)], _ => vec![dated(5, 5)] }), fra) }).collect()],
            vec![ep(5, 1, 1, &[(10, "FRA", ENDED)]).done(10, "DDR").native(20, 5, "GER").won(10)],
        ),
        (
            "R4-S5: a controller read from a partly read province table is the victim",
            vec![vec![at(10, whole(vec![prep(5, 1)]), fra), At { provinces_part: true, ..at(11, whole(vec![won(5, 1)]), fra) }]],
            vec![ep(5, 1, 1, &[(10, "FRA", SEEN), (11, "FRA", ENDED)]).done(11, "ENG").won(11)],
        ),
        (
            "R4-S11: a known outcome that changes under the same identity is the same raid",
            vec![vec![at(10, whole(vec![won(7, 1)]), fra), at(11, whole(vec![raid(7, 1, END, CRITICAL)]), fra)]],
            vec![ep(7, 1, 1, &[(10, "FRA", ENDED)]).done(10, "ENG").won(10)],
        ),
        (
            "R4-S12: a truncated read without the key is no disappearance",
            vec![vec![at(10, whole(vec![prep(7, 1)]), ger), at(11, Raids::Part(vec![]), ger)]],
            vec![ep(7, 1, 1, &[(10, "GER", SEEN)])],
        ),
        (
            "R4-S12: an absence read whole after an unread hour is a disappearance by that hour",
            vec![vec![at(10, whole(vec![prep(7, 1)]), ger), at(11, Raids::Unread(vec![]), ger), at(12, whole(vec![]), ger)]],
            vec![ep(7, 1, 1, &[(10, "GER", SEEN)]).gone(12, false)],
        ),
    ];
    let mut failed = Vec::new();
    for (name, stretches, want) in cases {
        let segments = recording(&stretches);
        let full = load_product(&segments, "1", None, None);
        let shown: Vec<Seen> = full.episodes.iter().map(seen).collect();
        if shown != want {
            failed.push(format!("{name}: full reply\n  got  {shown:?}\n  want {want:?}"));
        }
        let hours: Vec<i32> = stretches.iter().flatten().map(|at| at.hour).collect();
        for &to in &hours {
            let through = load_product(&segments, "1", None, Some(to));
            failed.extend(malformed(&through).into_iter().map(|fault| format!("{name}: the reply through {to} is malformed: {fault}")));
            let actual = through.episodes;
            let expected: Vec<RaidEpisode> = full.episodes.iter().filter_map(|episode| episode_at(episode, to)).collect();
            if actual != expected {
                failed.push(format!("{name}: the reply through {to} is not the full reply at {to}\n  got  {actual:?}\n  want {expected:?}"));
            }
        }
        let clip = |e: &RaidEpisode| (e.id.clone(), e.raider_slot, e.succeeded_hour, e.ends.last().and_then(|end| end.raider.clone()), e.victims.last().and_then(|v| v.tag.clone()));
        for &from in &hours {
            let ranged = load_product(&segments, "1", Some(from), None);
            failed.extend(malformed(&ranged).into_iter().map(|fault| format!("{name}: the reply from {from} is malformed: {fault}")));
            let got: Vec<_> = ranged.episodes.iter().map(clip).collect();
            let expected: Vec<_> = full.episodes.iter().filter(|e| e.last_hour >= from).map(clip).collect();
            if got != expected {
                failed.push(format!("{name}: the reply from {from} is not the full reply clipped there\n  got  {got:?}\n  want {expected:?}"));
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn a_narrow_range_counts_only_its_own_raids_and_raids_are_available_only_once_one_was_recorded() {
    let scope = RaidScope { province: 10, ..Default::default() };
    let countries = vec![(10, 1, b"ENG".as_slice()), (20, 2, b"GER".as_slice())];
    let frames = (8..=14)
        .map(|hour| match hour {
            8 | 9 | 11 => make_frame(1, hour, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![])), vec![]),
            _ => make_frame(1, hour, countries.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid(7, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS)])), vec![target_row(7, scope.clone(), 40)]),
        })
        .collect();
    let segments = [records(1, frames)];
    let icons = |reply: &super::RaidLogReply| reply.warnings.iter().filter(|w| w.starts_with("raid icons are unavailable")).cloned().collect::<Vec<_>>();
    assert_eq!(icons(&load_product(&segments, "1", None, None)).iter().map(|w| w.ends_with("(count: 2)")).collect::<Vec<_>>(), [true]);
    assert_eq!(icons(&load_product(&segments, "1", Some(13), None)).iter().map(|w| w.ends_with("(count: 1)")).collect::<Vec<_>>(), [true], "a raid ended before the range is not counted");
    assert_eq!(load_product(&segments, "1", None, Some(9)).available, None, "no raid was recorded by hour 9");
    assert_eq!(load_product(&segments, "1", None, Some(10)).available, Some(true));

    let untagged = vec![(20, 2, b"GER".as_slice())];
    let frames = (10..=11).map(|hour| make_frame(1, hour, untagged.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)])), vec![target_row(7, scope.clone(), 40)])).collect();
    let chosen = load_with(&[records(1, frames)], RaidLogParams { branch_id: "1".into(), from: None, to: None, countries: vec![Country::Slot(1)], catalogue: false });
    assert_eq!(chosen.episodes.len(), 1, "R4-S13: a raid by a chosen slot is kept though no tag names its raider");
}
