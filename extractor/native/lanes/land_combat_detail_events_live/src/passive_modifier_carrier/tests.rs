use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct FakeMemory {
    bytes: BTreeMap<usize, u8>,
    heap: BTreeSet<usize>,
    divisions: BTreeMap<usize, u32>,
}

impl FakeMemory {
    fn new() -> Self {
        Self::default()
    }

    fn put_bytes(&mut self, addr: usize, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.bytes.insert(addr + index, *byte);
        }
    }

    fn put_u32(&mut self, addr: usize, value: u32) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    fn put_u64(&mut self, addr: usize, value: u64) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    fn put_i64(&mut self, addr: usize, value: i64) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    fn mark_heap(&mut self, addr: usize, len: usize) {
        for offset in 0..len {
            self.heap.insert(addr + offset);
        }
    }

    fn set_division(&mut self, unit_ptr: usize, division_id: u32) {
        self.divisions.insert(unit_ptr, division_id);
    }
}

impl PassiveCombatMemory for FakeMemory {
    fn read_u32(&self, addr: usize) -> Option<u32> {
        let mut buf = [0u8; 4];
        for (index, slot) in buf.iter_mut().enumerate() {
            *slot = *self.bytes.get(&(addr + index))?;
        }
        Some(u32::from_le_bytes(buf))
    }

    fn read_u64(&self, addr: usize) -> Option<u64> {
        let mut buf = [0u8; 8];
        for (index, slot) in buf.iter_mut().enumerate() {
            *slot = *self.bytes.get(&(addr + index))?;
        }
        Some(u64::from_le_bytes(buf))
    }

    fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        for index in 0..len {
            out.push(*self.bytes.get(&(addr + index))?);
        }
        Some(out)
    }

    fn is_heap_pointer(&self, addr: usize) -> bool {
        self.heap.contains(&addr)
    }

    fn division_id(&self, wrapper_ptr: usize) -> Option<u32> {
        self.divisions.get(&wrapper_ptr).copied()
    }
}

const SIDE: usize = 0x1_0000;
const UNIT_VECTOR: usize = 0x2_0000;
const UNIT_A: usize = 0x3_0000;
const MODIFIER_VECTOR: usize = 0x4_0000;
const LEADER_TABLE: usize = 0x5_0000;

fn put_identity_accumulators(memory: &mut FakeMemory, unit: usize) {
    memory.put_i64(unit + UNIT_OFF_FOLD_ATTACK, MODIFIER_IDENTITY);
    memory.put_i64(unit + UNIT_OFF_FOLD_DEFENCE, MODIFIER_IDENTITY);
    memory.put_i64(unit + UNIT_OFF_SCALER_ATTACK, MODIFIER_IDENTITY);
    memory.put_i64(unit + UNIT_OFF_SCALER_DEFENCE, MODIFIER_IDENTITY);
}

fn put_modifier_header(
    memory: &mut FakeMemory,
    unit: usize,
    data: usize,
    capacity: u32,
    count: u32,
) {
    memory.put_u64(unit + UNIT_OFF_MODIFIER_DATA, data as u64);
    memory.put_u32(unit + UNIT_OFF_MODIFIER_CAPACITY, capacity);
    memory.put_u32(unit + UNIT_OFF_MODIFIER_COUNT, count);
}

fn put_modifier_entries(memory: &mut FakeMemory, data: usize, entries: &[CombatModifierEntry]) {
    memory.mark_heap(data, entries.len() * MODIFIER_ENTRY_STRIDE);
    for (index, entry) in entries.iter().enumerate() {
        let base = data + index * MODIFIER_ENTRY_STRIDE;
        memory.put_u32(base, entry.id);
        memory.put_u32(base + 4, 0);
        memory.put_i64(base + MODIFIER_ENTRY_OFF_VALUE_A, entry.value_a_raw);
        memory.put_i64(base + MODIFIER_ENTRY_OFF_VALUE_B, entry.value_b_raw);
    }
}

fn put_leader_table(memory: &mut FakeMemory, side: usize, data: usize, entries: &[(u64, u32)]) {
    memory.put_u64(side + SIDE_OFF_LEADER_DATA, data as u64);
    memory.put_u32(side + SIDE_OFF_LEADER_CAPACITY, entries.len().max(1) as u32);
    memory.put_u32(side + SIDE_OFF_LEADER_COUNT, entries.len() as u32);
    memory.mark_heap(data, entries.len().max(1) * LEADER_ENTRY_STRIDE);
    for (index, (handle, tally)) in entries.iter().enumerate() {
        let base = data + index * LEADER_ENTRY_STRIDE;
        memory.put_u64(base, *handle);
        memory.put_u32(base + 8, *tally);
    }
}

