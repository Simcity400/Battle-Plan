use super::*;
use std::collections::BTreeMap;

const MODULE: usize = 0x140000000;
const PRIMARY: usize = 0x20000000000;
const BODY: usize = PRIMARY + 0x10;
const REGISTRY: usize = PRIMARY + 0x1000;
const SLOTS: usize = PRIMARY + 0x2000;
const DIV_ID: u32 = 42;
const KEY: DeploymentKey = DeploymentKey {
    native_type: 1,
    id: 1,
};

// Each fixture supplies only individually reached fields. Missing fields fail
// instead of synthesizing zero; the log proves both skipped reads and closes.
#[derive(Default)]
struct Memory {
    fields: BTreeMap<usize, Vec<u8>>,
    overrides: BTreeMap<(usize, usize), Option<Vec<u8>>>,
    calls: Vec<(usize, usize)>,
}
impl Memory {
    fn put(&mut self, address: usize, bytes: impl AsRef<[u8]>) {
        self.fields.insert(address, bytes.as_ref().to_vec());
    }
    fn count(&self, address: usize) -> usize {
        self.calls.iter().filter(|(a, _)| *a == address).count()
    }
    fn read(&mut self, address: usize, out: &mut [u8]) -> bool {
        let occurrence = self.count(address) + 1;
        self.calls.push((address, out.len()));
        let bytes = match self.overrides.get(&(address, occurrence)) {
            Some(bytes) => bytes.as_ref(),
            None => self.fields.get(&address),
        };
        match bytes {
            Some(bytes) if bytes.len() == out.len() => {
                out.copy_from_slice(bytes);
                true
            }
            _ => {
                out.fill(0xa5); // Failed copies never become observed bytes.
                false
            }
        }
    }
    fn division(key: DeploymentKey) -> Self {
        let mut memory = Self::default();
        memory.put(PRIMARY, (MODULE as u64 + 0x2943ad0).to_le_bytes());
        memory.put(BODY, (MODULE as u64 + 0x2943cb0).to_le_bytes());
        memory.put(
            BODY + 8,
            [DIVISION_TYPE.to_le_bytes(), DIV_ID.to_le_bytes()].concat(),
        );
        memory.put(PRIMARY + 0x32c, key.native_type.to_le_bytes());
        memory.put(PRIMARY + 0x330, key.id.to_le_bytes());
        memory
    }
    fn table(key: DeploymentKey, mask: u32, overflow: u8) -> Self {
        let mut memory = Self::division(key);
        memory.put(
            root_slot(MODULE, key.native_type).unwrap(),
            (REGISTRY as u64).to_le_bytes(),
        );
        memory.put(REGISTRY + 8, (SLOTS as u64).to_le_bytes());
        memory.put(REGISTRY + 0x14, mask.to_le_bytes());
        memory.put(REGISTRY + 0x18, [overflow]);
        memory
    }
    fn slot(&mut self, index: usize, distance: u8, id: u32, native_type: u32) {
        let slot = SLOTS + 24 * index;
        self.put(slot + 4, [distance]);
        self.put(slot + 0xc, id.to_le_bytes());
        self.put(slot + 8, native_type.to_le_bytes());
    }
    fn run(&mut self, budget: &Budget) -> DeploymentObservation {
        sample(
            MODULE,
            BODY,
            DIVISION_TYPE,
            DIV_ID,
            budget,
            &mut |a, out| self.read(a, out),
        )
    }
    fn assert_paired(&self) {
        let mut counts = BTreeMap::new();
        for call in &self.calls {
            *counts.entry(*call).or_insert(0) += 1;
        }
        assert!(counts.values().all(|count| *count == 2), "{counts:?}");
    }
}

