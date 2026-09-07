use super::super::*;
use super::*;
use std::collections::BTreeMap;

const BASE: usize = 0x140000000;
const GS: usize = HEAP_LO + 0x400000;
const ROOT: usize = GS + 0x10000;
const PROVINCE: usize = GS + 0x20000;
const ARRAY: usize = GS + 0x30000;
const BODY: usize = GS + 0x40000;
const HOUR: i32 = DATE_EPOCH_CONST + 16_959_367; // 1936-01-01 07:00, within the fold calendar.
fn fixture_topology() -> CompactMapControlTopology {
    CompactMapControlTopology {
        province_array: ROOT,
        province_count: 2,
        states: Vec::new(),
        provinces: vec![CompactMapProvinceRoot {
            province_id: 1,
            province_ptr: PROVINCE,
            state_id: 0,
        }],
    }
}

fn capture(
    base: usize,
    gs: usize,
    hour: i32,
    ordinal: u64,
    budget: &Budget,
    read: impl FnMut(usize, &mut [u8]) -> bool,
) -> Census {
    super::capture(base, gs, hour, ordinal, &fixture_topology(), budget, read)
}

#[derive(Default)]
struct Memory {
    bytes: BTreeMap<usize, u8>,
    calls: Vec<(usize, usize)>,
    visits: BTreeMap<(usize, usize), usize>,
    mutation: Option<(usize, usize, usize, Vec<u8>)>,
}
impl Memory {
    fn put(&mut self, a: usize, b: &[u8]) {
        for (i, v) in b.iter().enumerate() {
            self.bytes.insert(a + i, *v);
        }
    }
    fn ptr(&mut self, a: usize, v: usize) {
        self.put(a, &(v as u64).to_le_bytes());
    }
    fn word(&mut self, a: usize, v: u32) {
        self.put(a, &v.to_le_bytes());
    }
    fn new() -> Self {
        let mut m = Self::default();
        m.ptr(
            BASE + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
            GS,
        );
        m.word(GS + GS_CURRENT_DATE_OFFSET, HOUR as u32);
        m.ptr(GS + CGAMESTATE_CPROVINCE_ARRAY, ROOT);
        m.word(GS + CGAMESTATE_CPROVINCE_COUNT, 2);
        m.ptr(ROOT, 0);
        m.ptr(ROOT + 8, PROVINCE);
        m.ptr(PROVINCE, BASE + CPROVINCE_VTABLE_RVA);
        m.word(PROVINCE + CPROVINCE_OFF_ID, 1);
        for offset in [0xe0, 0x110, 0xf8, 0x128] {
            m.put(PROVINCE + offset, &[0; 16]);
        }
        m.ptr(PROVINCE + 0xe0, ARRAY);
        m.word(PROVINCE + 0xe8, 3);
        m.word(PROVINCE + 0xec, 3);
        for (i, (primary, secondary, class, ty, id)) in [
            (0x2943ad0, 0x2943cb0, 0, 47, 9),
            (0x295b600, 0x295b7d8, 13, 51, 7),
            (0x295b600, 0x295b7d8, 13, 51, 3),
        ]
        .into_iter()
        .enumerate()
        {
            let body = BODY + i * 0x100;
            m.put(body, &[0; 32]);
            m.ptr(body, BASE + primary);
            m.word(body + 8, class);
            m.ptr(body + 16, BASE + secondary);
            m.word(body + 24, ty);
            m.word(body + 28, id);
            m.ptr(ARRAY + i * 8, body + if i == 1 { 16 } else { 0 });
        }
        m
    }
    fn read(&mut self, a: usize, b: &mut [u8]) -> bool {
        self.calls.push((a, b.len()));
        let n = self.visits.entry((a, b.len())).or_default();
        *n += 1;
        let n = *n;
        if let Some((at, width, visit, data)) = self.mutation.clone() {
            if (a, b.len(), n) == (at, width, visit) {
                self.put(at, &data);
            }
        }
        for (i, v) in b.iter_mut().enumerate() {
            let Some(byte) = self.bytes.get(&(a + i)) else {
                return false;
            };
            *v = *byte;
        }
        true
    }
}
#[test]
fn four_vectors_primary_secondary_order_and_empty_are_independent() {
    let mut m = Memory::new();
    let budget = Budget::default();
    let c = capture(BASE, GS, HOUR, 11, &budget, |a, b| m.read(a, b));
    assert_eq!(c.status, Status::Measured);
    let vectors = &c.provinces[1].vectors;
    assert_eq!(
        vectors[0]
            .slots
            .iter()
            .map(|s| match s.value {
                SlotValue::Measured(r) => r.id_raw,
                _ => panic!(),
            })
            .collect::<Vec<_>>(),
        vec![9, 7, 3]
    );
    for v in &vectors[1..] {
        assert_eq!(v.declared_count, Some(0));
        assert!(v.slots.is_empty());
        assert_eq!(v.status, Status::Measured);
    }
    assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
    let physical: usize = m.calls.iter().map(|(_, n)| n).sum();
    assert_eq!(physical, budget.verification_bytes());
    assert!(physical <= 56 + 184 * 2 + 80 * 3);
}
#[test]
fn reference_only_change_retains_class_but_class_change_withdraws_it() {
    for change_class in [false, true] {
        let mut m = Memory::new();
        let mut closing: Vec<_> = (0..32).map(|i| m.bytes[&(BODY + i)]).collect();
        let offset = if change_class { 8 } else { 28 };
        closing[offset..offset + 4].copy_from_slice(&99u32.to_le_bytes());
        m.mutation = Some((BODY, 32, 1, closing));
        let c = capture(BASE, GS, HOUR, 12, &Budget::default(), |a, b| m.read(a, b));
        assert_eq!(
            c.provinces[1].vectors[0].slots[0].value,
            SlotValue::Failed {
                cause: Cause::Changed,
                class_raw: if change_class { None } else { Some(0) }
            }
        );
        assert_eq!(c.status, Status::Partial);
    }
}
#[test]
fn secondary_reference_unreadable_keeps_only_the_independently_sealed_class() {
    for change_class in [false, true] {
        let mut m = Memory::new();
        let body = BODY + 0x100;
        for i in 24..32 {
            m.bytes.remove(&(body + i));
        }
        if change_class {
            let mut closing: Vec<_> = (0..16).map(|i| m.bytes[&(body + i)]).collect();
            closing[8..12].copy_from_slice(&0i32.to_le_bytes());
            m.mutation = Some((body, 16, 2, closing));
        }
        let c = capture(BASE, GS, HOUR, 15, &Budget::default(), |a, b| m.read(a, b));
        let slot = c.provinces[1].vectors[0].slots[1];
        assert_eq!(
            slot.value.class_witness(),
            if change_class { None } else { Some(13) }
        );
        assert!(matches!(slot.value, SlotValue::Failed { .. }));
        assert_eq!(
            m.visits[&(body + 24, 8)],
            2,
            "even an unreadable opening executes its funded close"
        );
        assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
    }
}
#[test]
fn small_shared_quota_still_executes_every_reserved_close() {
    let mut m = Memory::new();
    let budget = Budget::test_limits(56, 16 * 1024 * 1024);
    let c = capture(BASE, GS, HOUR, 13, &budget, |a, b| m.read(a, b));
    assert_ne!(c.status, Status::Measured);
    assert_eq!(c.cause, Cause::BoundExceeded);
    assert!(m.calls.iter().map(|(_, n)| n).sum::<usize>() <= 56);
    for ((_, _), count) in &m.visits {
        assert_eq!(*count, 2, "each funded opening has its closing read");
    }
}
#[test]
fn root_and_clock_mutation_refuse_the_whole_observation() {
    for clock in [false, true] {
        let mut m = Memory::new();
        m.mutation = Some(if clock {
            (
                GS + GS_CURRENT_DATE_OFFSET,
                4,
                2,
                (HOUR + 1).to_le_bytes().to_vec(),
            )
        } else {
            (ROOT + 8, 8, 3, 0u64.to_le_bytes().to_vec())
        });
        let c = capture_encoded(
            BASE,
            GS,
            HOUR,
            14,
            &fixture_topology(),
            &Budget::default(),
            |a, b| m.read(a, b),
        );
        assert!(!c.is_measured());
        let c = wire::decode(c.bytes()).unwrap();
        assert_eq!(c.status, Status::Failed);
        assert!(c.provinces.is_empty());
        assert_eq!(
            c.cause,
            if clock {
                Cause::ClockChanged
            } else {
                Cause::Changed
            }
        );
    }
}

