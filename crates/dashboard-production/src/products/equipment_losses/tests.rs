use super::coverage::{CountryCells, Coverage, DestructionCoverageSpan, Held, RunCells, UnknownReason, UnknownSpan, classify, classify_historical};
use super::*;
use replay_data::rows::{EquipmentLossCoverageRow as Row, Recorded};

const LEGACY_PARTIAL_TEXT: &str = "equipment destruction first/branch interval is partial";

fn run(drain: &str, unread: u8) -> Option<RunCells> {
    Some(RunCells {
        drain: Some(drain.to_owned()),
        drain_state: drain.is_empty().then_some(Row::DRAIN_STATE_COMPLETE),
        unread: Some(unread),
        counted_from: Some(10),
        ..RunCells::default()
    })
}

fn legacy_run(drain: &str, unread: u8) -> Option<RunCells> {
    Some(RunCells { drain: Some(drain.to_owned()), unread: Some(unread), counted_from: Some(10), ..RunCells::default() })
}

#[test]
fn a_totals_key_names_its_source_and_design() {
    let (low, high) = identity_range(7, 300);
    let key = low | u128::from(EquipmentLossTotalRow::DESTRUCTION + EquipmentDestructionCause::Attrition as u8) << 33 | 1 << 32 | 42;
    assert!(low <= key && key <= high);
    assert_eq!(leaf_of(key), Some((LossSource::Destruction(Cause(EquipmentDestructionCause::Attrition)), Some(42))));
    assert_eq!(leaf_of(low | 4 << 33), Some((LossSource::Recovered, None)));
    assert_eq!(leaf_of(low | u128::from(EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::SUNK_KIND) << 33), Some((LossSource::LendLeaseSunk, None)));
    assert_eq!(leaf_of(low | 20 << 33), None);
}

#[test]
fn combat_log_mode_shows_the_log_air_and_mixed_causes_and_destruction_mode_every_drain() {
    let log = LossSource::shown(LandSource::CombatLog);
    assert!(log.contains(&LossSource::CombatLog(0)) && log.contains(&LossSource::Recovered));
    assert!(log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::AirRaid))));
    assert!(log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::Unknown))));
    assert!(!log.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::LandCombat))));
    let drains = LossSource::shown(LandSource::Destruction);
    assert!(drains.contains(&LossSource::Destruction(Cause(EquipmentDestructionCause::LandCombat))));
    assert!(!drains.contains(&LossSource::CombatLog(0)));
    assert!(drains.contains(&LossSource::SunkShips));
}

#[test]
fn drain_reasons_become_one_span_each_from_their_first_hour_after_counting_starts() {
    let held = Held {
        run: vec![
            (0, 9, Some(RunCells { drain: Some(Row::DRAIN_NOT_READ.to_owned()), drain_state: Some(Row::DRAIN_STATE_NOT_READ), ..RunCells::default() })),
            (10, 19, run("", 0)),
            (20, 24, Some(RunCells { drain: Some(Row::DRAIN_NOT_READ.to_owned()), drain_state: Some(Row::DRAIN_STATE_NOT_READ), unread: Some(0), counted_from: Some(10), ..RunCells::default() })),
            (25, 29, None),
            (
                30,
                39,
                Some(RunCells {
                    drain: Some(format!("{}; {}", Row::DRAIN_OWNER_UNKNOWN, Row::DRAIN_SUPPLEMENTAL_NOT_RECORDED)),
                    drain_state: Some(Row::DRAIN_STATE_OWNER_UNKNOWN | Row::DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE),
                    unread: Some(0),
                    counted_from: Some(10),
                    ..RunCells::default()
                }),
            ),
            (40, 49, run("", 0)),
        ],
        ..Held::default()
    };
    assert_eq!(held.counted_from(), Some(10));
    let spans = held.destruction_unknown(99);
    let kinds: Vec<(UnknownReason, i32, i32)> = spans.iter().map(|span| (span.reason, span.first_hour, span.last_hour)).collect();
    assert!(kinds.contains(&(UnknownReason::DestructionGap, 20, 99)), "{kinds:?}");
    assert!(kinds.contains(&(UnknownReason::DestructionGap, 25, 99)), "the hours the tables were not available: {kinds:?}");
    assert!(kinds.contains(&(UnknownReason::DestructionUnresolved, 30, 99)));
    assert!(kinds.contains(&(UnknownReason::DestructionWithheld, 30, 99)));
    assert_eq!(spans.len(), 4, "a startup hour before counting claims nothing: {kinds:?}");
    let coverage = held.destruction_coverage();
    assert_eq!(coverage.first().map(|span| (span.first_hour, span.reason.clone())), Some((10, None)));
    assert_eq!(coverage.iter().filter(|span| span.reason.is_none()).count(), 2);
}

