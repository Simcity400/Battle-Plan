use super::state_fixture::CollectorStateFixture;
use super::*;
use crate::attach_log::province_occupants::tests::stage;
use compact_contract::province_occupants::{self as wire, SlotValue, Status};

const HOUR: i32 = DATE_EPOCH_CONST + 16_959_367; // 1936-01-01 07:00, within the fold calendar.
const GS: usize = 0;
const ROOT: usize = 0x4000;
const PROVINCE: usize = 0x5000;
const ARRAY: usize = 0x6000;
const BODY: usize = 0x7000;

struct Image {
    fixture: CollectorStateFixture,
}
impl Image {
    fn new() -> Self {
        let mut fixture = CollectorStateFixture::new(
            &[compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA],
            0x1000,
            0x10000,
        );
        fixture.stage_scheduler_clock_at(GS);
        fixture.heap.write_i32(GS + GS_CURRENT_DATE_OFFSET, HOUR);
        let base = fixture.module.module_base();
        let root = fixture.heap.at(ROOT);
        let province = fixture.heap.at(PROVINCE);
        let array = fixture.heap.at(ARRAY);
        fixture
            .heap
            .write_u64(GS + CGAMESTATE_CPROVINCE_ARRAY, root as u64);
        fixture.heap.write_u32(GS + CGAMESTATE_CPROVINCE_COUNT, 2);
        fixture.heap.write_u64(ROOT + 8, province as u64);
        fixture
            .heap
            .write_u64(PROVINCE, (base + CPROVINCE_VTABLE_RVA) as u64);
        fixture.heap.write_u32(PROVINCE + CPROVINCE_OFF_ID, 1);
        // A real state row keeps both existing D22 baselines nonempty, so the
        // next order-only observation exercises the delta path rather than the
        // preexisting empty-state full-baseline rule. Country operands are zero.
        let state = fixture.heap.at(0x9000);
        fixture
            .heap
            .write_u64(PROVINCE + CPROVINCE_OFF_STATE, state as u64);
        fixture
            .heap
            .write_u64(0x9000, (base + STATE_VTABLE_RVA) as u64);
        fixture.heap.write_u32(0x9000 + STATE_OFF_SID, 1);
        fixture.heap.write_u64(PROVINCE + 0xe0, array as u64);
        fixture.heap.write_u32(PROVINCE + 0xe8, 3);
        fixture.heap.write_u32(PROVINCE + 0xec, 3);
        for (i, (primary, secondary, class, ty, id)) in [
            (0x2943ad0, 0x2943cb0, 0, 47, 9),
            (0x295b600, 0x295b7d8, 13, 51, 7),
            (0x295b600, 0x295b7d8, 13, 51, 3),
        ]
        .into_iter()
        .enumerate()
        {
            let offset = BODY + i * 0x100;
            let pointer = fixture.heap.at(offset) + if i == 1 { 16 } else { 0 };
            fixture.heap.write_u64(offset, (base + primary) as u64);
            fixture.heap.write_u32(offset + 8, class);
            fixture
                .heap
                .write_u64(offset + 16, (base + secondary) as u64);
            fixture.heap.write_u32(offset + 24, ty);
            fixture.heap.write_u32(offset + 28, id);
            fixture.heap.write_u64(ARRAY + i * 8, pointer as u64);
        }
        Self { fixture }
    }

    fn acquire(
        &mut self,
        trackers: &mut CompactPerfGateTrackers,
        hour: i32,
        budget: &country_roster_readset::Budget,
        cross_tick: bool,
    ) -> (CompactMapControlRead, Vec<u8>, DomainClockBracket) {
        self.fixture
            .heap
            .write_i32(GS + GS_CURRENT_DATE_OFFSET, hour);
        let state = self.fixture.collector_state();
        let lanes = FrameLaneTracker::new(
            Instant::now(),
            Instant::now(),
            state.base,
            state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
        );
        let topology = CompactFrameMapTopologyCache::new(&lanes, hour);
        let gate = ReaderGate::new(1);
        let published = thread::scope(|scope| {
            spawn_gated_publishing(
                scope,
                Some(&gate),
                &lanes,
                COMPACT_DOMAIN_MAP_CONTROL,
                || {
                    let read = read_compact_map_control(
                        &state,
                        0,
                        &mut trackers.map_control,
                        &topology,
                        budget,
                    );
                    if cross_tick {
                        self.fixture
                            .heap
                            .write_i32(GS + GS_CURRENT_DATE_OFFSET, hour + 1);
                    }
                    read
                },
                |read| compact_map_control_read_payload(read, 0),
            )
            .join()
            .expect("actual D22 publishing worker")
        });
        let (read, bytes) = published.split();
        let bytes = bytes.expect("worker completed its owning encoder");
        let mut brackets = lanes.take_clock_brackets();
        assert_eq!(brackets.len(), 1);
        let bracket = brackets.pop().unwrap();
        if read.occupants.publication_funded() {
            let (_, census) = wire::split_map_control(&bytes).unwrap();
            assert_eq!(census.game_hour_raw, hour);
            assert_eq!(census.observation_ordinal, bracket.opening.unwrap().ordinal);
        } else {
            assert!(
                bytes.is_empty(),
                "unfunded worker cannot allocate a MAPC payload"
            );
        }
        (read, bytes, bracket)
    }
}

