use super::*;
use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};

const HOUR: i32 = 60_831_118;

fn bracket(before: i32, after: i32, panicked: bool) -> DomainClockBracket {
    DomainClockBracket {
        tag: COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE,
        opening: Some(ClockObservation { hour: before, ordinal: 1, at_us: 1 }),
        closing: Some(ClockObservation { hour: after, ordinal: 2, at_us: 2 }),
        worker_panicked: panicked,
    }
}

fn data_record() -> Vec<u8> {
    use compact_contract::xmap07_snow_mud::{encode_payload, Xmap07SnowMudSnapshot, Xmap07SnowMudWireBody};
    let grid = vec![7u8; 16];
    let snapshot = Xmap07SnowMudSnapshot {
        read_time_us: 3_000, status_flags: 0, source_sequence: 41,
        anchor_generation: 2, capture_ordinal: 1, first_capture_hour_raw: HOUR,
        game_hour_raw: HOUR, source_present: true, width: 2, height: 2,
        grid_sha256: Sha256::digest(&grid).into(), grid,
    };
    let payload = encode_payload(&snapshot, &Xmap07SnowMudWireBody::Keyframe).unwrap();
    let mut records = Vec::new();
    append_compact_record(&mut records, 41, HOUR, COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &payload);
    records
}

#[test]
fn xmap07_dispatch_joins_existing_guarded_reader_before_bulk_admission() {
    let source = include_str!("../attach_log.rs");
    let builder = &source[source.find("fn build_compact_extraction_gate_records(").unwrap()..];
    let read = builder.find("xmap07_snow_mud_read = xmap07_snow_mud_live_enabled.then(|| {").unwrap();
    let hold = builder.find("let admission = reader_gate_ref.map(|gate| gate.hold_admission());").unwrap();
    assert!(read < hold);
    let early = &builder[read..hold];
    assert_eq!(early.matches("spawn_gated(").count(), 1);
    assert_eq!(early.matches("read_compact_xmap07_snow_mud(").count(), 1);
    assert!(early.contains("handle.join()"));
    assert!(early.contains("compact_xmap07_snow_mud_worker_panic_read(status_flags, game_hour_raw)"));
    assert!(!source.contains("xmap07_snow_mud_handle"));
}

#[test]
fn xmap07_dispatch_same_hour_payload_survives_reconciliation_exactly() {
    let mut records = data_record();
    let expected = records.clone();
    let mut ordinals = BTreeMap::new();
    let (_, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records, 41, HOUR, &[bracket(HOUR, HOUR, false)],
        &mut ordinals, HourLabelMismatchPolicy::Refuse,
    ).unwrap();
    assert!(rejected.is_empty());
    assert!(ordinals.is_empty());
    assert_eq!(records, expected);
}

#[test]
fn xmap07_dispatch_late_torn_panicked_and_missing_brackets_still_gap() {
    let cases = [
        (vec![bracket(HOUR + 1, HOUR + 1, false)], DomainGapReason::HourLabelMismatch),
        (vec![bracket(HOUR, HOUR + 1, false)], DomainGapReason::Torn),
        (vec![bracket(HOUR, HOUR, true)], DomainGapReason::WorkerPanic),
        (Vec::new(), DomainGapReason::ClockUnreadable),
    ];
    for (brackets, expected) in cases {
        let mut records = data_record();
        let mut ordinals = BTreeMap::new();
        let (_, rejected) = replace_rejected_domain_records_with_gaps(
            &mut records, 41, HOUR, &brackets, &mut ordinals, HourLabelMismatchPolicy::Refuse,
        ).unwrap();
        assert!(rejected.contains(&COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE));
        let header = compact_contract::parse_compact_record_header(&records).unwrap();
        assert_eq!(header.game_hour_raw, HOUR);
        assert_eq!(header.record_type, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP);
        assert_eq!(records.len(), header.record_len());
        let gap = DomainGapRecordV1::decode(&records[usize::from(COMPACT_RECORD_HEADER_LEN)..]).unwrap();
        assert_eq!(gap.reason, expected);
        assert_eq!(gap.target_hour_raw, HOUR);
        assert_eq!(gap.outcome_ordinal, 0);
        if expected == DomainGapReason::HourLabelMismatch {
            assert_eq!((gap.tick_before, gap.tick_after), ((HOUR + 1) as u64, (HOUR + 1) as u64));
        }
    }
}