#[test]
fn zero_pair_short_circuits_and_single_zero_words_select_exact_unsigned_tiers() {
    let mut memory = Memory::division(DeploymentKey {
        native_type: 0,
        id: 0,
    });
    let budget = Budget::default();
    assert_eq!(memory.run(&budget).result, Ok(Resolved::ZeroPair));
    assert_eq!(budget.verification_bytes(), 64);
    assert_eq!(budget.retained_bytes(), 2 * size_of::<Block>());
    assert_eq!(memory.calls.len(), 10);
    memory.assert_paired();

    for (native_type, id, rva) in [
        (0, 1, 0x3438e60),
        (99, 0, 0x3439178),
        (100, 0, 0x3438e58),
        (0x1268, 1, 0x3438e58),
        (0x1269, 1, 0x3438e50),
        (u32::MAX, u32::MAX, 0x3438e50),
    ] {
        let key = DeploymentKey { native_type, id };
        let mut memory = Memory::division(key);
        memory.put(MODULE + rva, 0u64.to_le_bytes());
        let budget = Budget::default();
        let observed = memory.run(&budget);
        assert_eq!(observed.key, Ok(key));
        assert_eq!(observed.result, Ok(Resolved::RegistryNull));
        assert_eq!(memory.count(MODULE + rva), 2);
        assert_eq!(budget.verification_bytes(), 80);
        memory.assert_paired();
    }
}

#[test]
fn golden_hash_signed_indexes_and_wrapping_sentinel_preserve_native_bits() {
    // Fixed full-width products/folds from the pinned IMUL/MOVSXD/XOR law.
    for (native_type, id, folded) in [
        (1, 1, 0),
        (u32::MAX, u32::MAX, 1231231556),
        (0x80000000, 1, -916252091),
        (1, u32::MAX, 609931570),
        (0, 0x80000000, -1531867870),
    ] {
        assert_eq!(
            probe_bucket(DeploymentKey { native_type, id }, u32::MAX),
            folded
        );
    }
    assert_eq!(sentinel_index(u32::MAX, 0), 0);
    assert_eq!(sentinel_index(0x7fffffff, 0), i32::MIN);
    assert_eq!(sentinel_index(u32::MAX, 255), 255);
    assert_eq!(signed_slot(SLOTS, -1), Ok(SLOTS - 24));
    assert_eq!(signed_slot(0, -1), Err(Refusal::AddressOverflow));
    assert_eq!(
        signed_slot(usize::MAX - 23, 1),
        Err(Refusal::AddressOverflow)
    );
    assert_eq!(root_slot(usize::MAX, 0), Err(Refusal::AddressOverflow));
}

#[test]
fn distance_miss_reads_no_key_or_value_and_prices_only_reached_spans() {
    let mut memory = Memory::table(KEY, 7, 5);
    memory.put(SLOTS + 4, [0]);
    let budget = Budget::default();
    assert_eq!(
        memory.run(&budget).result,
        Ok(Resolved::Miss(Miss::Distance))
    );
    assert_eq!(budget.verification_bytes(), 108);
    assert_eq!(memory.count(SLOTS + 0xc), 0);
    assert_eq!(memory.count(SLOTS + 8), 0);
    assert_eq!(memory.count(SLOTS + 0x10), 0);
    memory.assert_paired();
}

#[test]
fn full_high_type_bits_participate_in_key_match_after_hash_selection() {
    let key = DeploymentKey {
        native_type: 0x80000000,
        id: 1,
    };
    let mut memory = Memory::table(key, 7, 5);
    memory.slot(5, 1, 1, 0); // Same low type bits and ID, different full type.
    memory.slot(6, 2, 1, 0x80000000);
    memory.put(SLOTS + 6 * 24 + 0x10, 1u64.to_le_bytes());
    let observed = memory.run(&Budget::default());
    assert_eq!(observed.key, Ok(key));
    assert_eq!(observed.result, Ok(Resolved::ResolvedNonNull));
    assert_eq!(memory.count(SLOTS + 5 * 24 + 0x10), 0);
    assert_eq!(memory.count(SLOTS + 6 * 24 + 0x10), 2);
    memory.assert_paired();
}