#[test]
fn a_records_unread_hours_and_unavailable_hours_are_unknown_never_zero() {
    let held = Held {
        run: vec![(0, 4, run("", 0)), (5, 7, run("", Row::UNREAD_SHIPS)), (8, 9, run("", Row::UNREAD_SHIPS | Row::UNREAD_CONVOYS)), (10, 12, None), (13, 20, run("", 0))],
        ..Held::default()
    };
    let ships: Vec<(i32, i32, bool)> = held.unread(Row::UNREAD_SHIPS, 20).iter().map(|span| (span.first_hour, span.last_hour, span.detail.is_none())).collect();
    assert_eq!(ships, vec![(5, 9, true), (10, 12, false)]);
    let convoys: Vec<(i32, i32)> = held.unread(Row::UNREAD_CONVOYS, 20).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(convoys, vec![(8, 9), (10, 12)]);
}

#[test]
fn each_ended_exchange_is_a_span_from_the_hour_its_batch_is_lost() {
    let country = |ended: u32, from: Option<i32>| Some(CountryCells { exchanges_ended: Some(ended), ended_from: from, ..CountryCells::default() });
    let held = Held { country: vec![(0, 9, country(0, None)), (10, 19, country(2, Some(8))), (20, 29, country(3, Some(15)))], ..Held::default() };
    let spans: Vec<(i32, i32)> = held.ended(40).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(spans, vec![(8, 40), (8, 40), (15, 40)]);
}

#[test]
fn land_coverage_follows_the_identitys_row_and_its_absence_is_unknown() {
    let country = |land: u8, ungrouped: u32| Some(CountryCells { land: Some(land), ungrouped: Some(ungrouped), ..CountryCells::default() });
    let held = Held { country: vec![(0, 4, country(Row::LOGGED, 0)), (5, 9, country(Row::LOGGED, 0)), (10, 14, country(Row::PARTIAL, 3)), (15, 19, None)], ..Held::default() };
    let land: Vec<(i32, i32, Coverage)> = held.land().iter().map(|span| (span.first_hour, span.last_hour, span.coverage)).collect();
    assert_eq!(land, vec![(0, 9, Coverage::Logged), (10, 14, Coverage::Partial { ungrouped: 3 }), (15, 19, Coverage::Unknown)]);
    assert_eq!(classify(Row::DRAIN_LOST), UnknownReason::DestructionGap);
    assert_eq!(classify("The recording's own reason"), UnknownReason::DestructionWithheld);
    assert_eq!(classify(LEGACY_PARTIAL_TEXT), UnknownReason::DestructionWithheld);
    assert_eq!(classify_historical(LEGACY_PARTIAL_TEXT), UnknownReason::DestructionGap);
}

#[test]
fn a_valid_at_hour_request_accepts_slot_or_tag_and_rejects_bad_branch() {
    assert_eq!(EquipmentLosses::validate(&EquipmentLossesParams { branch_id: "1".into(), country: Country::Slot(7), hour: 120, land_source: LandSource::Destruction }), Ok(()));
    assert!(EquipmentLosses::validate(&EquipmentLossesParams { branch_id: "bad".into(), country: Country::Slot(7), hour: 120, land_source: LandSource::Destruction }).is_err());
}

