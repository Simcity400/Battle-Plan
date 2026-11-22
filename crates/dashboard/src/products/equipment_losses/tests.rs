use super::coverage::{CountryCells, Coverage, Held, RunCells, UnknownReason, classify, classify_historical};
use super::*;
use replay::lanes::Bin;
use replay_data::rows::{EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentLossCoverageRow as Row};

fn run(drain: &str, unread: u8) -> Option<RunCells> {
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
fn a_bin_gives_its_first_value_at_its_first_hour_and_its_last_at_its_last() {
    let bin = |first: i64, last: i64| Bin { hours: 6, first, last, min: first.min(last), max: first.max(last), ..Bin::default() };
    let bins = [bin(0, 5), Bin::default(), bin(5, 5), bin(5, 9)];
    assert_eq!(bin_points(&bins, 0, 6, 0, 23), vec![(0, 0), (5, 5), (23, 9)]);
    assert_eq!(bin_points(&bins, 0, 6, 2, 20), vec![(2, 0), (5, 5), (20, 9)]);
}

#[test]
fn drain_reasons_become_one_span_each_from_their_first_hour_after_counting_starts() {
    let held = Held {
        run: vec![
            (0, 9, Some(RunCells { drain: Some(Row::DRAIN_NOT_READ.to_owned()), ..RunCells::default() })),
            (10, 19, run("", 0)),
            (20, 24, run(Row::DRAIN_NOT_READ, 0)),
            (25, 29, None),
            (30, 39, run(&format!("{}; {}", Row::DRAIN_OWNER_UNKNOWN, Row::DRAIN_SUPPLEMENTAL_NOT_RECORDED), 0)),
            (40, 49, run("", 0)),
        ],
        country: Vec::new(),
        legacy_partial_text: false,
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
        country: Vec::new(),
        legacy_partial_text: false,
    };
    let ships: Vec<(i32, i32, bool)> = held.unread(Row::UNREAD_SHIPS, 20).iter().map(|span| (span.first_hour, span.last_hour, span.detail.is_none())).collect();
    assert_eq!(ships, vec![(5, 9, true), (10, 12, false)]);
    let convoys: Vec<(i32, i32)> = held.unread(Row::UNREAD_CONVOYS, 20).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(convoys, vec![(8, 9), (10, 12)]);
}

#[test]
fn each_ended_exchange_is_a_span_from_the_hour_its_batch_is_lost() {
    let country = |ended: u32, from: Option<i32>| Some(CountryCells { exchanges_ended: Some(ended), ended_from: from, ..CountryCells::default() });
    let held = Held { run: Vec::new(), country: vec![(0, 9, country(0, None)), (10, 19, country(2, Some(8))), (20, 29, country(3, Some(15)))], legacy_partial_text: false };
    let spans: Vec<(i32, i32)> = held.ended(40).iter().map(|span| (span.first_hour, span.last_hour)).collect();
    assert_eq!(spans, vec![(8, 40), (8, 40), (15, 40)]);
}

#[test]
fn land_coverage_follows_the_identitys_row_and_its_absence_is_unknown() {
    let country = |land: u8, ungrouped: u32| Some(CountryCells { land: Some(land), ungrouped: Some(ungrouped), ..CountryCells::default() });
    let held = Held { run: Vec::new(), country: vec![(0, 4, country(Row::LOGGED, 0)), (5, 9, country(Row::LOGGED, 0)), (10, 14, country(Row::PARTIAL, 3)), (15, 19, None)], legacy_partial_text: false };
    let land: Vec<(i32, i32, Coverage)> = held.land().iter().map(|span| (span.first_hour, span.last_hour, span.coverage)).collect();
    assert_eq!(land, vec![(0, 9, Coverage::Logged), (10, 14, Coverage::Partial { ungrouped: 3 }), (15, 19, Coverage::Unknown)]);
    assert_eq!(classify(Row::DRAIN_LOST), UnknownReason::DestructionGap);
    assert_eq!(classify("The recording's own reason"), UnknownReason::DestructionWithheld);
    assert_eq!(classify(EQUIPMENT_DESTRUCTION_PARTIAL_REASON), UnknownReason::DestructionWithheld);
    assert_eq!(classify_historical(EQUIPMENT_DESTRUCTION_PARTIAL_REASON), UnknownReason::DestructionGap);
}

#[test]
fn a_request_reading_1500_exact_hours_is_valid() {
    let params = LossesParams { branch_id: "1".into(), country: "GER".into(), land_source: LandSource::default(), identity: None, from: 0, to: 2_000, width: 64, reads: (0..1_500).collect() };
    assert_eq!(EquipmentLosses::validate(&params), Ok(()));
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
                    drain: Some(EQUIPMENT_DESTRUCTION_PARTIAL_REASON.to_owned()),
                    drain_state: Some(Row::DRAIN_STATE_WITHHELD),
                    unread: Some(0),
                    counted_from: Some(10),
                    withheld_from: Some(25),
                    incomplete_from: Some(20),
                    ..RunCells::default()
                }),
            ),
        ],
        country: Vec::new(),
        legacy_partial_text: false,
    };
    let spans = held.destruction_unknown(40);
    assert!(spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionGap && span.detail.as_deref() == Some("Reworded branch interval")), "{spans:?}");
    assert!(!spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionWithheld), "{spans:?}");
    assert!(spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionWithheld && span.detail.as_deref() == Some(EQUIPMENT_DESTRUCTION_PARTIAL_REASON)), "{spans:?}");
    assert!(!spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionGap), "{spans:?}");

    let historical = Held {
        run: vec![
            (0, 9, Some(RunCells { drain: Some(EQUIPMENT_DESTRUCTION_PARTIAL_REASON.to_owned()), ..RunCells::default() })),
            (10, 19, run("", 0)),
            (20, 24, run(EQUIPMENT_DESTRUCTION_PARTIAL_REASON, 0)),
            (25, 29, Some(RunCells { drain: Some("drain overflowed".to_owned()), unread: Some(0), counted_from: Some(10), withheld_from: Some(25), ..RunCells::default() })),
        ],
        country: Vec::new(),
        legacy_partial_text: true,
    };
    let old_spans = historical.destruction_unknown(40);
    assert!(old_spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionGap), "{old_spans:?}");
    assert!(!old_spans.iter().any(|span| span.first_hour == 20 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
    assert!(old_spans.iter().any(|span| span.first_hour == 25 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
}

#[test]
fn historical_and_current_recordings_distinguish_startup_partial_and_withheld_hours() {
    use replay::{
        columnar_file::{BlockEnd, FileWriter, Settings},
        columnar_frames::{
            Recording, columns_at, fixtures, schema,
            synthetic::{Input, Sample},
        },
    };
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
    let (current_bytes, _) = fixtures::written_at(current_format, b"current-coverage", &[(1, 100, 1)], &[TableId::EquipmentLossCoverage], |synthetic, _, ordinal| {
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
    assert_eq!(held.incomplete_from(), Some(102));
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
    let mut store = Store::new(old_kinds.clone(), replay::columnar_frames::keeps_last()[..old_columns.len()].to_vec());
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
            0 => (EQUIPMENT_DESTRUCTION_PARTIAL_REASON, None, None, None),
            1 => ("", Some(101), None, None),
            2 => (EQUIPMENT_DESTRUCTION_PARTIAL_REASON, Some(101), None, Some(102)),
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
    let capture = replay::columnar_file::Capture {
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
    assert!(old_spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionGap && span.detail.as_deref() == Some(EQUIPMENT_DESTRUCTION_PARTIAL_REASON)), "{old_spans:?}");
    assert!(!old_spans.iter().any(|span| span.first_hour == 102 && span.reason == UnknownReason::DestructionWithheld), "{old_spans:?}");
    assert!(old_spans.iter().any(|span| span.first_hour == 104 && span.reason == UnknownReason::DestructionWithheld && span.detail.as_deref() == Some("drain overflowed")), "{old_spans:?}");
}