#[test]
fn refused_registry_or_slot_base_is_distinct_from_an_observed_null_root() {
    for root in [true, false] {
        let mut memory = Memory::table(KEY, 7, 5);
        let address = if root {
            MODULE + 0x3438e68
        } else {
            REGISTRY + 8
        };
        memory.put(address, 1u64.to_le_bytes());
        let observed = memory.run(&Budget::default());
        assert_eq!(observed.key, Ok(KEY));
        assert_eq!(
            observed.result,
            Err(Refusal::RangeRefused(if root {
                Field::RegistryRoot
            } else {
                Field::RegistrySlots
            }))
        );
        assert_eq!(memory.count(1), 0);
        memory.assert_paired();
    }
}

#[test]
fn id_mismatch_skips_type_and_type_mismatch_skips_value() {
    let mut memory = Memory::table(KEY, 7, 5);
    memory.slot(0, 1, 2, 1); // ID mismatch.
    memory.slot(1, 2, 1, 2); // Type mismatch.
    memory.put(SLOTS + 2 * 24 + 4, [2]); // 2 < CL 3.
    let budget = Budget::default();
    assert_eq!(
        memory.run(&budget).result,
        Ok(Resolved::Miss(Miss::Distance))
    );
    assert_eq!(memory.count(SLOTS + 8), 0);
    assert_eq!(memory.count(SLOTS + 24 + 8), 2);
    assert_eq!(memory.count(SLOTS + 0x10), 0);
    assert_eq!(memory.count(SLOTS + 24 + 0x10), 0);
    assert_eq!(budget.verification_bytes(), 136);
    memory.assert_paired();
}

#[test]
fn matched_value_nullness_is_distinct_and_nonzero_targets_are_never_followed() {
    for (value, expected) in [
        (0, Resolved::MatchedNull),
        (1, Resolved::ResolvedNonNull),
        (u64::MAX, Resolved::ResolvedNonNull),
    ] {
        let mut memory = Memory::table(KEY, 7, 5);
        memory.slot(0, 1, 1, 1);
        memory.put(SLOTS + 0x10, value.to_le_bytes());
        let budget = Budget::default();
        let observed = memory.run(&budget);
        assert_eq!(observed.key, Ok(KEY));
        assert_eq!(observed.result, Ok(expected));
        assert_eq!(
            observed.header,
            Some(RawHeader {
                mask: 7,
                overflow: 5
            })
        );
        assert_eq!(budget.verification_bytes(), 140);
        assert_eq!(budget.retained_bytes(), 3 * size_of::<Block>());
        assert_eq!(memory.calls.len(), 26);
        memory.assert_paired();
    }
}

#[test]
fn matched_sentinel_skips_value_but_mismatched_sentinel_is_not_a_native_miss() {
    for matching in [false, true] {
        let mut memory = Memory::table(KEY, 7, 5);
        for index in 0..13 {
            memory.slot(index, 255, 2, 1);
        }
        memory.slot(13, 255, if matching { 1 } else { 2 }, 1);
        let budget = Budget::default();
        let expected = if matching {
            Ok(Resolved::Miss(Miss::Sentinel))
        } else {
            Err(Refusal::RangeRefused(Field::ProbeDistance))
        };
        assert_eq!(memory.run(&budget).result, expected);
        assert_eq!(memory.count(SLOTS + 13 * 24 + 0x10), 0);
        assert_eq!(memory.count(SLOTS + 14 * 24 + 4), 0);
        memory.assert_paired();
    }
}