#[test]
fn startup_and_branch_partial_intervals_stay_distinct_from_withheld_reads() {
    let held = Held {
        run: vec![
            (
                0,
                9,
                Some(RunCells {
                    drain: Some("Reworded startup interval".to_owned()),
                    drain_state: Some(Row::DRAIN_STATE_PARTIAL_INTERVAL),
                    unread: Some(0),
                    ..RunCells::default()
                }),
            ),
            (
                10,
                19,
                Some(RunCells {
                    drain: Some(String::new()),
                    drain_state: Some(Row::DRAIN_STATE_COMPLETE),
                    unread: Some(0),
                    counted_from: Some(10),
                    ..RunCells::default()
                }),
            ),
            (
                20,
                24,
                Some(RunCells {
                    drain: Some("Reworded branch interval".to_owned()),
                    drain_state: Some(Row::DRAIN_STATE_PARTIAL_INTERVAL),
                    unread: Some(0),
                    counted_from: Some(10),
                    incomplete_from: Some(20),
                    ..RunCells::default()
                }),
            ),
            (
                25,
                29,
                Some(RunCells {
                    drain: Some(LEGACY_PARTIAL_TEXT.to_owned()),
                    drain_state: Some(Row::DRAIN_STATE_WITHHELD),
                    unread: Some(0),
                    counted_from: Some(10),
                    withheld_from: Some(25),
                    incomplete_from: Some(20),
                    ..RunCells::default()
                }),
            ),
        ],
        ..Held::default()
    };
    let spans = held.destruction_unknown(40);
    assert!(spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionGap && span.detail.as_deref() == Some("Reworded branch interval")), "{spans:?}");
    assert!(!spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionWithheld), "{spans:?}");
    assert!(spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionWithheld && span.detail.as_deref() == Some(LEGACY_PARTIAL_TEXT)), "{spans:?}");
    assert!(!spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionGap), "{spans:?}");

    let historical = Held {
        run: vec![
            (0, 9, Some(RunCells { drain: Some(LEGACY_PARTIAL_TEXT.to_owned()), ..RunCells::default() })),
            (10, 19, legacy_run("", 0)),
            (20, 24, legacy_run(LEGACY_PARTIAL_TEXT, 0)),
            (25, 29, Some(RunCells { drain: Some("drain overflowed".to_owned()), unread: Some(0), counted_from: Some(10), withheld_from: Some(25), ..RunCells::default() })),
        ],
        legacy_partial_text: true,
        ..Held::default()
    };
    let old_spans = historical.destruction_unknown(40);
    assert!(old_spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionGap), "{old_spans:?}");
    assert!(!old_spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
    assert!(old_spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
}

#[test]
fn historical_and_current_recordings_distinguish_startup_partial_and_withheld_hours() {
    use replay_journal::{columnar_file::{BlockEnd, FileWriter}, fixtures, synthetic::{Input, Sample}};
    use replay_settings::compression::Settings;
    use replay_recording::{columnar_frames::Recording, schema::{columns_at, schema}};
    use replay_columnar::{
        container::{SectionKey, Tags},
        envelope::{self, Identity},
        lane::{Clock, ClockHour},
        manifest::Status,
        state::Cell as StoreCell,
        store::Store,
        unit::{self, Encoder},
    };
    use replay_data::{
        identity::{CaptureMode as Mode, Identity as Game, IdentityCodec, SampleBoundary, SessionStart, Verification},
        rows::{CountryLinkRow, CountryRow, EquipmentDestructionCoverage, EquipmentDestructionRow, compatibility::EQUIPMENT_DESTRUCTION_STARTUP_FORMAT},
    };

    let current_format = EQUIPMENT_DESTRUCTION_STARTUP_FORMAT;
    let countries = [CountryRow { id: 1, canonical_slot: 1, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 1, canonical_slot: 1 }];
    let variant = (7_u64 << 32) | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
    let loss = [EquipmentDestructionRow {
        country: 1,
        cause: EquipmentDestructionCause::LandCombat,
        variant,
        amount_raw: 5,
        coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
        unavailable_reason: Recorded::Value(String::new()),
    }];
    let startup_partial = [EquipmentDestructionRow {
        coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
        unavailable_reason: Recorded::Value("reworded startup interval".to_owned()),
        ..Default::default()
    }];
    let branch_partial = [EquipmentDestructionRow {
        coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
        unavailable_reason: Recorded::Value("reworded branch interval".to_owned()),
        ..Default::default()
    }];
    let (current_bytes, _) = fixtures::written_at(recorder_derive::writers_within, current_format, b"current-coverage", &[(1, 100, 1)], &[TableId::EquipmentLossCoverage], |synthetic, _, ordinal| {
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let input = match ordinal {
            0 => Input::Snapshot { rows: &startup_partial, partial: None, read: true, sample: Sample::FromReadBit },
            1 => Input::Snapshot { rows: &loss, partial: None, read: true, sample: Sample::FromReadBit },
            2 => Input::Snapshot { rows: &branch_partial, partial: None, read: true, sample: Sample::FromReadBit },
            3 => Input::Snapshot { rows: &loss, partial: None, read: true, sample: Sample::FromReadBit },
            _ => Input::Unavailable { reason: "drain overflowed", read: true, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::EquipmentDestruction, input).unwrap();
    });
    let current = Recording::open(current_bytes).unwrap();
    assert_eq!(current.format(), current_format);
    let mut held = Held::default();
    held.read(&current, 0, coverage_key(1, 1), RUN_KEY, 100, 105).unwrap();
    assert!(!held.legacy_partial_text);
    assert_eq!(held.counted_from(), Some(101));
    let spans = held.destruction_unknown(105);
    assert!(spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionGap && span.detail.as_deref() == Some("reworded branch interval")), "{spans:?}");
    assert!(!spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionWithheld), "{spans:?}");
    assert!(spans.iter().any(|span| span.first_hour == 104 && span.reason == UnknownReason::DestructionWithheld && span.detail.as_deref() == Some("drain overflowed")), "{spans:?}");

    let old_format = EQUIPMENT_DESTRUCTION_STARTUP_FORMAT - 1;
    let old_columns = columns_at(old_format).unwrap();
    let full_schema = schema().unwrap();
    assert_eq!(old_columns[TableId::EquipmentLossCoverage.index()], 15);
    let mut old_kinds = full_schema[..old_columns.len()].to_vec();
    for (kinds, &count) in old_kinds.iter_mut().zip(&old_columns) {
        kinds.truncate(count);
    }
    let identity = Identity {
        schema: envelope::schema_digest_prefix(&full_schema[..old_columns.len()], &old_columns).unwrap(),
        campaign: [7; 16].to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let unit = unit::Settings { restore_blocks: 1, ..unit::Settings::at(3) };
    let mut file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity, old_columns.len(), Settings { level: unit.level }, None).unwrap();
    let mut store = Store::new(old_kinds.clone(), replay_recording::schema::keeps_last()[..old_columns.len()].to_vec());
    store.declare_logs(&replay_data::rows::logs()[..old_columns.len()]).unwrap();
    store.attach(Encoder::new(&old_kinds, unit).unwrap());
    file.start_run(1, envelope::epoch::START, 100).unwrap();
    store.begin_run(100);
    let mut clock = Clock::default();
    let mut strings = Vec::new();
    let mut string_count = 0_u64;
    let table = TableId::EquipmentLossCoverage.index();
    for ordinal in 0..6_i32 {
        let hour = 100 + ordinal;
        let (drain, counted, withheld, incomplete) = match ordinal {
            0 => (LEGACY_PARTIAL_TEXT, None, None, None),
            1 => ("", Some(101), None, None),
            2 => (LEGACY_PARTIAL_TEXT, Some(101), None, Some(102)),
            3 => ("", Some(101), None, Some(102)),
            _ => ("drain overflowed", Some(101), Some(104), Some(102)),
        };
        let mut appends: Vec<&[u8]> = Vec::new();
        let before = store.strings().next();
        let drain_id = store.intern(drain.as_bytes()).unwrap();
        if store.strings().next() > before {
            appends.push(drain.as_bytes());
        }
        for text in &appends {
            unit::put_varint(&mut strings, text.len() as u64);
            strings.extend_from_slice(text);
            string_count += 1;
        }
        store.begin_hour(hour, &[]).unwrap();
        store.status(table, Status::Complete, true, Some(hour), true).unwrap();
        {
            let mut changes = store.table(table).unwrap();
            let slot = changes.slot(RUN_KEY).unwrap_or_else(|| changes.insert(RUN_KEY).unwrap());
            changes.write(slot, 4, StoreCell::Bytes(drain_id)).unwrap();
            changes.write(slot, 5, StoreCell::U8(0)).unwrap();
            changes.write(slot, 6, StoreCell::OptionalI32(counted)).unwrap();
            changes.write(slot, 7, StoreCell::OptionalI32(withheld)).unwrap();
            changes.write(slot, 8, StoreCell::OptionalI32(incomplete)).unwrap();
            changes.write(slot, 13, StoreCell::U32(0)).unwrap();
            changes.write(slot, 14, StoreCell::U32(0)).unwrap();
        }
        store.settle();
        clock.hour(&ClockHour { flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0 });
        store.end_hour(&mut file).unwrap();
    }
    store.end_run(&mut file).unwrap();
    let mut string_payload = Vec::new();
    unit::put_varint(&mut string_payload, string_count);
    string_payload.extend_from_slice(&strings);
    file.end_block(BlockEnd { clock: &clock.finish(), strings: (&string_payload, 0), summaries: Vec::new(), span: Vec::new() }).unwrap();
    file.finish_run().unwrap();
    let game = Game {
        game_version: "synthetic".into(),
        exe_hash: [1; 32],
        exe_size: 1,
        vanilla_hash: [2; 32],
        dlc: Vec::new(),
        mods: Vec::new(),
        coverage_policy: 1,
        language: "synthetic".into(),
        recorder_version: "synthetic".into(),
        collector_hash: [3; 32],
        verification: Verification::Unchecked,
    };
    let mut session = Vec::new();
    SessionStart { id: 1, process_id: 1, process_start_time: 1, qpc_frequency: 1, sample_boundary: SampleBoundary::EndOfHour, capture_mode: Mode::Full, identity: game }.encode(&mut session).unwrap();
    let capture = replay_recording::capture::Capture {
        campaign_id: b"historical-coverage".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
        sessions: vec![session],
        run_sessions: vec![0],
        stop: None,
        format: old_format,
        journaled: 0,
        controls: Vec::new(),
    };
    let extent = file.container().put_zstd(envelope::Kind::Controls, Tags::default(), &capture.write().unwrap(), 3).unwrap();
    file.container().set_open(SectionKey::RECORDS, &[extent]).unwrap();
    let old_bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
    let old = Recording::open(old_bytes).unwrap();
    assert_eq!(old.format(), old_format);
    let mut old_held = Held::default();
    old_held.read(&old, 0, coverage_key(1, 1), RUN_KEY, 100, 105).unwrap();
    assert!(old_held.legacy_partial_text);
    assert!(old_held.run.iter().all(|(_, _, cells)| cells.as_ref().is_none_or(|cells| cells.drain_state.is_none())));
    assert_eq!(old_held.counted_from(), Some(101));
    let old_spans = old_held.destruction_unknown(105);
    assert!(old_spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionGap && span.detail.as_deref() == Some(LEGACY_PARTIAL_TEXT)), "{old_spans:?}");
    assert!(!old_spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
    assert!(old_spans.iter().any(|span| span.first_hour == 104 && span.reason == UnknownReason::DestructionWithheld && span.detail.as_deref() == Some("drain overflowed")), "{old_spans:?}");
}

