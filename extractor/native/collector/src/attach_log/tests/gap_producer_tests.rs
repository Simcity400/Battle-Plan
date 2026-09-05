use super::*;

fn d44_fixture() -> (usize, DivisionBodyBridgeEntry, Vec<u8>) {
    let base = HEAP_LO;
    let root = DivisionBodyBridgeEntry {
        div_id: 18406,
        body_ptr: HEAP_LO + 0x10000,
        owner_tag_db_id: 3,
    };
    let mut bytes = vec![0u8; DIVISION_OFF_FUEL + 4];
    bytes[..8].copy_from_slice(&((base + DIVISION_VTABLE_RVA) as u64).to_le_bytes());
    for (offset, value) in [
        (DIVISION_OFF_ID_TYPE, DIVISION_TYPE),
        (DIVISION_OFF_ID_ID, 18406),
        (DIVISION_OFF_LOCATION, 10),
        (DIVISION_OFF_COMBAT_REF_COUNT, 1),
        (DIVISION_OFF_OWNER_TAG_DB_ID, 3),
        (DIVISION_OFF_MOVE_PROGRESS, 123),
        (DIVISION_OFF_STRENGTH, 100),
        (DIVISION_OFF_ORGANISATION, 200),
        (DIVISION_OFF_MAX_SUPPLY, 800),
        (DIVISION_OFF_CURRENT_SUPPLY, 500),
        (DIVISION_OFF_SUPPLY_GAIN, 20),
        (DIVISION_OFF_FUEL, 900),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    (base, root, bytes)
}

fn d44_read_pair(
    base: usize,
    root: &DivisionBodyBridgeEntry,
    first: &[u8],
    second: &[u8],
) -> Result<CompactSupplyLogisticsAttritionRow, u32> {
    let mut calls = 0;
    let result =
        compact_supply_logistics_attrition_division_row_with_reader(base, root, |address, out| {
            assert_eq!(address, root.body_ptr);
            out.copy_from_slice(if calls == 0 { first } else { second });
            calls += 1;
            true
        });
    assert_eq!(calls, 2);
    result
}

#[test]
fn d44_every_consumed_byte_must_agree_but_unrelated_bytes_may_change() {
    let _registry = BoundTripRegistryFixture::acquire();
    supply_province::forget_engine_count_for_tests();
    assert!(supply_province::observe_engine_province_count(0xF00D, 13_414));
    let (base, root, first) = d44_fixture();
    let expected = d44_read_pair(base, &root, &first, &first).unwrap();
    assert_eq!(
        (
            expected.entity_id,
            expected.join_id,
            expected.owner_tag_db_id
        ),
        (18406, 10, 3)
    );
    assert_eq!(
        (
            expected.value_a_raw,
            expected.value_b_raw,
            expected.value_c_raw,
            expected.value_d_raw,
            expected.aux_raw
        ),
        (500, 800, 20, 300, 900)
    );
    // Independent layout fixture: includes every identity guard and consumed operand.
    let ranges = [
        0..8,
        0x08..0x0c,
        0x0c..0x10,
        0x18..0x1c,
        0x1a4..0x1a8,
        0x1c8..0x1cc,
        0x230..0x234,
        0x410..0x414,
        0x418..0x41c,
        0x5f0..0x5f4,
        0x5f8..0x5fc,
        0x600..0x604,
        0x610..0x614,
    ];
    for offset in ranges.iter().cloned().flatten() {
        let mut second = first.clone();
        second[offset] ^= 1;
        assert_eq!(
            d44_read_pair(base, &root, &first, &second),
            Err(D44_DIVISION_FAILURE_UNSTABLE),
            "offset {offset:#x}"
        );
    }
    let mut second = first.clone();
    for (offset, value) in second.iter_mut().enumerate() {
        if !ranges.iter().any(|range| range.contains(&offset)) {
            *value ^= 0xff;
        }
    }
    assert_ne!(first, second);
    assert_eq!(
        d44_read_pair(base, &root, &first, &second).unwrap(),
        expected
    );
    let mut other = first.clone();
    other[DIVISION_OFF_CURRENT_SUPPLY..DIVISION_OFF_CURRENT_SUPPLY + 4]
        .copy_from_slice(&300u32.to_le_bytes());
    let changed = d44_read_pair(base, &root, &other, &other).unwrap();
    assert_eq!((changed.value_a_raw, changed.value_d_raw), (300, 500));
}

#[test]
fn d44_unreadable_and_wrong_identity_remain_refused() {
    let _registry = BoundTripRegistryFixture::acquire();
    supply_province::forget_engine_count_for_tests();
    assert!(supply_province::observe_engine_province_count(0xF00D, 13_414));
    let (base, root, first) = d44_fixture();
    for fail_on in 0..2 {
        let mut calls = 0;
        let result =
            compact_supply_logistics_attrition_division_row_with_reader(base, &root, |_, out| {
                let fail = calls == fail_on;
                calls += 1;
                if !fail {
                    out.copy_from_slice(&first);
                }
                !fail
            });
        assert_eq!(
            result,
            Err(if fail_on == 0 {
                D44_DIVISION_FAILURE_FIRST_READ
            } else {
                D44_DIVISION_FAILURE_SECOND_READ
            })
        );
    }
    for (offset, reason) in [
        (0, D44_DIVISION_FAILURE_VTABLE),
        (DIVISION_OFF_ID_TYPE, D44_DIVISION_FAILURE_TYPE),
        (DIVISION_OFF_ID_ID, D44_DIVISION_FAILURE_ID),
    ] {
        let mut wrong = first.clone();
        wrong[offset..offset + 4].fill(0);
        assert_eq!(d44_read_pair(base, &root, &wrong, &wrong), Err(reason));
    }
}

fn d96_fixture() -> (Domain51CountryRoot, std::collections::BTreeMap<usize, u8>) {
    use compact_contract::tactic_weight::*;
    let country = Domain51CountryRoot {
        tag_db_id: 3,
        object_ptr: HEAP_LO + 0x10000,
    };
    let data = HEAP_LO + 0x20000;
    let mut memory = std::collections::BTreeMap::new();
    let mut put = |address, bytes: &[u8]| {
        for (i, b) in bytes.iter().enumerate() {
            memory.insert(address + i, *b);
        }
    };
    let container = country.object_ptr + COUNTRY_MODIFIER_ACCUMULATOR_OFFSET;
    put(
        container + MODIFIER_ACCUMULATOR_ARRAY_OFFSET,
        &(data as u64).to_le_bytes(),
    );
    put(
        container + MODIFIER_ACCUMULATOR_CAPACITY_OFFSET,
        &2u32.to_le_bytes(),
    );
    put(
        container + MODIFIER_ACCUMULATOR_COUNT_OFFSET,
        &2u32.to_le_bytes(),
    );
    for (index, key, value) in [(0, 5u32, -250i64), (1, 7u32, 0i64)] {
        let entry = data + index * MODIFIER_ACCUMULATOR_ENTRY_STRIDE;
        put(
            entry + MODIFIER_ACCUMULATOR_ENTRY_KEY_OFFSET,
            &key.to_le_bytes(),
        );
        put(
            entry + MODIFIER_ACCUMULATOR_ENTRY_VALUE_OFFSET,
            &value.to_le_bytes(),
        );
    }
    (country, memory)
}

fn d96_bytes(
    memory: &std::collections::BTreeMap<usize, u8>,
    address: usize,
    len: usize,
) -> Option<[u8; 8]> {
    let mut out = [0; 8];
    for i in 0..len {
        out[i] = *memory.get(&(address + i))?;
    }
    Some(out)
}

#[test]
fn d96_every_scalar_failure_keeps_stage_operand_entry_and_country() {
    let (country, memory) = d96_fixture();
    let expected = compact_country_modifier_accumulator_with_reader(country, |address, len| {
        d96_bytes(&memory, address, len)
    })
    .unwrap();
    assert_eq!(
        expected
            .iter()
            .map(|e| (e.key, e.value_raw))
            .collect::<Vec<_>>(),
        [(5, -250), (7, 0)]
    );
    let fields = [
        ("d96_accumulator_opening_data", None),
        ("d96_accumulator_opening_capacity", None),
        ("d96_accumulator_opening_count", None),
        ("d96_accumulator_entry_key", Some(0)),
        ("d96_accumulator_entry_value", Some(0)),
        ("d96_accumulator_entry_key", Some(1)),
        ("d96_accumulator_entry_value", Some(1)),
        ("d96_accumulator_closing_data", None),
        ("d96_accumulator_closing_capacity", None),
        ("d96_accumulator_closing_count", None),
    ];
    for (slot, (field, entry_index)) in fields.into_iter().enumerate() {
        for mode in 0..3 {
            let target = slot * 2 + usize::from(mode != 0);
            let mut calls = 0;
            let error =
                compact_country_modifier_accumulator_with_reader(country, |address, len| {
                    let at = calls;
                    calls += 1;
                    let mut bytes = d96_bytes(&memory, address, len)?;
                    if at == target {
                        if mode < 2 {
                            return None;
                        }
                        bytes[0] ^= 1;
                    }
                    Some(bytes)
                })
                .unwrap_err();
            let failure = [
                ModifierAccumulatorFailure::FirstReadUnavailable,
                ModifierAccumulatorFailure::SecondReadUnavailable,
                ModifierAccumulatorFailure::Unstable,
            ][mode];
            assert_eq!(
                error,
                ModifierAccumulatorError {
                    failure,
                    field,
                    entry_index
                }
            );
            for wrapped in [
                PreferredTacticWeightError::Country {
                    tag_db_id: 3,
                    source: PreferredTacticWeightRowError::Accumulator(error),
                },
                PreferredTacticWeightError::ClosingCountry {
                    tag_db_id: 3,
                    source: PreferredTacticWeightRowError::Accumulator(error),
                },
            ] {
                let units = preferred_tactic_weight_diagnostic_units(wrapped);
                assert!(units
                    .iter()
                    .any(|u| u.name == failure.diagnostic_name() && u.value == 1));
                assert!(units.iter().any(|u| u.name == field && u.value == 1));
                assert!(units
                    .iter()
                    .any(|u| u.name == "d96_failure_tag_db_id" && u.value == 3));
                assert_eq!(
                    units
                        .iter()
                        .find(|u| u.name == "d96_accumulator_entry_index")
                        .map(|u| u.value),
                    entry_index.map(u64::from)
                );
            }
        }
    }
}

#[test]
fn d96_structural_and_closing_refusals_are_not_missing_entries_or_zero_values() {
    use compact_contract::tactic_weight::*;
    use ModifierAccumulatorFailure::*;
    let (country, original) = d96_fixture();
    let container = country.object_ptr + COUNTRY_MODIFIER_ACCUMULATOR_OFFSET;
    let data = HEAP_LO + 0x20000;
    for (address, bytes, failure) in [
        (
            container + MODIFIER_ACCUMULATOR_COUNT_OFFSET,
            3u32.to_le_bytes().to_vec(),
            CountExceedsCapacity,
        ),
        (
            container + MODIFIER_ACCUMULATOR_CAPACITY_OFFSET,
            (MAX_TACTIC_MODIFIER_ACCUMULATOR_ENTRIES + 1)
                .to_le_bytes()
                .to_vec(),
            CapacityExceedsBound,
        ),
        (
            container + MODIFIER_ACCUMULATOR_ARRAY_OFFSET,
            1u64.to_le_bytes().to_vec(),
            PointerOutsideHeap,
        ),
        (
            data + MODIFIER_ACCUMULATOR_ENTRY_KEY_OFFSET,
            0u32.to_le_bytes().to_vec(),
            ZeroKey,
        ),
        (
            data + MODIFIER_ACCUMULATOR_ENTRY_STRIDE + MODIFIER_ACCUMULATOR_ENTRY_KEY_OFFSET,
            5u32.to_le_bytes().to_vec(),
            KeysNotIncreasing,
        ),
    ] {
        let mut memory = original.clone();
        for (i, b) in bytes.iter().enumerate() {
            memory.insert(address + i, *b);
        }
        let error = compact_country_modifier_accumulator_with_reader(country, |a, n| {
            d96_bytes(&memory, a, n)
        })
        .unwrap_err();
        assert_eq!(error.failure, failure);
    }
    for (slot, replacement, field) in [
        (
            7,
            ((data + 0x100) as u64).to_le_bytes(),
            "d96_accumulator_closing_data",
        ),
        (8, 3u64.to_le_bytes(), "d96_accumulator_closing_capacity"),
        (9, 1u64.to_le_bytes(), "d96_accumulator_closing_count"),
    ] {
        let mut calls = 0;
        let error = compact_country_modifier_accumulator_with_reader(country, |a, n| {
            let at = calls;
            calls += 1;
            if at / 2 == slot {
                Some(replacement)
            } else {
                d96_bytes(&original, a, n)
            }
        })
        .unwrap_err();
        assert_eq!(
            error,
            ModifierAccumulatorError {
                failure: HeaderChanged,
                field,
                entry_index: None
            }
        );
    }
    let overflow = Domain51CountryRoot {
        object_ptr: usize::MAX,
        ..country
    };
    assert_eq!(
        compact_country_modifier_accumulator_with_reader(overflow, |_, _| panic!(
            "overflow must not read"
        ))
        .unwrap_err()
        .failure,
        AddressOverflow
    );
    let payload = compact_country_preferred_tactic_weight_unavailable_payload(17);
    let decoded = decode_payload(&payload).unwrap();
    assert_eq!(decoded.status_flags, COMPACT_STATUS_SOURCE_UNAVAILABLE);
    assert!(decoded.rows.is_empty());
}