#[test]
fn cl_wrap_can_match_at_zero_and_continue_to_one_without_a_fake_probe_limit() {
    for match_at_zero in [false, true] {
        let mut memory = Memory::table(KEY, 511, 11);
        for index in 0..255 {
            memory.slot(index, 255, 2, 1);
        }
        memory.slot(255, 0, if match_at_zero { 1 } else { 2 }, 1);
        memory.put(SLOTS + 255 * 24 + 0x10, 1u64.to_le_bytes());
        memory.put(SLOTS + 256 * 24 + 4, [0]);
        let budget = Budget::default();
        let expected = if match_at_zero {
            Resolved::ResolvedNonNull
        } else {
            Resolved::Miss(Miss::Distance)
        };
        assert_eq!(memory.run(&budget).result, Ok(expected));
        assert_eq!(memory.count(SLOTS + 255 * 24 + 0xc), 2);
        assert_eq!(
            memory.count(SLOTS + 256 * 24 + 4),
            if match_at_zero { 0 } else { 2 }
        );
        memory.assert_paired();
    }
}

#[test]
fn factory_shape_limits_and_static_empty_are_admission_refusals_not_native_misses() {
    for exponent in 3..=30 {
        let mask = (1u32 << exponent) - 1;
        let table = Table::admit(SLOTS, mask, (exponent + 2) as u8).unwrap();
        assert_eq!(
            table.end - SLOTS,
            24 * (mask as usize + exponent as usize + 4)
        );
        assert!(table.holds(table.end - 1, 1));
        assert!(!table.holds(table.end - 1, 2));
        assert!(!table.holds(SLOTS - 1, 1));
    }
    for (mask, overflow) in [
        (0, 0),
        (1, 3),
        (3, 4),
        (6, 5),
        (7, 6),
        (0x7fffffff, 33),
        (u32::MAX, 34),
    ] {
        let mut memory = Memory::table(KEY, mask, overflow);
        assert_eq!(
            memory.run(&Budget::default()).result,
            Err(Refusal::AllocationShapeUnproved)
        );
        assert_eq!(memory.calls.len(), 18);
        memory.assert_paired();
    }
    assert!(matches!(
        Table::admit(usize::MAX - 10, 7, 5),
        Err(Refusal::AddressOverflow)
    ));
    for distance in [0, 1] {
        let mut memory = Memory::table(KEY, 0, 0);
        let slots = MODULE + 0x30a5ca8;
        memory.put(REGISTRY + 8, (slots as u64).to_le_bytes());
        memory.put(slots + 4, [distance]);
        let expected = if distance == 0 {
            Ok(Resolved::Miss(Miss::Distance))
        } else {
            Err(Refusal::UnsupportedStaticShape)
        };
        assert_eq!(memory.run(&Budget::default()).result, expected);
        assert_eq!(memory.count(slots + 0xc), 0);
        memory.assert_paired();
    }
}

#[test]
fn every_reached_read_failure_is_typed_and_still_closes_its_original_address() {
    for (address, field, invalidates_key) in [
        (PRIMARY, Field::PrimaryVtable, true),
        (BODY, Field::BodyVtable, true),
        (BODY + 8, Field::DivisionHandle, true),
        (PRIMARY + 0x32c, Field::DeploymentType, true),
        (PRIMARY + 0x330, Field::DeploymentId, true),
        (MODULE + 0x3438e68, Field::RegistryRoot, false),
        (REGISTRY + 8, Field::RegistrySlots, false),
        (REGISTRY + 0x14, Field::RegistryMask, false),
        (REGISTRY + 0x18, Field::RegistryOverflow, false),
        (SLOTS + 4, Field::ProbeDistance, false),
        (SLOTS + 0xc, Field::ProbeId, false),
        (SLOTS + 8, Field::ProbeType, false),
        (SLOTS + 0x10, Field::MatchedValue, false),
    ] {
        for occurrence in [1, 2] {
            let mut memory = Memory::table(KEY, 7, 5);
            memory.slot(0, 1, 1, 1);
            memory.put(SLOTS + 0x10, 1u64.to_le_bytes());
            memory.overrides.insert((address, occurrence), None);
            let observed = memory.run(&Budget::default());
            assert_eq!(
                observed.result,
                Err(Refusal::Unreadable(field)),
                "{field:?}, {occurrence}"
            );
            assert_eq!(
                observed.key,
                if invalidates_key {
                    Err(Refusal::Unreadable(field))
                } else {
                    Ok(KEY)
                }
            );
            assert_eq!(memory.count(address), 2);
            memory.assert_paired();
        }
    }
}