#[test]
fn current_format_unrecorded_drain_state_with_empty_text_is_never_complete() {
    let held = Held {
        run: vec![(
            10,
            15,
            Some(RunCells {
                drain: Some(String::new()),
                drain_state: None,
                unread: Some(0),
                counted_from: Some(10),
                ..RunCells::default()
            }),
        )],
        ..Held::default()
    };
    let coverage = held.destruction_coverage();
    assert_eq!(coverage.len(), 1);
    assert!(coverage[0].reason.is_some(), "unrecorded drain_state on current format must never appear complete: {coverage:?}");
    let spans = held.destruction_unknown(15);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].reason, UnknownReason::DestructionGap);
    assert_eq!(spans[0].first_hour, 10);
}

#[test]
fn recording_break_keeps_each_runs_counting_boundary_and_partial_startup_separate() {
    use replay_recording::columnar_frames::Recording;
    use replay_journal::{fixtures, synthetic::{Input, Sample}};
    use replay_data::rows::{CountryLinkRow, CountryRow, EquipmentDestructionCoverage, EquipmentDestructionRow, compatibility::EQUIPMENT_DESTRUCTION_STARTUP_FORMAT};

    let countries = [CountryRow { id: 1, canonical_slot: 1, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 1, canonical_slot: 1 }];
    let variant = (7_u64 << 32) | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
    let loss = [EquipmentDestructionRow {
        country: 1,
        cause: EquipmentDestructionCause::LandCombat,
        variant,
        amount_raw: 5,
        coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
        unavailable_reason: Recorded::Value(String::new()),
    }];
    let partial = [EquipmentDestructionRow {
        coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
        unavailable_reason: Recorded::Value("startup partial".to_owned()),
        ..Default::default()
    }];
    let (bytes, _) = fixtures::written_at(
        recorder_derive::writers_within,
        EQUIPMENT_DESTRUCTION_STARTUP_FORMAT,
        b"break-coverage",
        &[(1, 100, 1), (1, 200, 1)],
        &[TableId::EquipmentLossCoverage],
        |synthetic, run, ordinal| {
            synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            let input = match (run, ordinal) {
                (_, 0) => Input::Snapshot { rows: &partial, partial: None, read: true, sample: Sample::FromReadBit },
                (0, 3) => Input::Snapshot { rows: &partial, partial: None, read: true, sample: Sample::FromReadBit },
                _ => Input::Snapshot { rows: &loss, partial: None, read: true, sample: Sample::FromReadBit },
            };
            synthetic.table(TableId::EquipmentDestruction, input).unwrap();
        },
    );
    let recording = Recording::open(bytes).unwrap();
    let mut held = Held::default();
    held.read(&recording, 0, coverage_key(1, 1), RUN_KEY, 100, 105).unwrap();
    held.read(&recording, 1, coverage_key(1, 1), RUN_KEY, 200, 205).unwrap();
    let runs = held.destruction_runs();
    assert_eq!(runs.last().map(|run| (run.first_hour, run.counted_from)), Some((200, Some(201))));
    assert_eq!(held.counted_from(), Some(201));

    let coverage = held.destruction_coverage();
    assert!(!coverage.iter().any(|span| span.first_hour <= 200 && 200 <= span.last_hour), "run 1 startup partial must not be reported as an incomplete span: {coverage:?}");
    assert!(coverage.iter().any(|span| span.first_hour == 201 && span.last_hour == 205 && span.reason.is_none()), "{coverage:?}");

    let spans = held.destruction_unknown(205);
    assert_eq!(spans.len(), 1, "only run 0's gap should appear: {spans:?}");
    assert_eq!(spans[0].first_hour, 103);
    assert_eq!(spans[0].last_hour, 105, "run 0's unknown span must not leak into run 1");
    assert_eq!(spans[0].reason, UnknownReason::DestructionGap);
}