#[test]
fn repeated_pointer_occurrences_survive_but_conflicting_complete_objects_all_refuse() {
    for conflicting in [false, true] {
        let mut m = Memory::new();
        if conflicting {
            let original: Vec<_> = (0..32).map(|i| m.bytes[&(BODY + i)]).collect();
            m.put(BODY + 0x200, &original);
        } else {
            m.ptr(ARRAY + 16, BODY);
        }
        let c = capture(BASE, GS, HOUR, 40, &Budget::default(), |a, b| m.read(a, b));
        let slots = &c.provinces[1].vectors[0].slots;
        assert_eq!(slots.len(), 3);
        if conflicting {
            for ordinal in [0, 2] {
                assert_eq!(
                    slots[ordinal].value,
                    SlotValue::Failed {
                        cause: Cause::IdentityConflict,
                        class_raw: Some(0)
                    }
                );
            }
            assert!(matches!(
                slots[1].value,
                SlotValue::Measured(Reference {
                    class_raw: 13,
                    id_raw: 7,
                    ..
                })
            ));
        } else {
            assert_eq!(c.status, Status::Measured);
            assert_eq!(slots[0].value, slots[2].value);
            assert_eq!(
                (slots[0].physical_ordinal, slots[2].physical_ordinal),
                (0, 2)
            );
        }
        assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
    }
}