#[test]
fn changed_identity_handle_header_prefix_and_value_withdraw_only_their_dependents() {
    for (address, field, invalidates_key) in [
        (PRIMARY, Field::PrimaryVtable, true),
        (BODY, Field::BodyVtable, true),
        (BODY + 8, Field::DivisionHandle, true),
        (PRIMARY + 0x32c, Field::DeploymentType, true),
        (PRIMARY + 0x330, Field::DeploymentId, true),
        (MODULE + 0x3438e68, Field::RegistryRoot, false),
        (REGISTRY + 8, Field::RegistrySlots, false),
        (REGISTRY + 0x14, Field::RegistryMask, false),
        (REGISTRY + 0x18, Field::RegistryOverflow, false),
        (SLOTS + 4, Field::ProbeDistance, false),
        (SLOTS + 0xc, Field::ProbeId, false),
        (SLOTS + 24 + 8, Field::ProbeType, false),
        (SLOTS + 24 + 0x10, Field::MatchedValue, false),
    ] {
        let mut memory = Memory::table(KEY, 7, 5);
        memory.slot(0, 1, 2, 1); // Nonmatching prefix is part of the proof.
        memory.slot(1, 2, 1, 1);
        memory.put(SLOTS + 24 + 0x10, 0u64.to_le_bytes());
        let mut changed = memory.fields[&address].clone();
        changed[0] ^= 8;
        memory.overrides.insert((address, 2), Some(changed));
        let observed = memory.run(&Budget::default());
        assert_eq!(observed.result, Err(Refusal::Changed(field)), "{field:?}");
        assert_eq!(
            observed.key,
            if invalidates_key {
                Err(Refusal::Changed(field))
            } else {
                Ok(KEY)
            }
        );
        assert_eq!(
            observed.header,
            Some(RawHeader {
                mask: 7,
                overflow: 5
            })
        );
        memory.assert_paired(); // Closing never follows replacement root/slots.
    }
}

#[test]
fn invalid_original_identity_reads_no_deployment_and_never_reports_null() {
    let mut memory = Memory::division(KEY);
    memory.put(
        BODY + 8,
        [DIVISION_TYPE.to_le_bytes(), (DIV_ID + 1).to_le_bytes()].concat(),
    );
    let observed = memory.run(&Budget::default());
    assert_eq!(observed.key, Err(Refusal::InvalidIdentity));
    assert_eq!(observed.result, Err(Refusal::InvalidIdentity));
    assert_eq!(memory.count(PRIMARY + 0x32c), 0);
    memory.assert_paired();
}

#[test]
fn exact_verification_boundary_stops_before_read_and_keeps_key_and_all_closes() {
    let mut memory = Memory::table(KEY, 7, 5);
    memory.slot(0, 1, 1, 1);
    memory.put(SLOTS + 0x10, 1u64.to_le_bytes());
    let budget = Budget::test_limits(139, 16384);
    let observed = memory.run(&budget);
    assert_eq!(observed.key, Ok(KEY));
    assert_eq!(observed.result, Err(Refusal::VerificationBound));
    assert_eq!(memory.count(SLOTS + 0x10), 0);
    assert_eq!(budget.verification_bytes(), 140); // Sticky limit + 1, no refund.
    assert!(budget.reserve_verification(0).is_err());
    memory.assert_paired();
}