#[test]
fn withheld_before_counting_publishes_recorded_refusal_without_fabricating_truncation() {
    let held = Held {
        run: vec![(
            100,
            105,
            Some(RunCells {
                drain: Some("equipment destruction hooks unavailable".to_owned()),
                drain_state: Some(Row::DRAIN_STATE_WITHHELD),
                unread: Some(0),
                counted_from: None,
                withheld_from: Some(100),
                ..RunCells::default()
            }),
        )],
        ..Held::default()
    };
    assert_eq!(held.counted_from(), None);
    let coverage = held.destruction_coverage();
    assert_eq!(coverage, vec![DestructionCoverageSpan { first_hour: 100, last_hour: 105, reason: Some("equipment destruction hooks unavailable".to_owned()) }]);
    let spans = held.destruction_unknown(105);
    assert_eq!(
        spans,
        vec![UnknownSpan {
            first_hour: 100,
            last_hour: 105,
            reason: UnknownReason::DestructionWithheld,
            detail: Some("equipment destruction hooks unavailable".to_owned()),
        }]
    );

    let silent = Held {
        run: vec![(
            100,
            105,
            Some(RunCells {
                drain: Some(String::new()),
                drain_state: Some(Row::DRAIN_STATE_WITHHELD),
                unread: Some(0),
                counted_from: None,
                withheld_from: Some(100),
                ..RunCells::default()
            }),
        )],
        ..Held::default()
    };
    let fallback = silent.destruction_unknown(105);
    assert_eq!(
        fallback,
        vec![UnknownSpan {
            first_hour: 100,
            last_hour: 105,
            reason: UnknownReason::DestructionWithheld,
            detail: Some(Row::DRAIN_WITHHELD.to_owned()),
        }]
    );
}