#[test]
fn family_retains_first_failure_and_each_slot_keeps_its_own_cause() {
    let mut m = Memory::new();
    m.word(BODY + 24, 0);
    m.word(BODY + 28, 0);
    m.ptr(BODY + 0x100 + 16, BASE + 0x123456);
    let c = capture(BASE, GS, HOUR, 41, &Budget::default(), |a, b| m.read(a, b));
    let p = &c.provinces[1];
    let v = &p.vectors[0];
    assert_eq!(
        (c.cause, p.cause, v.cause),
        (
            Cause::UninitializedReference,
            Cause::UninitializedReference,
            Cause::UninitializedReference
        )
    );
    assert_eq!(
        v.slots[0].value,
        SlotValue::Failed {
            cause: Cause::UninitializedReference,
            class_raw: Some(0)
        }
    );
    assert_eq!(
        v.slots[1].value,
        SlotValue::Failed {
            cause: Cause::UnsupportedClass,
            class_raw: None
        }
    );
    assert!(matches!(v.slots[2].value, SlotValue::Measured(_)));
    assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
}

#[test]
fn negative_header_is_unavailable_and_bounded_prefix_is_never_measured_empty() {
    let mut invalid = Memory::new();
    invalid.word(PROVINCE + 0xec, u32::MAX);
    let c = capture(BASE, GS, HOUR, 42, &Budget::default(), |a, b| {
        invalid.read(a, b)
    });
    assert_eq!(c.provinces[1].vectors[0].declared_count, None);
    assert_eq!(c.provinces[1].vectors[0].cause, Cause::InvalidHeader);
    let mut bounded = Memory::new();
    let budget = Budget::test_limits(240, 16 * 1024 * 1024);
    let c = capture(BASE, GS, HOUR, 43, &budget, |a, b| bounded.read(a, b));
    let v = &c.provinces[1].vectors[0];
    assert_eq!(v.declared_count, Some(3));
    assert_ne!(v.status, Status::Measured);
    assert_eq!(v.cause, Cause::BoundExceeded);
    assert!(v.covered_prefix > 0 && v.covered_prefix < 3);
    assert!(bounded.calls.iter().map(|(_, n)| n).sum::<usize>() <= 240);
    assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
}