fn side_with_one_unit(entries: &[CombatModifierEntry], leader: &[(u64, u32)]) -> FakeMemory {
    let mut memory = FakeMemory::new();
    memory.mark_heap(SIDE, 0x600);
    memory.mark_heap(UNIT_VECTOR, 0x100);
    memory.mark_heap(UNIT_A, 0x400);
    memory.set_division(UNIT_A, 7788);

    memory.put_u64(SIDE + SIDE_OFF_UNIT_DATA, UNIT_VECTOR as u64);
    memory.put_u32(SIDE + SIDE_OFF_UNIT_COUNT, 1);
    memory.put_u64(UNIT_VECTOR, UNIT_A as u64);

    put_identity_accumulators(&mut memory, UNIT_A);
    put_modifier_header(
        &mut memory,
        UNIT_A,
        MODIFIER_VECTOR,
        entries.len().max(1) as u32,
        entries.len() as u32,
    );
    put_modifier_entries(&mut memory, MODIFIER_VECTOR, entries);
    put_leader_table(&mut memory, SIDE, LEADER_TABLE, leader);
    memory
}

fn sample_entries() -> Vec<CombatModifierEntry> {
    vec![
        CombatModifierEntry {
            id: 11,
            value_a_raw: 25_000,
            value_b_raw: -10_000,
        },
        CombatModifierEntry {
            id: 4,
            value_a_raw: -5_000,
            value_b_raw: 30_000,
        },
    ]
}

#[test]
fn unbounded_modifier_vector_read_is_refused() {
    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 3)]);
    put_modifier_header(&mut memory, UNIT_A, MODIFIER_VECTOR, 2, 9);
    assert_eq!(
        read_unit_modifiers(&memory, UNIT_A, 0, 7788),
        Err(CarrierRefusal::ModifierHeaderOutOfRange),
        "count above capacity must refuse, not read"
    );

    put_modifier_header(
        &mut memory,
        UNIT_A,
        MODIFIER_VECTOR,
        MAX_MODIFIER_ENTRIES + 10,
        MAX_MODIFIER_ENTRIES + 1,
    );
    assert_eq!(
        read_unit_modifiers(&memory, UNIT_A, 0, 7788),
        Err(CarrierRefusal::ModifierHeaderOutOfRange),
        "count above the refusal ceiling must refuse, not read"
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 3)]);
    memory.put_u32(SIDE + SIDE_OFF_UNIT_COUNT, MAX_SIDE_UNITS + 1);
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::UnitVectorDisagreesWithCensus),
        "side unit vector must be bounded too"
    );
    let memory = side_with_one_unit(&sample_entries(), &[(0xAB, 3)]);
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &vec![7788; MAX_SIDE_UNITS as usize + 1]),
        Err(CarrierRefusal::UnitVectorHeaderOutOfRange),
        "an over-long census is refused rather than trusted"
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 3)]);
    memory.put_u32(SIDE + SIDE_OFF_LEADER_CAPACITY, 1);
    memory.put_u32(SIDE + SIDE_OFF_LEADER_COUNT, MAX_LEADER_ENTRIES + 1);
    assert_eq!(
        read_side_leader(&memory, SIDE),
        Err(CarrierRefusal::LeaderHeaderOutOfRange),
        "leader table must be bounded too"
    );
}