#[test]
fn combined_lost_and_unresolved_drain_states_emit_both_unknown_spans() {
    let held = Held {
        run: vec![
            (10, 10, run("", 0)),
            (
                11,
                11,
                Some(RunCells {
                    drain: Some(format!("{}; {}", Row::DRAIN_LOST, Row::DRAIN_OWNER_UNKNOWN)),
                    drain_state: Some(Row::DRAIN_STATE_LOST | Row::DRAIN_STATE_OWNER_UNKNOWN),
                    unread: Some(0),
                    counted_from: Some(10),
                    incomplete_from: Some(11),
                    ..RunCells::default()
                }),
            ),
        ],
        ..Held::default()
    };
    let spans = held.destruction_unknown(15);
    assert_eq!(
        spans,
        vec![
            UnknownSpan {
                first_hour: 11,
                last_hour: 15,
                reason: UnknownReason::DestructionGap,
                detail: Some(Row::DRAIN_LOST.to_owned()),
            },
            UnknownSpan {
                first_hour: 11,
                last_hour: 15,
                reason: UnknownReason::DestructionUnresolved,
                detail: Some(Row::DRAIN_OWNER_UNKNOWN.to_owned()),
            },
        ]
    );
}

#[test]
fn current_format_unrecorded_drain_state_with_nonempty_text_reports_gap_not_guessed_class() {
    let held = Held {
        run: vec![
            (
                10,
                11,
                Some(RunCells {
                    drain: Some("Reworded startup interval".to_owned()),
                    drain_state: None,
                    unread: Some(0),
                    counted_from: Some(10),
                    ..RunCells::default()
                }),
            ),
            (
                12,
                13,
                Some(RunCells {
                    drain: Some(Row::DRAIN_OWNER_UNKNOWN.to_owned()),
                    drain_state: None,
                    unread: Some(0),
                    counted_from: Some(10),
                    ..RunCells::default()
                }),
            ),
            (
                14,
                15,
                Some(RunCells {
                    drain: Some("Reworded refusal".to_owned()),
                    drain_state: None,
                    unread: Some(0),
                    counted_from: Some(10),
                    withheld_from: Some(14),
                    ..RunCells::default()
                }),
            ),
        ],
        ..Held::default()
    };
    let spans = held.destruction_unknown(15);
    assert!(spans.contains(&UnknownSpan { first_hour: 10, last_hour: 15, reason: UnknownReason::DestructionGap, detail: Some("Reworded startup interval".to_owned()) }), "{spans:?}");
    assert!(spans.contains(&UnknownSpan { first_hour: 12, last_hour: 15, reason: UnknownReason::DestructionGap, detail: Some(Row::DRAIN_OWNER_UNKNOWN.to_owned()) }), "{spans:?}");
    assert!(!spans.iter().any(|span| span.reason == UnknownReason::DestructionUnresolved), "absent format-145 drain_state must not guess unresolved from text: {spans:?}");
    assert!(spans.contains(&UnknownSpan { first_hour: 14, last_hour: 15, reason: UnknownReason::DestructionGap, detail: Some("Reworded refusal".to_owned()) }), "{spans:?}");
    assert!(spans.contains(&UnknownSpan { first_hour: 14, last_hour: 15, reason: UnknownReason::DestructionWithheld, detail: Some(Row::DRAIN_WITHHELD.to_owned()) }), "{spans:?}");
    assert_eq!(spans.len(), 4, "{spans:?}");
}

