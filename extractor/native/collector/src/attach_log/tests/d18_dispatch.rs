use super::state_fixture::{
    clean_status_fixture, compact_control_scheduling, drive_compact_frame, fixture_game_hour_raw,
    CollectorStateFixture,
};
use super::*;

const COUNTRY_AT: usize = 0x100;
const GAME_STATE_AT: usize = 0x2000;

fn identity_fixture(hour: i32) -> CollectorStateFixture {
    let mut fixture = clean_status_fixture(0x20000);
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);
    fixture
        .heap
        .write_i32(GAME_STATE_AT + GS_CURRENT_DATE_OFFSET, hour);
    let vtable = fixture.collector_state().country_identity.country_vtable;
    fixture.heap.write_u64(COUNTRY_AT, vtable as u64);
    fixture.heap.write_u32(COUNTRY_AT + OBJ_TAG_DB_ID, 1);
    fixture
}

#[test]
fn country_identity_closes_before_bulk_readers_and_keeps_its_roster() {
    let hour = fixture_game_hour_raw();
    let fixture = identity_fixture(hour);
    let mut control = compact_control_scheduling(&["country_identity", "fuel_live"]);
    control.reader_concurrency = 1;
    let outcome = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    let identity = outcome.lane_span(COMPACT_DOMAIN_COUNTRY_IDENTITY).unwrap();
    let fuel = outcome.lane_span(COMPACT_DOMAIN_FUEL).unwrap();
    assert!(identity.end_us <= fuel.start_us, "{identity:?}; {fuel:?}");
    assert_ne!(identity.lane, LW_MAIN_LANE);
    assert_eq!(
        outcome
            .timing
            .lane_spans
            .iter()
            .filter(|span| span.tag == COMPACT_DOMAIN_COUNTRY_IDENTITY)
            .count(),
        1
    );
    let data = outcome
        .domain_data(COMPACT_DOMAIN_COUNTRY_IDENTITY)
        .unwrap();
    assert_eq!(data.game_hour_raw, hour);
    assert_eq!(
        u32::from_le_bytes(data.payload[8..12].try_into().unwrap()),
        1
    );
    assert_eq!(
        u32::from_le_bytes(data.payload[12..16].try_into().unwrap()),
        1
    );
    assert_eq!(
        u32::from_le_bytes(data.payload[16..20].try_into().unwrap()),
        0
    );
    let brackets = &outcome.timing.domain_clock_brackets;
    let bracket = brackets
        .iter()
        .find(|b| b.tag == COMPACT_DOMAIN_COUNTRY_IDENTITY)
        .unwrap();
    assert_eq!(bracket_observed_hour_raw(bracket), Some(hour as u64));
    assert!(
        !rejected_bracket_domains(brackets, hour, HOUR_LABEL_MISMATCH_POLICY)
            .contains_key(&COMPACT_DOMAIN_COUNTRY_IDENTITY)
    );
}

#[test]
fn early_country_identity_still_refuses_a_next_hour_bracket() {
    let hour = fixture_game_hour_raw();
    let fixture = identity_fixture(hour + 1);
    let control = compact_control_scheduling(&["country_identity"]);
    let outcome = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    assert!(outcome
        .domain_data(COMPACT_DOMAIN_COUNTRY_IDENTITY)
        .is_none());
    let gap = outcome
        .record(
            COMPACT_DOMAIN_COUNTRY_IDENTITY,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        )
        .unwrap();
    let gap = compact_contract::domain_gap::DomainGapRecordV1::decode(&gap.payload).unwrap();
    assert_eq!(
        gap.reason,
        compact_contract::domain_gap::DomainGapReason::HourLabelMismatch
    );
    assert_eq!(gap.target_hour_raw, hour);
    assert_eq!(gap.tick_before, (hour + 1) as u64);
    assert_eq!(gap.tick_after, (hour + 1) as u64);
}

#[test]
fn unscheduled_country_identity_has_no_early_read_or_record() {
    let hour = fixture_game_hour_raw();
    let fixture = identity_fixture(hour);
    let control = compact_control_scheduling(&["fuel_live"]);
    let outcome = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    assert!(outcome.lane_span(COMPACT_DOMAIN_COUNTRY_IDENTITY).is_none());
    assert!(outcome
        .records
        .iter()
        .all(|record| record.domain_id != COMPACT_DOMAIN_COUNTRY_IDENTITY));
}