#[test]
fn both_sticky_budget_stops_drain_prior_tickets_without_additional_funding() {
    for retained_stop in [false, true] {
        let block_bytes = size_of::<Block>();
        let budget = Budget::test_limits(
            if retained_stop { 100 } else { 64 },
            if retained_stop {
                block_bytes
            } else {
                block_bytes * 2
            },
        );
        let mut trace = Trace::default();
        let mut calls = Vec::new();
        let mut read = |a, out: &mut [u8]| {
            calls.push(a);
            out.fill(7);
            true
        };
        for index in 0..32 {
            assert_eq!(
                trace.field::<1>(&budget, SLOTS + index, Field::ProbeDistance, &mut read),
                Ok([7])
            );
        }
        assert_eq!(
            trace.field::<1>(&budget, SLOTS + 32, Field::ProbeDistance, &mut read),
            Err(if retained_stop {
                Refusal::RetainedBound
            } else {
                Refusal::VerificationBound
            })
        );
        assert_eq!(trace.close(&mut read), Ok(()));
        assert_eq!(calls.len(), 64);
        assert_eq!(
            &calls[32..],
            &(0..32).rev().map(|i| SLOTS + i).collect::<Vec<_>>()
        );
        assert_eq!(
            budget.retained_bytes(),
            if retained_stop {
                block_bytes + 1
            } else {
                block_bytes * 2
            }
        );
        assert_eq!(
            budget.verification_bytes(),
            if retained_stop { 64 } else { 65 }
        );
    }
}

#[test]
fn successive_divisions_consume_the_same_budget_without_row_local_reset_or_refund() {
    let budget = Budget::test_limits(128, 4 * size_of::<Block>());
    for _ in 0..2 {
        let mut memory = Memory::division(DeploymentKey {
            native_type: 0,
            id: 0,
        });
        assert_eq!(memory.run(&budget).result, Ok(Resolved::ZeroPair));
        memory.assert_paired();
    }
    assert_eq!(budget.verification_bytes(), 128);
    assert_eq!(budget.retained_bytes(), 4 * size_of::<Block>());
    let mut memory = Memory::division(KEY);
    let observed = memory.run(&budget);
    assert_eq!(observed.key, Err(Refusal::RetainedBound));
    assert_eq!(observed.result, Err(Refusal::RetainedBound));
    assert!(memory.calls.is_empty());
    assert_eq!(budget.retained_bytes(), 4 * size_of::<Block>() + 1);
    assert_eq!(budget.verification_bytes(), 128);
}

#[test]
fn trace_allocation_failure_is_fallible_and_preserves_preexisting_close_tickets() {
    unsafe fn refuse_allocation(_: Layout) -> *mut u8 {
        std::ptr::null_mut()
    }
    for previous in [0, 32] {
        let budget = Budget::default();
        let mut trace = Trace::default();
        let mut calls = 0;
        let mut read = |_, out: &mut [u8]| {
            calls += 1;
            out.fill(3);
            true
        };
        for index in 0..previous {
            trace
                .field::<1>(&budget, SLOTS + index, Field::ProbeDistance, &mut read)
                .unwrap();
        }
        trace.allocate = refuse_allocation;
        assert_eq!(
            trace.field::<1>(&budget, SLOTS + previous, Field::ProbeDistance, &mut read),
            Err(Refusal::AllocationFailed)
        );
        assert_eq!(trace.close(&mut read), Ok(()));
        assert_eq!(calls, previous * 2);
        assert_eq!(budget.verification_bytes(), previous * 2);
        assert_eq!(
            budget.retained_bytes(),
            (previous / 32 + 1) * size_of::<Block>()
        );
    }
}

#[test]
fn changed_close_outranks_unreadable_and_all_later_closes_still_run() {
    let budget = Budget::default();
    let mut trace = Trace::default();
    for index in 0..3 {
        trace
            .field::<1>(
                &budget,
                SLOTS + index,
                Field::ProbeDistance,
                &mut |_, out| {
                    out[0] = 1;
                    true
                },
            )
            .unwrap();
    }
    let mut calls = Vec::new();
    assert_eq!(
        trace.close(&mut |a, out| {
            calls.push(a);
            out[0] = 2;
            a == SLOTS + 1
        }),
        Err(Refusal::Changed(Field::ProbeDistance))
    );
    assert_eq!(calls, vec![SLOTS + 2, SLOTS + 1, SLOTS]);
    assert_eq!(budget.verification_bytes(), 6);
}