fn assert_order(bytes: &[u8], expected: &[u32]) {
    let (_, census) = wire::split_map_control(bytes).unwrap();
    assert_eq!(census.status, Status::Measured);
    let vectors = &census.provinces[1].vectors;
    let ids: Vec<_> = vectors[0]
        .slots
        .iter()
        .map(|slot| match slot.value {
            SlotValue::Measured(reference) => reference.id_raw,
            _ => panic!("expected an actual complete typed observation"),
        })
        .collect();
    assert_eq!(ids, expected);
    for vector in &vectors[1..] {
        assert_eq!(vector.declared_count, Some(0));
        assert!(vector.slots.is_empty());
    }
}

#[test]
fn cached_topology_still_reads_current_order_inside_the_actual_publishing_bracket() {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let (first, bytes, bracket) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(
        bracket_hour_verdict(&bracket, HOUR),
        BracketHourVerdict::InFrameHour
    );
    assert_eq!(first.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_order(&bytes, &[9, 7, 3]);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let third = image.fixture.heap.at(BODY + 0x200);
    let second = image.fixture.heap.at(BODY + 0x100) + 16;
    image.fixture.heap.write_u64(ARRAY + 8, third as u64);
    image.fixture.heap.write_u64(ARRAY + 16, second as u64);
    let (next, bytes, bracket) = image.acquire(
        &mut trackers,
        HOUR + 1,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(
        bracket_hour_verdict(&bracket, HOUR + 1),
        BracketHourVerdict::InFrameHour
    );
    assert!(
        next.province_rows.is_empty(),
        "unchanged map rows use the existing topology/delta path"
    );
    assert_ne!(next.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_order(&bytes, &[9, 3, 7]);
}

#[test]
fn published_partial_clears_map_baseline_but_sink_refusal_restores_its_frame_entry() {
    for published in [false, true] {
        let mut image = Image::new();
        let mut trackers = CompactPerfGateTrackers::default();
        image.acquire(
            &mut trackers,
            HOUR,
            &country_roster_readset::Budget::default(),
            false,
        );
        commit_country_staged_advances(&mut trackers, &BTreeSet::new());
        assert_eq!(trackers.map_control.previous_provinces.len(), 1);
        let (partial, _, _) = image.acquire(
            &mut trackers,
            HOUR + 1,
            &country_roster_readset::Budget::test_limits(0, 16 * 1024 * 1024),
            false,
        );
        assert!(compact_contract::compact_status_is_failure(
            partial.status_flags
        ));
        assert!(trackers.map_control.previous_provinces.is_empty());
        let withheld = if published {
            BTreeSet::new()
        } else {
            BTreeSet::from([COMPACT_DOMAIN_MAP_CONTROL])
        };
        commit_country_staged_advances(&mut trackers, &withheld);
        assert_eq!(
            trackers.map_control.previous_provinces.len(),
            if published { 0 } else { 1 }
        );
        let (next, bytes, _) = image.acquire(
            &mut trackers,
            HOUR + 2,
            &country_roster_readset::Budget::default(),
            false,
        );
        assert_eq!(
            next.status_flags & COMPACT_STATUS_DELTA_DATA == 0,
            published
        );
        assert_order(&bytes, &[9, 7, 3]);
    }
}

#[test]
fn outer_clock_refusal_rolls_back_and_drops_the_map_topology_before_recovery() {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let (_, initial, _) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let (_, rejected_payload, bracket) = image.acquire(
        &mut trackers,
        HOUR + 1,
        &country_roster_readset::Budget::default(),
        true,
    );
    assert_eq!(
        bracket_hour_verdict(&bracket, HOUR + 1),
        BracketHourVerdict::Straddled
    );
    reset_trackers_after_rejected_bracket(
        &mut trackers,
        &BTreeSet::from([COMPACT_DOMAIN_MAP_CONTROL]),
    );
    assert!(trackers.map_control.topology.is_none());
    let (next, bytes, _) = image.acquire(
        &mut trackers,
        HOUR + 2,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(next.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_order(&bytes, &[9, 7, 3]);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_joint_data(&mut stream, &mut sequence, HOUR, &initial);
    close_joint_hour(&mut stream, &mut sequence, HOUR);
    let mut rejected_records = Vec::new();
    let mut rejected_sequence = sequence;
    append_joint_data(&mut rejected_records, &mut rejected_sequence, HOUR + 1, &rejected_payload);
    let (next_sequence, rejected) = replace_rejected_domain_records_with_gaps(
        &mut rejected_records, sequence, HOUR + 1, &[bracket],
        &mut BTreeMap::new(), HourLabelMismatchPolicy::Refuse,
    ).unwrap();
    assert!(rejected.contains(&COMPACT_DOMAIN_MAP_CONTROL));
    let header = compact_contract::parse_compact_record_header(&rejected_records).unwrap();
    assert_eq!(header.record_type, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP);
    let gap = compact_contract::domain_gap::DomainGapRecordV1::decode(
        &rejected_records[usize::from(COMPACT_RECORD_HEADER_LEN)..header.record_len()],
    ).unwrap();
    assert_eq!(gap.reason, compact_contract::domain_gap::DomainGapReason::Torn);
    stream.extend_from_slice(&rejected_records);
    sequence = next_sequence;
    assert_joint_refused(&stage(&stream, sequence, HOUR + 1));
    close_joint_hour(&mut stream, &mut sequence, HOUR + 1);
    append_joint_data(&mut stream, &mut sequence, HOUR + 2, &bytes);
    let recovered = stage(&stream, sequence, HOUR + 2);
    assert_eq!(recovered["snapshot"]["province_occupants"]["root_coverage"]["status"], 1);
    assert!(recovered["snapshot"]["provinces"]["1"]["controller"].is_null());
}

#[test]
#[ignore = "exports actual bracketed publisher/fold stages; exclusive output required"]
fn actual_publishing_bracket_mapc2_fold_history_and_next_hour_export() {
    use std::io::Write;
    let output = std::env::var_os("BATTLE_PLAN_POC1_JOINT_OUTPUT").or_else(|| std::env::var_os("HOI4_POC1_JOINT_OUTPUT")).expect("exclusive output required");
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let (_, first, bracket) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(
        bracket_hour_verdict(&bracket, HOUR),
        BracketHourVerdict::InFrameHour
    );
    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        HOUR,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &first,
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let accepted = stage(&stream, 2, HOUR);
    assert_eq!(
        accepted["snapshot"]["province_occupants"]["root_coverage"]["status"],
        1
    );
    let (_, failed, _) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::test_limits(0, 16 * 1024 * 1024),
        false,
    );
    append_compact_record(
        &mut stream,
        2,
        HOUR,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &failed,
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let refused = stage(&stream, 3, HOUR);
    assert!(refused["snapshot"]["province_occupants"].is_null());
    let (recovery_read, recovery, _) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(recovery_read.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_compact_record(
        &mut stream,
        3,
        HOUR,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &recovery,
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let historical = stage(&stream, 4, HOUR);
    assert!(historical["snapshot"]["province_occupants"].is_null());
    append_compact_record(
        &mut stream,
        4,
        HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
    );
    // A raw full resync in the same refused hour does not erase the producer's
    // journalled obligation to publish a full baseline in the next clean hour.
    let (next_read, next, _) = image.acquire(
        &mut trackers,
        HOUR + 1,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_eq!(next_read.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_compact_record(
        &mut stream,
        5,
        HOUR + 1,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &next,
    );
    let clean = stage(&stream, 6, HOUR + 1);
    assert_eq!(
        clean["snapshot"]["province_occupants"]["root_coverage"]["status"],
        1
    );
    let bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"schema": 1, "stages": [accepted, refused, historical, clean]}),
    )
    .unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn cached_control_topology_cannot_certify_replacement_province_with_the_same_id() {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let (_, bytes, _) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    assert_order(&bytes, &[9, 7, 3]);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let replacement = 0x8000;
    let base = image.fixture.module.module_base();
    let pointer = image.fixture.heap.at(replacement);
    image
        .fixture
        .heap
        .write_u64(replacement, (base + CPROVINCE_VTABLE_RVA) as u64);
    image
        .fixture
        .heap
        .write_u32(replacement + CPROVINCE_OFF_ID, 1);
    image.fixture.heap.write_u64(ROOT + 8, pointer as u64);
    let (read, bytes, _) = image.acquire(
        &mut trackers,
        HOUR,
        &country_roster_readset::Budget::default(),
        false,
    );
    let (_, census) = wire::split_map_control(&bytes).unwrap();
    assert_eq!(census.status, Status::Failed);
    assert_eq!(census.cause, wire::Cause::Changed);
    assert_ne!(read.status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert!(census.provinces.is_empty());
    assert!(
        trackers.map_control.topology.is_none(),
        "stale discovery is withdrawn"
    );
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    let (recovered, bytes, _) = image.acquire(
        &mut trackers,
        HOUR + 1,
        &country_roster_readset::Budget::default(),
        false,
    );
    let (_, census) = wire::split_map_control(&bytes).unwrap();
    assert_eq!(census.status, Status::Measured);
    assert_eq!(census.provinces[1].vectors[0].declared_count, Some(0));
    assert_eq!(recovered.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(
        trackers
            .map_control
            .topology
            .as_ref()
            .unwrap()
            .province_ptr(1),
        Some(pointer)
    );
}

#[test]
fn initial_quota_exhaustion_and_dependency_failure_publish_only_the_typed_gap() {
    for dependency_failure in [false, true] {
        let mut image = Image::new();
        if dependency_failure {
            image
                .fixture
                .heap
                .write_u32(GS + CGAMESTATE_CPROVINCE_COUNT, 0);
        }
        let mut trackers = CompactPerfGateTrackers::default();
        let budget = country_roster_readset::Budget::test_limits(10 * 1024 * 1024, 0);
        let (read, bytes, _) = image.acquire(&mut trackers, HOUR, &budget, false);
        assert!(!read.occupants.publication_funded());
        assert!(bytes.is_empty());
        assert!(compact_contract::compact_status_is_failure(
            read.status_flags
        ));
        assert!(trackers.map_control.previous_provinces.is_empty());
        let mut raw = Vec::new();
        append_unfunded_map_control_gap(&mut raw, 1, HOUR, 0);
        assert_eq!(
            u16::from_le_bytes(raw[22..24].try_into().unwrap()),
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
        );
        assert!(!raw.windows(4).any(|w| w == b"MAPC"));
        assert!(stage(&raw, 2, HOUR)["snapshot"]["province_occupants"].is_null());
        assert!(
            budget.retained_bytes() > 0,
            "failed reservation remains charged"
        );
    }
}


// Every row/leaf below is produced by the same Image acquisition and encoder.
// Only the hostile malformed-DATA case changes a byte, before actual ingest.
fn append_joint_data(stream: &mut Vec<u8>, sequence: &mut u64, hour: i32, payload: &[u8]) {
    append_compact_record(stream, *sequence, hour, COMPACT_DOMAIN_MAP_CONTROL,
        COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, payload);
    *sequence += 1;
}
fn close_joint_hour(stream: &mut Vec<u8>, sequence: &mut u64, hour: i32) {
    append_compact_record(stream, *sequence, hour, compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END, 0,
        &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN]);
    *sequence += 1;
}
fn assert_joint_refused(value: &serde_json::Value) {
    assert!(value["snapshot"]["province_occupants"].is_null());
    assert!(value["snapshot"]["provenance"]["collector"]["domain_problems"]
        .as_array().unwrap().iter().any(|p| p["domain_id"] == 22));
}
fn assert_joint_current(value: &serde_json::Value, controller: u32, order: &[u32]) {
    assert_eq!(value["snapshot"]["provinces"]["1"]["controller"], controller.to_string());
    let carrier = &value["snapshot"]["province_occupants"];
    assert_eq!(carrier["root_coverage"]["status"], 1);
    let slots = carrier["provinces"][1]["vectors"][0]["slots"].as_array().unwrap();
    let actual: Vec<u64> = slots.iter().map(|slot| slot["reference"]["id_raw"].as_u64().unwrap()).collect();
    assert_eq!(actual, order.iter().map(|id| u64::from(*id)).collect::<Vec<_>>());
}

#[test]
fn published_partial_and_withheld_pin_rollback_have_distinct_actual_fold_recovery() {
    for published in [false, true] {
        let mut image = Image::new();
        let mut trackers = CompactPerfGateTrackers::default();
        let mut stream = Vec::new();
        let mut sequence = 1;
        let (_, initial, _) = image.acquire(&mut trackers, HOUR, &country_roster_readset::Budget::default(), false);
        append_joint_data(&mut stream, &mut sequence, HOUR, &initial);
        commit_country_staged_advances(&mut trackers, &BTreeSet::new());
        close_joint_hour(&mut stream, &mut sequence, HOUR);

        image.fixture.heap.write_u32(PROVINCE + CPROVINCE_OFF_CONTROLLER, 17);
        let (partial, bytes, _) = image.acquire(&mut trackers, HOUR + 1,
            &country_roster_readset::Budget::test_limits(0, 16 * 1024 * 1024), false);
        assert!(compact_contract::compact_status_is_failure(partial.status_flags));
        if published {
            append_joint_data(&mut stream, &mut sequence, HOUR + 1, &bytes);
            commit_country_staged_advances(&mut trackers, &BTreeSet::new());
        } else {
            // Actual typed no-DATA outcome; the authoritative sink did not take
            // the staged read, so restore the producer's exact frame-entry pin.
            append_unfunded_map_control_gap(&mut stream, sequence, HOUR + 1, 0);
            sequence += 1;
            commit_country_staged_advances(&mut trackers, &BTreeSet::from([COMPACT_DOMAIN_MAP_CONTROL]));
        }
        assert_joint_refused(&stage(&stream, sequence, HOUR + 1));
        close_joint_hour(&mut stream, &mut sequence, HOUR + 1);

        image.fixture.heap.write_u64(ARRAY + 8, image.fixture.heap.at(BODY + 0x200) as u64);
        image.fixture.heap.write_u64(ARRAY + 16, (image.fixture.heap.at(BODY + 0x100) + 16) as u64);
        let (next, bytes, _) = image.acquire(&mut trackers, HOUR + 2, &country_roster_readset::Budget::default(), false);
        assert_eq!(next.status_flags & COMPACT_STATUS_DELTA_DATA == 0, published);
        assert_order(&bytes, &[9, 3, 7]);
        append_joint_data(&mut stream, &mut sequence, HOUR + 2, &bytes);
        assert_joint_current(&stage(&stream, sequence, HOUR + 2), 17, &[9, 3, 7]);
    }
}

fn refused_data_requires_full_before_actual_delta_can_recover(kind: &str) {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut stream = Vec::new();
    let mut sequence = 1;
    let (_, bytes, _) = image.acquire(&mut trackers, HOUR, &country_roster_readset::Budget::default(), false);
    append_joint_data(&mut stream, &mut sequence, HOUR, &bytes);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    close_joint_hour(&mut stream, &mut sequence, HOUR);
    image.fixture.heap.write_u32(PROVINCE + CPROVINCE_OFF_CONTROLLER, 17);
    let (changed, mut damaged, _) = image.acquire(&mut trackers, HOUR + 1, &country_roster_readset::Budget::default(), false);
    assert_ne!(changed.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    match kind {
        "malformed" => damaged[0] ^= 1,
        "inadmissible_resync" => {
            // MAPC2 envelope precedes the owning 32-byte map row header.
            let status = wire::MAP_HEADER_LEN + 16;
            let raw = u32::from_le_bytes(damaged[status..status + 4].try_into().unwrap());
            damaged[status..status + 4].copy_from_slice(&(raw | compact_contract::COMPACT_STATUS_BASELINE_CLEAR).to_le_bytes());
        }
        "empty_full" => {
            // Reuse the actual encoder and current POC observation, but omit
            // both map row families and claim FULL: an invalid replacement.
            damaged = compact_map_control_data_payload(&[], &[], 0, 0, 0, 0, 0, changed.occupants.bytes());
        }
        _ => panic!("unknown hostile DATA case"),
    } // Mutate only hostile encoded input, never the folded snapshot.
    append_joint_data(&mut stream, &mut sequence, HOUR + 1, &damaged);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    assert_joint_refused(&stage(&stream, sequence, HOUR + 1));
    close_joint_hour(&mut stream, &mut sequence, HOUR + 1);
    let (next, bytes, _) = image.acquire(&mut trackers, HOUR + 2, &country_roster_readset::Budget::default(), false);
    assert_ne!(next.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert!(next.state_rows.is_empty() && next.province_rows.is_empty(),
        "producer already advanced controller17; next valid delta cannot repair a missed row");
    append_joint_data(&mut stream, &mut sequence, HOUR + 2, &bytes);
    assert_joint_refused(&stage(&stream, sequence, HOUR + 2));
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    close_joint_hour(&mut stream, &mut sequence, HOUR + 2);
    // Exercise the existing real topology rebuild/full-publication path.
    trackers.map_control.topology = None;
    let (full, bytes, _) = image.acquire(&mut trackers, HOUR + 3, &country_roster_readset::Budget::default(), false);
    assert_eq!(full.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_joint_data(&mut stream, &mut sequence, HOUR + 3, &bytes);
    assert_joint_current(&stage(&stream, sequence, HOUR + 3), 17, &[9, 7, 3]);
}

#[test]
fn malformed_published_data_requires_full_before_actual_delta_can_recover() {
    refused_data_requires_full_before_actual_delta_can_recover("malformed");
}
#[test]
fn inadmissible_resync_changed_data_cannot_recover_with_an_empty_delta() {
    refused_data_requires_full_before_actual_delta_can_recover("inadmissible_resync");
}
#[test]
fn empty_full_data_cannot_recover_with_an_empty_delta() {
    refused_data_requires_full_before_actual_delta_can_recover("empty_full");
}

#[test]
fn unrelated_domain_gap_keeps_actual_d22_delta_usable() {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut stream = Vec::new();
    let mut sequence = 1;
    let (_, bytes, _) = image.acquire(&mut trackers, HOUR, &country_roster_readset::Budget::default(), false);
    append_joint_data(&mut stream, &mut sequence, HOUR, &bytes);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    close_joint_hour(&mut stream, &mut sequence, HOUR);
    image.fixture.heap.write_u32(PROVINCE + CPROVINCE_OFF_CONTROLLER, 17);
    let (delta, bytes, _) = image.acquire(&mut trackers, HOUR + 1, &country_roster_readset::Budget::default(), false);
    assert_ne!(delta.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_joint_data(&mut stream, &mut sequence, HOUR + 1, &bytes);
    let gap = compact_contract::domain_gap::DomainGapRecordV1 {
        part: compact_contract::domain_gap::DomainGapPart::State,
        reason: compact_contract::domain_gap::DomainGapReason::SourceUnavailable,
        target_hour_raw: HOUR + 1, range_start_hour_raw: HOUR + 1, range_end_hour_raw: HOUR + 1,
        tick_before: 0, tick_after: 0, queue_capacity: 0, queue_depth: 0, queue_dropped: 0, outcome_ordinal: 0,
    }.encode_with_witness_provenance(compact_contract::domain_gap::DomainGapWitnessProvenance::Unobserved).unwrap();
    append_compact_record(&mut stream, sequence, HOUR + 1, 23,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &gap);
    sequence += 1;
    assert_joint_current(&stage(&stream, sequence, HOUR + 1), 17, &[9, 7, 3]);
}


#[test]
fn partial_full_recovery_preserves_same_hour_ledger_then_next_clean_hour() {
    let mut image = Image::new();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut stream = Vec::new();
    let mut sequence = 1;
    let (_, bytes, _) = image.acquire(&mut trackers, HOUR, &country_roster_readset::Budget::default(), false);
    append_joint_data(&mut stream, &mut sequence, HOUR, &bytes);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    image.fixture.heap.write_u32(PROVINCE + CPROVINCE_OFF_CONTROLLER, 17);
    let (partial, bytes, _) = image.acquire(&mut trackers, HOUR,
        &country_roster_readset::Budget::test_limits(0, 16 * 1024 * 1024), false);
    assert!(compact_contract::compact_status_is_failure(partial.status_flags));
    append_joint_data(&mut stream, &mut sequence, HOUR, &bytes);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    assert_joint_refused(&stage(&stream, sequence, HOUR));
    let (full, bytes, _) = image.acquire(&mut trackers, HOUR, &country_roster_readset::Budget::default(), false);
    assert_eq!(full.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_joint_data(&mut stream, &mut sequence, HOUR, &bytes);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    assert_joint_refused(&stage(&stream, sequence, HOUR));
    close_joint_hour(&mut stream, &mut sequence, HOUR);
    let (next, bytes, _) = image.acquire(&mut trackers, HOUR + 1, &country_roster_readset::Budget::default(), false);
    assert_eq!(next.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    append_joint_data(&mut stream, &mut sequence, HOUR + 1, &bytes);
    assert_joint_current(&stage(&stream, sequence, HOUR + 1), 17, &[9, 7, 3]);
}