#[test]
fn modifier_rows_cannot_claim_completeness_without_a_tick_phase_witness() {
    let memory = side_with_one_unit(&sample_entries(), &[(0xAB, 3)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");

    for unit in &read.units {
        assert_eq!(
            unit.witness,
            ModifierTickWitness::NoTickPhaseWitness,
            "the reader has no tick-phase witness and must not claim one"
        );
        assert!(unit.witness.is_provisional());
    }

    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    let row = rows.unit_modifier_rows[0];
    assert_eq!(
        row.witness_code,
        ModifierTickWitness::NoTickPhaseWitness.wire_code(),
        "the provisional typing must survive onto the wire"
    );
    assert_ne!(row.witness_code, 0, "a zero witness is an omitted witness");

    let stripped = UnitModifierRow {
        witness_code: 0,
        ..row
    };
    assert!(
        validate_unit_modifier_row(&stripped).is_err(),
        "a row with no witness must be rejected, not silently treated as proven"
    );

    let mut payload = Vec::new();
    append_unit_modifier_row(&mut payload, &row);
    assert_eq!(payload.len(), UNIT_MODIFIER_ROW_LEN);
    let decoded = decode_unit_modifier_row(&payload, 0);
    assert_eq!(decoded, row);
    assert!(ModifierTickWitness::from_wire_code(decoded.witness_code)
        .expect("witness decodes")
        .is_provisional());
}

#[test]
fn leader_is_reported_as_last_non_null_never_as_current() {
    let memory = side_with_one_unit(&sample_entries(), &[(0xDEAD_BEEF, 5), (0x11, 2)]);
    let leader = read_side_leader(&memory, SIDE).expect("readable leader table");

    assert_eq!(
        leader.freshness,
        LeaderFreshness::LastNonNullNoFreshnessProof,
        "a passive read cannot prove the cached leader is the current one"
    );
    assert!(leader.freshness.is_stale_permitted());
    assert_eq!(
        leader.selection,
        LeaderSelection::LastNonNull {
            handle: 0xDEAD_BEEF,
            tally: 5
        },
        "entry 0 is the most recent non-null selection"
    );
    assert_eq!(leader.entry_count, 2);

    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    let row = rows.leader_cache_rows[0];
    assert_eq!(
        row.freshness_code,
        LeaderFreshness::LastNonNullNoFreshnessProof.wire_code(),
        "the freshness caveat must travel with the leader, not beside it"
    );
    assert_ne!(row.freshness_code, 0);

    let stripped = LeaderCacheRow {
        freshness_code: 0,
        ..row
    };
    assert!(
        validate_leader_cache_row(&stripped).is_err(),
        "a leader with no freshness typing must be rejected"
    );

    let mut payload = Vec::new();
    append_leader_cache_row(&mut payload, &row);
    assert_eq!(payload.len(), LEADER_CACHE_ROW_LEN);
    assert_eq!(decode_leader_cache_row(&payload, 0), row);
}

#[test]
fn identity_accumulators_are_carried_as_100000_and_never_dropped() {
    let memory = side_with_one_unit(&[], &[(0xAB, 1)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");

    assert_eq!(read.units.len(), 1, "a unit at identity is still a unit");
    let unit = &read.units[0];
    assert!(unit.entries.is_empty());
    assert_eq!(unit.fold_attack_raw, 100_000);
    assert_eq!(unit.fold_defence_raw, 100_000);
    assert_eq!(unit.scaler_attack_raw, 100_000);
    assert_eq!(unit.scaler_defence_raw, 100_000);
    assert_eq!(
        MODIFIER_IDENTITY, 100_000,
    );
    assert!(unit.is_identity());

    assert_eq!(unit.effective_attack_raw(), 100_000);
    assert_eq!(unit.effective_defence_raw(), 100_000);

    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    assert_eq!(
        rows.unit_modifier_rows.len(),
        1,
        "an at-rest unit must not be dropped as 'did not happen'"
    );
    let row = rows.unit_modifier_rows[0];
    assert_eq!(row.entry_count, 0);
    assert_eq!(row.fold_attack_raw, 100_000);
    assert_eq!(row.fold_defence_raw, 100_000);
    assert_eq!(row.scaler_attack_raw, 100_000);
    assert_eq!(row.scaler_defence_raw, 100_000);

    let mut payload = Vec::new();
    append_unit_modifier_row(&mut payload, &row);
    assert_eq!(
        decode_unit_modifier_row(&payload, 0).fold_attack_raw,
        100_000
    );
}

#[test]
fn empty_leader_cache_is_never_reported_as_an_authenticated_no_leader() {
    let memory = side_with_one_unit(&sample_entries(), &[]);
    let leader = read_side_leader(&memory, SIDE).expect("readable empty leader table");

    assert_eq!(
        leader.selection,
        LeaderSelection::NeverRecorded,
        "an empty cache is an absence of evidence, not a no-leader result"
    );
    assert_eq!(
        leader.selection.handle(),
        None,
        "there is no handle to report"
    );
    assert_eq!(leader.selection.tally(), None);
    assert_eq!(leader.entry_count, 0);

    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    let row = rows.leader_cache_rows[0];
    assert_eq!(
        row.selection_present, 0,
        "the wire must mark the absence, not encode leader 0"
    );

    let smuggled = LeaderCacheRow {
        selection_present: 0,
        leader_handle: 1234,
        ..row
    };
    assert!(
        validate_leader_cache_row(&smuggled).is_err(),
        "absence must not carry a handle"
    );
    let counted = LeaderCacheRow {
        selection_present: 0,
        entry_count: 3,
        ..row
    };
    assert!(
        validate_leader_cache_row(&counted).is_err(),
        "absence must not count entries"
    );
    let orphaned = LeaderCacheRow {
        selection_present: 1,
        entry_count: 0,
        ..row
    };
    assert!(
        validate_leader_cache_row(&orphaned).is_err(),
        "a present selection needs an entry behind it"
    );

    assert!(matches!(LeaderSelection::NeverRecorded.handle(), None));
}

#[test]
fn modifier_entries_are_carried_in_engine_append_order() {
    let entries = sample_entries();
    let memory = side_with_one_unit(&entries, &[(0xAB, 1)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    assert_eq!(read.units[0].entries, entries, "no sorting, no dedup");

    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    assert_eq!(rows.modifier_entry_rows.len(), entries.len());
    for (index, row) in rows.modifier_entry_rows.iter().enumerate() {
        assert_eq!(row.entry_index, index as u32);
        assert_eq!(row.modifier_id, entries[index].id);
        assert_eq!(row.value_a_raw, entries[index].value_a_raw);
        assert_eq!(row.value_b_raw, entries[index].value_b_raw);
        let mut payload = Vec::new();
        append_modifier_entry_row(&mut payload, row);
        assert_eq!(payload.len(), MODIFIER_ENTRY_ROW_LEN);
        assert_eq!(&decode_modifier_entry_row(&payload, 0), row);
    }
}

#[test]
fn effective_values_follow_the_engine_non_raw_getters() {
    let mut memory = side_with_one_unit(&[], &[(0xAB, 1)]);
    memory.put_i64(UNIT_A + UNIT_OFF_FOLD_ATTACK, 150_000);
    memory.put_i64(UNIT_A + UNIT_OFF_FOLD_DEFENCE, 80_000);
    memory.put_i64(UNIT_A + UNIT_OFF_SCALER_ATTACK, 200_000);
    memory.put_i64(UNIT_A + UNIT_OFF_SCALER_DEFENCE, 50_000);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    let unit = &read.units[0];

    assert_eq!(unit.effective_attack_raw(), 200_000 * 150_000 / 100_000);
    assert_eq!(unit.effective_defence_raw(), 50_000 * 80_000 / 100_000);
    assert!(!unit.is_identity(), "these are not identity values");

    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    let row = rows.unit_modifier_rows[0];
    assert_eq!(row.scaler_attack_raw, 200_000);
    assert_eq!(row.scaler_defence_raw, 50_000);
}

#[test]
fn unreadable_structures_fail_closed_with_a_typed_reason() {
    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    put_modifier_header(&mut memory, UNIT_A, 0xDEAD_0000, 4, 2);
    let refusal = read_side_modifiers(&memory, SIDE, &[7788]).expect_err("must refuse");
    assert_eq!(refusal, CarrierRefusal::ModifierDataNotHeapPointer);

    let mut rows = CarrierRows::default();
    rows.push_refusal(42, 9, 1, refusal);
    assert!(rows.unit_modifier_rows.is_empty(), "no rows from a refusal");
    let row = rows.refusal_rows[0];
    assert_eq!(row.refusal_code, refusal.wire_code());
    assert_ne!(row.refusal_code, 0);
    assert!(validate_carrier_refusal_row(&row).is_ok());
    assert!(
        validate_carrier_refusal_row(&CarrierRefusalRow {
            refusal_code: 0,
            ..row
        })
        .is_err(),
        "an untyped refusal is not a refusal"
    );

    let mut payload = Vec::new();
    append_carrier_refusal_row(&mut payload, &row);
    assert_eq!(payload.len(), CARRIER_REFUSAL_ROW_LEN);
    assert_eq!(decode_carrier_refusal_row(&payload, 0), row);
}

#[test]
fn side_and_unit_identity_are_validated_before_any_field_is_believed() {
    let memory = FakeMemory::new();
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::SideNotHeapPointer)
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    memory.divisions.clear();
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::UnitIdentityInvalid),
        "an unauthenticated element must not be given a division id"
    );
}

#[test]
fn a_header_that_changes_during_the_read_is_refused() {
    struct ShiftingMemory {
        inner: FakeMemory,
        seen: std::cell::Cell<u32>,
    }
    impl PassiveCombatMemory for ShiftingMemory {
        fn read_u32(&self, addr: usize) -> Option<u32> {
            if addr == UNIT_A + UNIT_OFF_MODIFIER_COUNT {
                let nth = self.seen.get();
                self.seen.set(nth + 1);
                return Some(if nth == 0 { 2 } else { 3 });
            }
            self.inner.read_u32(addr)
        }
        fn read_u64(&self, addr: usize) -> Option<u64> {
            self.inner.read_u64(addr)
        }
        fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
            self.inner.read_bytes(addr, len)
        }
        fn is_heap_pointer(&self, addr: usize) -> bool {
            self.inner.is_heap_pointer(addr)
        }
        fn division_id(&self, wrapper_ptr: usize) -> Option<u32> {
            self.inner.division_id(wrapper_ptr)
        }
    }

    let memory = ShiftingMemory {
        inner: side_with_one_unit(&sample_entries(), &[(0xAB, 1)]),
        seen: std::cell::Cell::new(0),
    };
    assert_eq!(
        read_unit_modifiers(&memory, UNIT_A, 0, 7788),
        Err(CarrierRefusal::ModifierHeaderChangedDuringRead)
    );
}

#[test]
fn discovery_uses_the_sim_side_unit_vector_not_the_panel() {
    assert_eq!(SIDE_OFF_UNIT_DATA, 0x20);
    assert_eq!(SIDE_OFF_UNIT_COUNT, 0x2C);

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    const UNIT_B: usize = 0x6_0000;
    memory.mark_heap(UNIT_B, 0x400);
    memory.set_division(UNIT_B, 9911);
    memory.put_u64(UNIT_VECTOR + 8, UNIT_B as u64);
    memory.put_u32(SIDE + SIDE_OFF_UNIT_COUNT, 2);
    put_identity_accumulators(&mut memory, UNIT_B);
    put_modifier_header(&mut memory, UNIT_B, MODIFIER_VECTOR, 2, 0);

    let read = read_side_modifiers(&memory, SIDE, &[7788, 9911]).expect("readable side");
    assert_eq!(
        read.units.iter().map(|u| u.division_id).collect::<Vec<_>>(),
        vec![7788, 9911],
        "every unit in the sim vector is covered, unclicked ones included"
    );
    assert_eq!(read.units[1].ordinal, 1);
}

#[test]
fn carrier_rows_cover_both_sides_with_both_caveats_present() {
    let memory = side_with_one_unit(&sample_entries(), &[(0x77, 4)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");

    let mut rows = CarrierRows::default();
    assert!(rows.is_empty());
    rows.push_side(42, 9, 1, &read);
    rows.push_side(42, 9, 2, &read);
    assert!(!rows.is_empty());

    assert_eq!(rows.unit_modifier_rows.len(), 2);
    assert_eq!(rows.leader_cache_rows.len(), 2);
    assert_eq!(rows.modifier_entry_rows.len(), 4);
    for row in &rows.unit_modifier_rows {
        assert!(validate_unit_modifier_row(row).is_ok());
        assert_ne!(row.witness_code, 0);
    }
    for row in &rows.leader_cache_rows {
        assert!(validate_leader_cache_row(row).is_ok());
        assert_ne!(row.freshness_code, 0);
    }
    assert_eq!(
        rows.leader_cache_rows
            .iter()
            .map(|r| r.side_wire_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn honesty_gap_wire_codes_round_trip_and_reject_unknowns() {
    for witness in [
        ModifierTickWitness::NoTickPhaseWitness,
        ModifierTickWitness::ProvenQuiescent,
    ] {
        assert_eq!(
            ModifierTickWitness::from_wire_code(witness.wire_code()),
            Some(witness)
        );
    }
    assert_eq!(ModifierTickWitness::from_wire_code(0), None);
    assert_eq!(ModifierTickWitness::from_wire_code(99), None);

    for freshness in [
        LeaderFreshness::LastNonNullNoFreshnessProof,
        LeaderFreshness::ProvenCurrent,
    ] {
        assert_eq!(
            LeaderFreshness::from_wire_code(freshness.wire_code()),
            Some(freshness)
        );
    }
    assert_eq!(LeaderFreshness::from_wire_code(0), None);
    assert_eq!(LeaderFreshness::from_wire_code(99), None);

    assert!(!ModifierTickWitness::ProvenQuiescent.is_provisional());
    assert!(!LeaderFreshness::ProvenCurrent.is_stale_permitted());
}

#[test]
fn reserved_proof_codes_are_refused_until_something_can_prove_them() {
    let memory = side_with_one_unit(&sample_entries(), &[(0x77, 4)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);

    let claimed = UnitModifierRow {
        witness_code: ModifierTickWitness::ProvenQuiescent.wire_code(),
        ..rows.unit_modifier_rows[0]
    };
    assert!(
        validate_unit_modifier_row(&claimed).is_err(),
        "ProvenQuiescent is unprovable in this build and must not encode"
    );

    let claimed = LeaderCacheRow {
        freshness_code: LeaderFreshness::ProvenCurrent.wire_code(),
        ..rows.leader_cache_rows[0]
    };
    assert!(
        validate_leader_cache_row(&claimed).is_err(),
        "ProvenCurrent is unprovable in this build and must not encode"
    );

    let mut payload = Vec::new();
    append_carrier_trailer(&mut payload, &rows);
    let witness_at = CARRIER_TRAILER_HEADER_LEN + 60;
    payload[witness_at..witness_at + 4].copy_from_slice(
        &ModifierTickWitness::ProvenQuiescent
            .wire_code()
            .to_le_bytes(),
    );
    assert!(
        decode_carrier_trailer(&payload, 0).is_err(),
        "a trailer asserting a proven read must be rejected on decode"
    );
}

#[test]
fn a_zero_handle_in_an_occupied_leader_slot_is_refused() {
    let memory = side_with_one_unit(&sample_entries(), &[(0, 3)]);
    assert_eq!(
        read_side_leader(&memory, SIDE),
        Err(CarrierRefusal::LeaderEntryHandleInvalid),
        "the engine never stores a null selection, so this read is not trustworthy"
    );

    let row = LeaderCacheRow {
        combat_id: 42,
        generation: 9,
        side_wire_id: 1,
        selection_present: 1,
        leader_handle: 0,
        tally: 3,
        entry_count: 1,
        freshness_code: LeaderFreshness::LastNonNullNoFreshnessProof.wire_code(),
    };
    assert!(
        validate_leader_cache_row(&row).is_err(),
        "a present selection with a zero handle reads as an authenticated no-leader"
    );
}

#[test]
fn an_empty_unit_vector_is_still_re_read_before_it_is_believed() {
    struct GrowingMemory {
        inner: FakeMemory,
        seen: std::cell::Cell<u32>,
    }
    impl PassiveCombatMemory for GrowingMemory {
        fn read_u32(&self, addr: usize) -> Option<u32> {
            if addr == SIDE + SIDE_OFF_UNIT_COUNT {
                let nth = self.seen.get();
                self.seen.set(nth + 1);
                return Some(if nth == 0 { 0 } else { 1 });
            }
            self.inner.read_u32(addr)
        }
        fn read_u64(&self, addr: usize) -> Option<u64> {
            self.inner.read_u64(addr)
        }
        fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
            self.inner.read_bytes(addr, len)
        }
        fn is_heap_pointer(&self, addr: usize) -> bool {
            self.inner.is_heap_pointer(addr)
        }
        fn division_id(&self, wrapper_ptr: usize) -> Option<u32> {
            self.inner.division_id(wrapper_ptr)
        }
    }

    let memory = GrowingMemory {
        inner: side_with_one_unit(&[], &[(0xAB, 1)]),
        seen: std::cell::Cell::new(0),
    };
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[]),
        Err(CarrierRefusal::UnitVectorHeaderChangedDuringRead),
        "an empty observation must be re-read before it is published as empty"
    );
}

#[test]
fn a_frame_that_would_outgrow_the_decoder_is_refused_not_truncated() {
    let memory = side_with_one_unit(&sample_entries(), &[(0x77, 4)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");

    let mut rows = CarrierRows::default();
    rows.unit_modifier_rows = vec![
        UnitModifierRow {
            combat_id: 1,
            generation: 1,
            side_wire_id: 1,
            ordinal: 0,
            division_id: 1,
            entry_count: 0,
            fold_attack_raw: MODIFIER_IDENTITY,
            fold_defence_raw: MODIFIER_IDENTITY,
            scaler_attack_raw: MODIFIER_IDENTITY,
            scaler_defence_raw: MODIFIER_IDENTITY,
            witness_code: ModifierTickWitness::NoTickPhaseWitness.wire_code(),
        };
        MAX_UNIT_MODIFIER_ROWS
    ];
    rows.push_side(42, 9, 1, &read);

    assert_eq!(
        rows.unit_modifier_rows.len(),
        MAX_UNIT_MODIFIER_ROWS,
        "the ceiling must hold"
    );
    assert_eq!(
        rows.refusal_rows.len(),
        1,
        "the refused side must be named, not silently dropped"
    );
    assert_eq!(
        rows.refusal_rows[0].refusal_code,
        CarrierRefusal::CarrierRowBudgetExceeded.wire_code()
    );

    let mut rows = CarrierRows::default();
    rows.push_side(42, 9, 1, &read);
    let mut payload = Vec::new();
    append_carrier_trailer(&mut payload, &rows);
    assert_eq!(payload.len(), carrier_trailer_len(&rows));
    let (decoded, end) = decode_carrier_trailer(&payload, 0).expect("producer output decodes");
    assert_eq!(end, payload.len());
    assert_eq!(decoded, rows);
}

#[test]
fn the_unit_vector_read_is_bounded_by_the_census_not_by_an_unproven_capacity() {
    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    memory.mark_heap(UNIT_VECTOR, 0x400);
    for slot in 1..64usize {
        memory.put_u64(UNIT_VECTOR + slot * 8, UNIT_A as u64);
    }
    memory.put_u32(SIDE + SIDE_OFF_UNIT_COUNT, 64);
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::UnitVectorDisagreesWithCensus),
        "a readable over-long vector is still not an in-bounds read"
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    memory.set_division(UNIT_A, 4321);
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::UnitVectorDisagreesWithCensus),
        "the units must be the ones D21 censused, not merely as many"
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    memory.put_u32(SIDE + SIDE_OFF_UNIT_COUNT, 0);
    assert_eq!(
        read_side_modifiers(&memory, SIDE, &[7788]),
        Err(CarrierRefusal::UnitVectorDisagreesWithCensus)
    );

    let mut memory = side_with_one_unit(&sample_entries(), &[(0xAB, 1)]);
    memory.put_u32(SIDE + 0x28, u32::MAX);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    assert_eq!(read.units.len(), 1);
    memory.put_u32(SIDE + 0x28, 0);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    assert_eq!(read.units.len(), 1, "side+0x28 must not be load-bearing");
}

#[test]
fn the_encoder_refuses_every_row_set_the_decoder_would_refuse() {
    fn unit_row() -> UnitModifierRow {
        UnitModifierRow {
            combat_id: 1,
            generation: 1,
            side_wire_id: 1,
            ordinal: 0,
            division_id: 1,
            entry_count: 0,
            fold_attack_raw: MODIFIER_IDENTITY,
            fold_defence_raw: MODIFIER_IDENTITY,
            scaler_attack_raw: MODIFIER_IDENTITY,
            scaler_defence_raw: MODIFIER_IDENTITY,
            witness_code: ModifierTickWitness::NoTickPhaseWitness.wire_code(),
        }
    }
    fn entry_row() -> ModifierEntryRow {
        ModifierEntryRow {
            combat_id: 1,
            generation: 1,
            side_wire_id: 1,
            unit_ordinal: 0,
            entry_index: 0,
            modifier_id: 3,
            value_a_raw: 1,
            value_b_raw: 2,
        }
    }
    fn leader_row() -> LeaderCacheRow {
        LeaderCacheRow {
            combat_id: 1,
            generation: 1,
            side_wire_id: 1,
            selection_present: 1,
            leader_handle: 9,
            tally: 1,
            entry_count: 1,
            freshness_code: LeaderFreshness::LastNonNullNoFreshnessProof.wire_code(),
        }
    }
    fn refusal_row() -> CarrierRefusalRow {
        CarrierRefusalRow {
            combat_id: 1,
            generation: 1,
            side_wire_id: 1,
            refusal_code: CarrierRefusal::FoldsUnreadable.wire_code(),
        }
    }

    let at_unit_ceiling = CarrierRows {
        unit_modifier_rows: vec![unit_row(); MAX_UNIT_MODIFIER_ROWS],
        ..Default::default()
    };
    assert!(validate_carrier_rows(&at_unit_ceiling).is_ok());
    let over_units = CarrierRows {
        unit_modifier_rows: vec![unit_row(); MAX_UNIT_MODIFIER_ROWS + 1],
        ..Default::default()
    };
    assert!(validate_carrier_rows(&over_units).is_err(), "unit ceiling");

    let over_entries = CarrierRows {
        modifier_entry_rows: vec![entry_row(); MAX_MODIFIER_ENTRY_ROWS + 1],
        ..Default::default()
    };
    assert!(
        validate_carrier_rows(&over_entries).is_err(),
        "entry ceiling"
    );

    let over_leaders = CarrierRows {
        leader_cache_rows: vec![leader_row(); MAX_LEADER_CACHE_ROWS + 1],
        ..Default::default()
    };
    assert!(
        validate_carrier_rows(&over_leaders).is_err(),
        "leader ceiling"
    );

    let over_refusals = CarrierRows {
        refusal_rows: vec![refusal_row(); MAX_CARRIER_REFUSAL_ROWS + 1],
        ..Default::default()
    };
    assert!(
        validate_carrier_rows(&over_refusals).is_err(),
        "refusal ceiling"
    );

    let refused = std::panic::catch_unwind(|| {
        let mut payload = Vec::new();
        append_carrier_trailer(&mut payload, &over_units);
    });
    assert!(
        refused.is_err(),
        "append_carrier_trailer must refuse an over-budget row set"
    );
}

#[test]
fn push_side_respects_the_entry_and_leader_ceilings_too() {
    let memory = side_with_one_unit(&sample_entries(), &[(0x77, 4)]);
    let read = read_side_modifiers(&memory, SIDE, &[7788]).expect("readable side");
    let entries_per_side = read.units.iter().map(|u| u.entries.len()).sum::<usize>();
    assert!(entries_per_side > 0, "fixture must carry entries");

    let mut rows = CarrierRows {
        modifier_entry_rows: vec![
            ModifierEntryRow {
                combat_id: 1,
                generation: 1,
                side_wire_id: 1,
                unit_ordinal: 0,
                entry_index: 0,
                modifier_id: 1,
                value_a_raw: 0,
                value_b_raw: 0,
            };
            MAX_MODIFIER_ENTRY_ROWS
        ],
        ..Default::default()
    };
    rows.push_side(42, 9, 1, &read);
    assert!(
        rows.unit_modifier_rows.is_empty(),
        "entry ceiling must hold"
    );
    assert_eq!(
        rows.refusal_rows[0].refusal_code,
        CarrierRefusal::CarrierRowBudgetExceeded.wire_code()
    );

    let mut rows = CarrierRows {
        leader_cache_rows: vec![
            LeaderCacheRow {
                combat_id: 1,
                generation: 1,
                side_wire_id: 1,
                selection_present: 1,
                leader_handle: 9,
                tally: 1,
                entry_count: 1,
                freshness_code: LeaderFreshness::LastNonNullNoFreshnessProof.wire_code(),
            };
            MAX_LEADER_CACHE_ROWS
        ],
        ..Default::default()
    };
    rows.push_side(42, 9, 1, &read);
    assert!(
        rows.unit_modifier_rows.is_empty(),
        "leader ceiling must hold"
    );
    assert_eq!(
        rows.refusal_rows[0].refusal_code,
        CarrierRefusal::CarrierRowBudgetExceeded.wire_code()
    );

    let mut rows = CarrierRows {
        refusal_rows: vec![
            CarrierRefusalRow {
                combat_id: 1,
                generation: 1,
                side_wire_id: 1,
                refusal_code: CarrierRefusal::FoldsUnreadable.wire_code(),
            };
            MAX_CARRIER_REFUSAL_ROWS
        ],
        ..Default::default()
    };
    rows.push_refusal(42, 9, 1, CarrierRefusal::FoldsUnreadable);
    assert_eq!(rows.refusal_rows.len(), MAX_CARRIER_REFUSAL_ROWS);
    assert!(validate_carrier_rows(&rows).is_ok());
}