#[test]
fn at_hour_breakdown_gates_startup_withheld_supplemental_gaps_unavailable_totals_and_quiet_through() {
    use dashboard_base::{
        products::breakdown::{BreakdownAmount, BreakdownState},
        test_fixtures::{CAMPAIGN, records, source as base_frame},
    };
    use replay::campaign::Replay;
    use replay_capture_spool::spool::Record;
    use replay_data::rows::{CountryRow, EquipmentDestructionCoverage, EquipmentDestructionRow, Recorded, TableRows};
    use replay_frame::{
        branches::{BranchReason, BranchStart, Parentage},
        frame::TableView,
    };
    use std::sync::{Arc, atomic::AtomicBool};

    let variant = (7_u64 << 32) | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
    let make_frame = |branch: u64, hour: i32, tag: &[u8], rows: Vec<EquipmentDestructionRow>| {
        let mut frame = base_frame(branch, hour, Some(1));
        if let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] {
            countries[0] = CountryRow { tag: tag.to_vec(), ..countries[0].clone() };
        }
        *frame.tables[TableId::EquipmentDestruction.index()] = TableView::Complete(TableRows::EquipmentDestruction(rows));
        frame.metrics.table_ns[TableId::EquipmentDestruction.index()] = 1;
        *frame.tables[TableId::SunkShips.index()] = TableView::Complete(TableRows::SunkShips(vec![replay_data::rows::SunkShipsRow::default()]));
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        *frame.tables[TableId::SunkConvoys.index()] = TableView::Complete(TableRows::SunkConvoys(Vec::new()));
        frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
        *frame.tables[TableId::LendLeaseEquipment.index()] = TableView::Complete(TableRows::LendLeaseEquipment(Vec::new()));
        frame.metrics.table_ns[TableId::LendLeaseEquipment.index()] = 1;
        frame
    };
    let partial = |reason: &str| {
        vec![EquipmentDestructionRow {
            coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
            unavailable_reason: Recorded::Value(reason.to_owned()),
            ..Default::default()
        }]
    };
    let complete_loss = |amount_raw: i64| EquipmentDestructionRow {
        country: 9001,
        cause: EquipmentDestructionCause::LandCombat,
        variant,
        amount_raw,
        coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
        unavailable_reason: Recorded::Value(String::new()),
    };
    let supplemental_marker = |reason: &str| EquipmentDestructionRow {
        coverage: Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable),
        unavailable_reason: Recorded::Value(reason.to_owned()),
        ..Default::default()
    };

    let parent_frames: Vec<_> = (100..=119)
        .map(|hour| match hour {
            100 => make_frame(1, hour, b"GER", partial("startup partial")),
            101 => make_frame(1, hour, b"GER", vec![supplemental_marker("supplemental hooks unavailable"), complete_loss(500_000)]),
            102..=105 => make_frame(1, hour, b"GER", vec![supplemental_marker("supplemental hooks unavailable")]),
            _ => make_frame(1, hour, b"GER", Vec::new()),
        })
        .collect();
    let child_frames: Vec<_> = (120..=130)
        .map(|hour| match hour {
            120 => make_frame(2, hour, b"BBB", partial("child startup partial")),
            121 => make_frame(2, hour, b"BBB", vec![complete_loss(250_000)]),
            _ => make_frame(2, hour, b"BBB", Vec::new()),
        })
        .collect();

    let parent = records(1, parent_frames);
    let mut child = records(2, child_frames);
    child[1] = Record::BranchStart(BranchStart {
        id: 2,
        session: 2,
        loaded_hour: Some(119),
        first_hour: 120,
        parent: Parentage::Probable { branch: 1, hour: 119 },
        reason: BranchReason::LoadSignal,
    });
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[parent, child]).unwrap().into_timelines(&Default::default()).unwrap();
    let branch_id = replay.timelines().expect("timelines").list[0].id;
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "losses-at-hour".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |hour: i32| {
        EquipmentLosses::load(
            &source,
            &EquipmentLossesParams { branch_id: branch_id.to_string(), country: Country::Slot(7), hour, land_source: LandSource::Destruction },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };

    let at_100 = load(100);
    assert_eq!(at_100.from, 100);
    assert_eq!(at_100.breakdown.total.state, BreakdownState::NotRecorded, "startup partial hour never claims complete zero: {:?}", at_100.breakdown);

    let at_101 = load(101);
    assert_eq!(at_101.breakdown.total.state, BreakdownState::Partial, "supplemental-unavailable loss is partial with shares withheld: {:?}", at_101.breakdown);
    assert_eq!(at_101.breakdown.total.amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }));
    assert!(at_101.breakdown.parts.iter().all(|part| part.share.is_none()));

    let at_120 = load(120);
    assert_eq!(at_120.breakdown.total.state, BreakdownState::NotRecorded, "child startup partial hour after tag change GER -> BBB is not recorded");

    let at_121 = load(121);
    assert_eq!(at_121.breakdown.total.state, BreakdownState::Known, "slot 7 continues seamlessly after tag change GER -> BBB: {:?}", at_121.breakdown);
    assert_eq!(at_121.breakdown.total.amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "2".into() }), "250000 / 100000 reduces to exact 5/2");
    assert_eq!(at_121.through, 130, "quiet stretch stands through the segment end in one request");
    assert_eq!(at_121.from, 120, "a count that restarted after the load is dated from its own start, never the branch start");
}