#[test]
fn concurrent_acquisitions_share_atomic_quota_and_attempt_all_funded_closes() {
    let budget = Budget::test_limits(512, 16 * 1024 * 1024);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|ordinal| {
                let budget = &budget;
                scope.spawn(move || {
                    let mut m = Memory::new();
                    let c = capture(BASE, GS, HOUR, 50 + ordinal, budget, |a, b| m.read(a, b));
                    // Successful complete-header normalization coalesces its
                    // opening spans into one 32-byte close. Compare actual byte
                    // coverage, not call counts, across the opening/closing cut.
                    let total: usize = m.calls.iter().map(|(_, width)| width).sum();
                    assert_eq!(total % 2, 0);
                    let mut opening_bytes = 0;
                    let split = if total == 0 { 0 } else {
                        m.calls.iter().position(|(_, width)| {
                            opening_bytes += *width;
                            opening_bytes == total / 2
                        }).expect("every funded opening byte must have a close") + 1
                    };
                    let coverage = |calls: &[(usize, usize)]| {
                        let mut bytes = BTreeMap::<usize, usize>::new();
                        for &(address, width) in calls {
                            for at in address..address + width { *bytes.entry(at).or_default() += 1; }
                        }
                        bytes
                    };
                    assert_eq!(coverage(&m.calls[..split]), coverage(&m.calls[split..]));
                    (c, m.calls.iter().map(|(_, n)| n).sum::<usize>())
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let physical: usize = results.iter().map(|(_, n)| n).sum();
    // The atomic budget records limit+1 after any refused reservation;
    // that sticky exhaustion sentinel is not an extra physical read.
    assert_eq!(budget.verification_bytes(), 513);
    assert!(physical <= 512);
    assert!(results.iter().any(|(c, _)| c.status != Status::Measured));
    for (c, _) in results {
        assert_eq!(wire::decode(&wire::encode(&c).unwrap()).unwrap(), c);
    }
}

fn payload(m: &mut Memory, hour: i32, ordinal: u64) -> Vec<u8> {
    m.word(GS + GS_CURRENT_DATE_OFFSET, hour as u32);
    m.visits.clear();
    m.calls.clear();
    let captured = capture_encoded(
        BASE,
        GS,
        hour,
        ordinal,
        &fixture_topology(),
        &Budget::default(),
        |a, b| m.read(a, b),
    );
    assert!(captured.is_measured());
    compact_map_control_data_payload(
        &[],
        &[CompactMapProvinceRow {
            province_id: 1,
            controller_tag_db_id: 0,
            state_id: 0,
        }],
        0,
        0,
        1,
        0,
        1,
        captured.bytes(),
    )
}
pub(crate) fn stage(stream: &[u8], sequence: u64, hour: i32) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let expected_hour = compact_contract::calendar::hour_index_from_raw(hour)
        .expect("producer/fold fixture must use an admitted calendar hour");
    let mut raw = stream.to_vec();
    append_compact_record(
        &mut raw,
        sequence,
        hour,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
    );
    let mut fold = recorder_host::world_fold::WorldFold::new("province_occupants_joint");
    fold.ingest_stream(&raw);
    let frames = fold.finish();
    let mut v3 = recorder_host::world_fold::WorldFold::new("province_occupants_joint");
    v3.enable_v3_seam(recorder_host::world_fold::V3SeamConfig {
        countries_depth: 3,
        divisions_depth: 2,
        states_depth: 2,
        provinces_depth: 1,
        census_only: false,
    })
    .unwrap();
    v3.ingest_stream(&raw);
    assert_eq!(
        v3.finish(),
        frames,
        "actual v3 cache path must preserve the entire snapshot, including POC1 refusal"
    );
    let frame = frames.last().expect("actual FRAME_END must emit the requested snapshot");
    assert_eq!(frame.0, expected_hour, "no earlier snapshot may substitute for this stage");
    let snapshot: serde_json::Value = serde_json::from_slice(&frame.2).unwrap();
    serde_json::json!({"snapshot_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&snapshot).unwrap())),"snapshot":snapshot,
        "raw_h4cr_sha256":format!("{:x}",Sha256::digest(&raw)),"raw_h4cr_hex":raw.iter().map(|b|format!("{b:02x}")).collect::<String>()})
}
#[test]
fn map_rows_and_retained_delta_identities_require_the_fresh_province_census() {
    for extra_current_row in [false, true] {
        let mut m = Memory::new();
        let first = payload(&mut m, HOUR, 30);
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
        assert_eq!(
            stage(&stream, 2, HOUR)["snapshot"]["province_occupants"]["root_coverage"]["status"],
            1
        );
        let mut current_topology = fixture_topology();
        if !extra_current_row {
            m.ptr(ROOT + 8, 0);
            // This fold-only adversary supplies the independently witnessed
            // current root census, not stale D22 discovery. The actual publisher
            // replacement/refusal/rebuild path has its own regression.
            current_topology.provinces.clear();
        }
        let captured = capture_encoded(
            BASE,
            GS,
            HOUR,
            31,
            &current_topology,
            &Budget::default(),
            |a, b| m.read(a, b),
        );
        assert!(
            captured.is_measured(),
            "the leaf itself is a genuine complete observation"
        );
        let rows = if extra_current_row {
            vec![
                CompactMapProvinceRow {
                    province_id: 1,
                    controller_tag_db_id: 0,
                    state_id: 0,
                },
                CompactMapProvinceRow {
                    province_id: 2,
                    controller_tag_db_id: 0,
                    state_id: 0,
                },
            ]
        } else {
            Vec::new()
        };
        let count = rows.len() as u32;
        let bad = compact_map_control_data_payload(
            &[],
            &rows,
            0,
            COMPACT_STATUS_DELTA_DATA,
            count,
            0,
            count,
            captured.bytes(),
        );
        append_compact_record(
            &mut stream,
            2,
            HOUR,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &bad,
        );
        assert!(stage(&stream,3,HOUR)["snapshot"]["province_occupants"].is_null(),
            "a valid leaf cannot authenticate a new or retained map identity that it did not witness");
        m.ptr(ROOT + 8, PROVINCE);
        let recovery = payload(&mut m, HOUR, 32);
        append_compact_record(
            &mut stream,
            3,
            HOUR,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &recovery,
        );
        assert!(
            stage(&stream, 4, HOUR)["snapshot"]["province_occupants"].is_null(),
            "same-hour historical fault remains"
        );
        append_compact_record(
            &mut stream,
            4,
            HOUR,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        let next = payload(&mut m, HOUR + 1, 33);
        append_compact_record(
            &mut stream,
            5,
            HOUR + 1,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &next,
        );
        assert_eq!(
            stage(&stream, 6, HOUR + 1)["snapshot"]["province_occupants"]["root_coverage"]
                ["status"],
            1
        );
    }
}

#[test]
fn fresh_cells_must_match_the_topology_that_supplied_control_rows_before_following() {
    for replacement in [0, PROVINCE + 0x1000] {
        let mut m = Memory::new();
        m.ptr(ROOT + 8, replacement);
        if replacement != 0 {
            m.ptr(replacement, BASE + CPROVINCE_VTABLE_RVA);
            m.word(replacement + CPROVINCE_OFF_ID, 1);
        }
        let budget = Budget::default();
        let c = capture(BASE, GS, HOUR, 71, &budget, |a, b| m.read(a, b));
        assert_eq!(c.status, Status::Failed);
        assert_eq!(c.cause, Cause::Changed);
        assert!(c.provinces.is_empty());
        assert!(!m
            .calls
            .iter()
            .any(|(a, _)| *a == replacement && replacement != 0));
        assert_eq!(
            m.calls.iter().map(|(_, n)| n).sum::<usize>(),
            budget.verification_bytes()
        );
        assert_eq!(m.visits[&(ROOT + 8, 8)], 2, "funded cell close still runs");
    }
}

#[test]
fn a_full_reference_pair_collision_across_classes_withdraws_both_occurrences() {
    let mut m = Memory::new();
    let other = BODY + 0x100;
    m.ptr(other, BASE + 0x294c480);
    m.word(other + 8, 1);
    m.ptr(other + 16, BASE + 0x294c660);
    m.word(other + 24, 47);
    m.word(other + 28, 9);
    let c = capture(BASE, GS, HOUR, 80, &Budget::default(), |a, b| m.read(a, b));
    let slots = &c.provinces[1].vectors[0].slots;
    assert_eq!(
        slots[0].value,
        SlotValue::Failed {
            cause: Cause::IdentityConflict,
            class_raw: Some(0)
        }
    );
    assert_eq!(
        slots[1].value,
        SlotValue::Failed {
            cause: Cause::IdentityConflict,
            class_raw: Some(1)
        }
    );
    assert!(matches!(slots[2].value, SlotValue::Measured(_)));
    assert_eq!(c.status, Status::Partial);
}

#[test]
fn shared_retention_funds_all_three_leaf_destinations_before_publication() {
    let mut m = Memory::new();
    let probe = Budget::default();
    let measured = capture_encoded(BASE, GS, HOUR, 81, &fixture_topology(), &probe, |a, b| {
        m.read(a, b)
    });
    assert!(measured.is_measured());
    let needed = probe.retained_bytes();
    assert!(needed >= 3 * measured.bytes().len() + 80 + usize::from(COMPACT_RECORD_HEADER_LEN));
    for (limit, expected) in [(needed, true), (needed - 2 * measured.bytes().len(), false)] {
        let mut m = Memory::new();
        let budget = Budget::test_limits(10 * 1024 * 1024, limit);
        let captured = capture_encoded(BASE, GS, HOUR, 81, &fixture_topology(), &budget, |a, b| {
            m.read(a, b)
        });
        assert_eq!(captured.is_measured(), expected);
        assert_eq!(
            m.calls.iter().map(|(_, n)| n).sum::<usize>(),
            budget.verification_bytes()
        );
        if expected {
            assert_eq!(budget.retained_bytes(), needed);
        } else {
            assert!(
                !captured.is_measured(),
                "the former single-copy allowance cannot publish measured"
            );
        }
    }
}