#[test]
fn an_item_with_no_recorded_loss_yet_but_incomplete_coverage_stays_as_missing_never_vanishes() {
    use super::breakdown::loss_amount;
    use dashboard_base::products::breakdown::Amount;
    assert_eq!(loss_amount(Some(0), Some("Some losses for this item were not recorded")), Some(Amount::NotRecorded("Some losses for this item were not recorded".into())));
    assert_eq!(loss_amount(Some(0), None), None, "a complete zero adds no part");
}

#[test]
fn withheld_destruction_and_unavailable_country_hours_never_claim_zero() {
    use dashboard_base::{
        products::breakdown::{BreakdownAmount, BreakdownState},
        test_fixtures::{CAMPAIGN, records, source as base_frame},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{EquipmentDestructionCoverage, EquipmentDestructionRow, Recorded, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let variant = (7_u64 << 32) | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
    let partial = vec![EquipmentDestructionRow {
        coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
        unavailable_reason: Recorded::Value("startup partial".to_owned()),
        ..Default::default()
    }];
    let loss = |amount_raw: i64| {
        vec![EquipmentDestructionRow {
            country: 9001,
            cause: EquipmentDestructionCause::LandCombat,
            variant,
            amount_raw,
            coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
            unavailable_reason: Recorded::Value(String::new()),
        }]
    };
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = base_frame(1, hour, Some(1));
            frame.metrics.table_ns[TableId::EquipmentDestruction.index()] = 1;
            *frame.tables[TableId::SunkShips.index()] = TableView::Complete(TableRows::SunkShips(vec![replay_data::rows::SunkShipsRow::default()]));
            frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
            *frame.tables[TableId::SunkConvoys.index()] = TableView::Complete(TableRows::SunkConvoys(Vec::new()));
            frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
            *frame.tables[TableId::LendLeaseEquipment.index()] = TableView::Complete(TableRows::LendLeaseEquipment(Vec::new()));
            frame.metrics.table_ns[TableId::LendLeaseEquipment.index()] = 1;
            if hour == 103 {
                *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries failed".into() };
            }
            *frame.tables[TableId::EquipmentDestruction.index()] = match hour {
                100 => TableView::Complete(TableRows::EquipmentDestruction(partial.clone())),
                102 => TableView::Complete(TableRows::EquipmentDestruction(loss(200_000))),
                104 => TableView::Complete(TableRows::EquipmentDestruction(loss(300_000))),
                105..=110 => TableView::Unavailable { reason: "drain overflowed".into() },
                _ => TableView::Complete(TableRows::EquipmentDestruction(Vec::new())),
            };
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "losses-withheld".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |hour: i32| {
        EquipmentLosses::load(
            &source,
            &EquipmentLossesParams { branch_id: "1".into(), country: Country::Slot(7), hour, land_source: LandSource::Destruction },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };

    let at_102 = load(102);
    assert_eq!(at_102.breakdown.total.state, BreakdownState::Known);
    assert_eq!(at_102.breakdown.total.amount, Some(BreakdownAmount { numerator: "2".into(), denominator: "1".into() }));
    assert_eq!(at_102.through, 102, "holder unavailable at 103 tightens through to 102");

    let at_103 = load(103);
    assert_eq!(at_103.breakdown.total.state, BreakdownState::NotRecorded);
    assert!(at_103.breakdown.total.reason.as_deref().is_some_and(|reason| reason.contains("Countries")), "{:?}", at_103.breakdown.total);

    let at_104 = load(104);
    assert_eq!(at_104.breakdown.total.state, BreakdownState::Partial, "unavailable Countries at 103 leaves 104 partial: {:?}", at_104.breakdown);
    assert_eq!(at_104.breakdown.total.amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }));
    assert_eq!(at_104.through, 104, "withheld drain at 105 tightens through to 104");

    let at_106 = load(106);
    assert_eq!(at_106.breakdown.total.state, BreakdownState::NotRecorded, "withheld destruction hours never show stale or zero totals: {:?}", at_106.breakdown);
}
