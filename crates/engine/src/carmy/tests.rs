#[test]
fn icon_inputs_keep_raw_byte_and_full_alternate_template_handle() {
    use super::*;
    let mut bytes = fixture();
    let source = crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 3,
        template: handle(0, 52),
    };
    bytes[A1 + SEPARATE_COUNTER] = 255;
    p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, T1 as u64);
    p64(&mut bytes, T1 + 8, handle(0xffff_ffff, 0));
    let run = |bytes: &[u8]| {
        read_icon_inputs(
            &crate::memory::SliceMemory { base: 0, bytes },
            0,
            source,
            None,
        )
    };
    assert_eq!(
        run(&bytes),
        Ok(IconInputsRow {
            division: source.division,
            separate_counter: 255,
            alternate_template: handle(0xffff_ffff, 0),
        })
    );
    p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, 0);
    assert_eq!(run(&bytes).unwrap().alternate_template, 0);
}

#[test]
fn icon_inputs_reject_stale_army_sources_and_bad_nonnull_templates() {
    use super::*;
    let source = crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 3,
        template: handle(0, 52),
    };
    for (at, value, reason) in [
        (A1, 0, "army icon source identity changed"),
        (A1 + SECONDARY_BASE, 0, "army icon source identity changed"),
        (A1 + cunit::HANDLE, 0, "army icon source identity changed"),
        (
            A1 + cunit::ROSTER_OWNER,
            2,
            "army icon source identity changed",
        ),
        (T1, 0, "division template vtable mismatch"),
        (T1 + 8, 0, "non-null division template has null handle"),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, T1 as u64);
        p64(&mut bytes, at, value);
        assert_eq!(
            read_icon_inputs(
                &crate::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                source,
                None
            ),
            Err(reason)
        );
    }
}

#[test]
fn icon_inputs_unreadable_byte_and_bad_pointer_never_become_zero() {
    use super::*;
    let mut bytes = fixture();
    let source = crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 3,
        template: handle(0, 52),
    };
    assert_eq!(
        read_icon_inputs(
            &crate::memory::SliceMemory {
                base: 0,
                bytes: &bytes[..A1 + SEPARATE_COUNTER]
            },
            0,
            source,
            None
        ),
        Err("army separate-counter byte unreadable")
    );
    p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, u64::MAX);
    assert_eq!(
        read_icon_inputs(
            &crate::memory::SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            source,
            None
        ),
        Err("division template vtable mismatch")
    );
}

#[test]
fn seeded_icons_reuse_identity_and_exact_standard_template_alias() {
    use super::*;
    let mut bytes = fixture();
    let source = crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 25,
        template: handle(17, 52),
    };
    p32(&mut bytes, A1 + cunit::ROSTER_OWNER, source.country);
    p64(&mut bytes, A1 + STANDARD_TEMPLATE, T1 as u64);
    p64(&mut bytes, T1 + 8, source.template);
    bytes[A1 + SEPARATE_COUNTER] = 255;
    let mut seeds = [DivisionSeed {
        pointer: A1,
        division: source.division,
        ..Default::default()
    }];
    capture_division_seeds(
        &crate::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        &mut seeds,
    );
    let seed = seeds[0];
    const ALTERNATE: usize = T1 + 0x100;
    p64(
        &mut bytes,
        ALTERNATE,
        creferenced_division_template::VTABLE_RVA as u64,
    );
    p64(&mut bytes, ALTERNATE + 8, handle(23, 52));
    for (pointer, calls_saved) in [(0, 8), (T1, 10), (ALTERNATE, 8)] {
        p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, pointer as u64);
        let memory = crate::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let scalar = crate::read_census::ReadCensus::new(&memory);
        let expected = read_icon_inputs(&scalar, 0, source, None).unwrap();
        let owned = crate::read_census::ReadCensus::new(&memory);
        assert_eq!(
            read_icon_inputs(&owned, 0, source, Some(&seed)),
            Ok(expected)
        );
        assert_eq!(scalar.calls() - owned.calls(), calls_saved);
        assert_eq!(expected.separate_counter, 255);
    }
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let owned = crate::read_census::ReadCensus::new(&memory);
    assert_eq!(
        read_icon_inputs(
            &owned,
            0,
            crate::cequipment_variant_pool::DivisionSource {
                country: 3,
                ..source
            },
            Some(&seed)
        ),
        Err("army icon source identity changed")
    );
    assert_eq!(owned.calls(), 0);
}

#[test]
fn seeded_icons_keep_failed_fields_and_source_keys_without_retry() {
    use super::*;
    let bytes = fixture();
    let source = crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 3,
        template: handle(0, 52),
    };
    let mut seeds = [DivisionSeed {
        pointer: A1,
        division: source.division,
        ..Default::default()
    }];
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    capture_division_seeds(&memory, &mut seeds);
    for fault in 0..6 {
        let mut seed = seeds[0];
        match fault {
            0 => seed.primary_vtable = None,
            1 => seed.secondary_vtable = None,
            2 => seed.handle = None,
            3 => seed.roster_owner = None,
            4 => seed.pointer += 1,
            _ => seed.division ^= 1 << 32,
        }
        let owned = crate::read_census::ReadCensus::new(&memory);
        assert_eq!(
            read_icon_inputs(&owned, 0, source, Some(&seed)),
            Err("army icon source identity changed")
        );
        assert_eq!(owned.calls(), 0);
    }
}

#[test]
fn owned_normal_speed_preserves_native_products_and_safe_zero_guard() {
    use super::*;
    let mut common = NormalSpeedOperands {
        modifier: NormalModifierOperands {
            additive: 80_000,
            weather: [(50_000, -10_000), (0, 0), (0, 0), (0, 0)],
            supply: (-20_000, 50_000),
            air: MovementAirOperands {
                fraction: 50_000,
                impact: 100_000,
                scale: -20_000,
                defense: 1_000,
            },
        },
        base_speed: 1_000_000,
        organisation: Some((50_000, -40_000)),
        fuel: Some((50_000, 20_000)),
        outer_scale: 50_000,
        bounded: false,
    };
    common.bounded = common.bounded();
    let edge = NormalEdgeOperands {
        additive: 10_000,
        river: Some((50_000, -10_000, -20_000)),
    };
    let inputs = MovementSpeedInputs::Normal { common, edge };
    // Native modifiers: common 56000, edge -25000; speed 310000,
    // organisation 248000, fuel 148800, outer scale 74400.
    assert_eq!(inputs.evaluate(), 74_400);
    assert!(common.bounded && edge.bounded());
    assert!(!inputs.is_zero());
    for scale in [0, i64::MIN] {
        common.outer_scale = scale;
        common.bounded = common.bounded();
        assert!(!common.bounded);
        let inputs = MovementSpeedInputs::Normal { common, edge };
        assert_eq!(inputs.evaluate(), 0);
        assert_eq!(inputs.is_zero(), inputs.evaluate() == 0);
    }
}

#[test]
fn owned_normal_nonzero_proof_covers_extreme_allowed_operands() {
    use super::*;
    for sign in [-1, 1] {
        for base_speed in [-1_000_000, 0, 1_000_000] {
            for scale in [1, 100_000] {
                for organisation in [
                    None,
                    Some((0, -100_000)),
                    Some((100_000, 0)),
                    Some((50_000, -40_000)),
                ] {
                    for fuel in [None, Some((100_000, 100_000)), Some((50_000, 20_000))] {
                        let mut common = NormalSpeedOperands {
                            modifier: NormalModifierOperands {
                                additive: sign * 100_000_000,
                                weather: [(sign * 100_000, 1_000_000); 4],
                                supply: (sign * 1_000_000, 1_000_000),
                                air: MovementAirOperands {
                                    fraction: sign * 100_000,
                                    impact: 1_000_000,
                                    scale: 1_000_000,
                                    defense: sign * 1_000_000,
                                },
                            },
                            base_speed,
                            organisation,
                            fuel,
                            outer_scale: scale,
                            bounded: false,
                        };
                        common.bounded = common.bounded();
                        let edge = NormalEdgeOperands {
                            additive: sign * 100_000_000,
                            river: Some((sign * 1_000_000, -1_000_000, sign * 1_000_000)),
                        };
                        assert!(common.bounded && edge.bounded());
                        let inputs = MovementSpeedInputs::Normal { common, edge };
                        assert!(!inputs.is_zero());
                        assert!(inputs.evaluate() > 0);
                        common.modifier.additive = i64::MAX;
                        common.bounded = common.bounded();
                        let inputs = MovementSpeedInputs::Normal { common, edge };
                        assert!(!common.bounded);
                        assert_eq!(inputs.is_zero(), inputs.evaluate() == 0);
                    }
                }
            }
        }
    }
}

use super::{Row, walk, walk_with};
use crate::{
    carmy, carmy_group, ccountry, corders_group, cprovince, creferenced_division_template, cunit,
    memory::SliceMemory, walk::Outcome,
};
const GS: usize = 0x1000;
const COUNTRIES: usize = 0x3000;
const LINKS: usize = 0x3100;
const PROVINCES: usize = 0x3200;
const FIRST: usize = 0x3600;
const SECOND: usize = 0x3640;
const C1: usize = 0x5000;
const C2: usize = 0x7000;
const A1: usize = 0xa000;
const A2: usize = 0xb000;
const A3: usize = 0xc000;
const P1: usize = 0xd000;
const T1: usize = 0xe000;
const G1: usize = 0xf000;

fn p32(b: &mut [u8], at: usize, value: u32) {
    b[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(b: &mut [u8], at: usize, value: u64) {
    b[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn handle(id: u32, kind: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(kind)
}
fn array(b: &mut [u8], at: usize, data: usize, count: u32) {
    p64(b, at, data as u64);
    p32(b, at + 8, count);
    p32(b, at + 12, count);
}
fn fixture() -> Vec<u8> {
    let mut b = vec![0; 0x10000];
    array(&mut b, GS + 0x310, COUNTRIES, 4);
    array(&mut b, GS + 0x340, LINKS, 4);
    array(&mut b, GS + 0x2b0, PROVINCES, 2);
    // Country slot zero is reserved. Slot3 aliases canonical slot1.
    for (slot, ptr) in [u64::MAX, C1 as u64, C2 as u64, C1 as u64]
        .into_iter()
        .enumerate()
    {
        p64(&mut b, COUNTRIES + slot * 8, ptr);
    }
    for (slot, id) in [0, 1, 2, 1].into_iter().enumerate() {
        p32(&mut b, LINKS + slot * 4, id);
    }
    for (country, id, data, count) in [(C1, 3, FIRST, 3), (C2, 2, SECOND, 2)] {
        p64(&mut b, country, ccountry::VTABLE_RVA as u64);
        p32(&mut b, country + 8, id);
        array(&mut b, country + 0x290, data, count);
    }
    // Division rosters include slot zero, and may contain null holes.
    for (slot, ptr) in [A1 as u64, 0, A2 as u64].into_iter().enumerate() {
        p64(&mut b, FIRST + slot * 8, ptr);
    }
    p64(&mut b, SECOND, 0);
    p64(&mut b, SECOND + 8, A3 as u64);
    p64(&mut b, PROVINCES, u64::MAX);
    p64(&mut b, PROVINCES + 8, P1 as u64);
    p64(&mut b, P1, cprovince::VTABLE_RVA as u64);
    p32(&mut b, P1 + 0xa4, 1);
    p64(&mut b, T1, creferenced_division_template::VTABLE_RVA as u64);
    p64(&mut b, T1 + 8, handle(0, 52)); // A one-zero-word handle is not null.
    p64(&mut b, G1, corders_group::VTABLE_RVA as u64);
    p64(&mut b, G1 + 8, handle(57, corders_group::REFERENCE_TYPE));
    // A3 keeps the native null parent, which is not a failed read.
    for (at, id, owner, group) in [
        (A1, handle(9, 9000), 3, G1),
        (A2, handle(9, 51), 3, G1),
        (A3, handle(1, 0), 2, 0),
    ] {
        p64(&mut b, at, carmy::VTABLE_RVA as u64);
        p64(&mut b, at + 0x10, carmy::SECONDARY_VTABLE_RVA as u64);
        p64(&mut b, at + 0x18, id);
        p64(&mut b, at + carmy::ORDERS_GROUP, group as u64);
        p32(&mut b, at + 0x1d8, owner);
        p32(&mut b, at + 0x1dc, 0);
        p32(&mut b, at + 0x1e0, 2);
        p64(&mut b, at + 0x1f0, P1 as u64);
        p64(&mut b, at + 0x3b8, T1 as u64);
        p64(&mut b, at + 0x420, i64::MIN as u64);
        p64(&mut b, at + 0x428, i64::MAX as u64);
    }
    b
}
fn run(b: &[u8], out: &mut [Row]) -> Outcome {
    walk(&SliceMemory { base: 0, bytes: b }, 0, GS, out)
}
fn expected(id: u64, owner: u32) -> Row {
    Row {
        id,
        roster_owner: owner,
        logical_owner: 2,
        expeditionary_owner: 0,
        province: 1,
        template: handle(0, 52),
        hit_points: i64::MIN,
        organisation: i64::MAX,
        orders_group: handle(57, corders_group::REFERENCE_TYPE),
        reveal_skip: 0,
    }
}

#[test]
fn raw_fields_and_native_roster_order_survive_country_aliases_and_holes() {
    let b = fixture();
    let mut out = [Row::default(); 5];
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 3 });
    assert_eq!(
        &out[..3],
        &[
            expected(handle(9, 9000), 3),
            expected(handle(9, 51), 3),
            Row {
                orders_group: 0,
                ..expected(handle(1, 0), 2)
            },
        ]
    );
    // The two type-distinct id9 rows remain separate and unsorted; alias slot3 adds no copies.
}

#[test]
fn orders_group_parent_is_authenticated_and_a_null_parent_is_not_a_failure() {
    let mut b = fixture();
    // An army-group parent uses the derived vtable and the same type word.
    p64(&mut b, G1, carmy_group::VTABLE_RVA as u64);
    let mut out = [Row::default(); 5];
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 3 });
    assert_eq!(
        out[0].orders_group,
        handle(57, corders_group::REFERENCE_TYPE)
    );
    // A foreign vtable at the back-pointer is a failure, never a zero row.
    p64(&mut b, G1, cprovince::VTABLE_RVA as u64);
    assert_eq!(
        run(&b, &mut out),
        Outcome::Unavailable {
            reason: "division orders group vtable mismatch"
        }
    );
    // So is a group object whose reference type is not type 53.
    p64(&mut b, G1, corders_group::VTABLE_RVA as u64);
    p64(&mut b, G1 + 8, handle(57, 61));
    assert_eq!(
        run(&b, &mut out),
        Outcome::Unavailable {
            reason: "division orders group reference type mismatch"
        }
    );
}

#[test]
fn shared_visitors_preserve_source_identity_and_never_visit_omitted_or_failed_rows() {
    let mut bytes = fixture();
    let mut output = [Row::default(); 3];
    // Independent walks must report the current row, not retained source data.
    for hit_points in [i64::MIN, 42] {
        p64(&mut bytes, A1 + 0x420, hit_points as u64);
        let mut visited = [(0, Row::default()); 3];
        let mut count = 0;
        assert_eq!(
            walk_with(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut output,
                |pointer, row| {
                    visited[count] = (pointer, *row);
                    count += 1;
                },
            ),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(count, 3);
        assert_eq!(visited, [(A1, output[0]), (A2, output[1]), (A3, output[2])]);
        assert_eq!(visited[0].1.hit_points, hit_points);
        assert_eq!(
            (
                visited[0].1.id,
                visited[0].1.template,
                visited[0].1.roster_owner
            ),
            (handle(9, 9000), handle(0, 52), 3)
        );
    }
    p64(&mut bytes, A2 + 0x3b8, u64::MAX);
    let mut count = 0;
    assert_eq!(
        walk_with(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut output[..1],
            |pointer, _| {
                assert_eq!(pointer, A1);
                count += 1;
            },
        ),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    assert_eq!(count, 1);
    count = 0;
    assert!(matches!(
        walk_with(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut output,
            |pointer, _| {
                assert_eq!(pointer, A1);
                count += 1;
            },
        ),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(count, 1); // Earlier visits do not turn a failed census into complete data.
    p64(&mut bytes, A1 + 0x3b8, u64::MAX);
    assert!(matches!(
        walk_with(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut output,
            |_, _| panic!("unreadable row reached visitor"),
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn null_location_and_template_are_explicit_and_nonzero_half_handles_are_valid() {
    let mut b = fixture();
    let mut out = [Row::default(); 3];
    p64(&mut b, A1 + 0x1f0, 0);
    p64(&mut b, A1 + 0x3b8, 0);
    p64(&mut b, A1 + 0x18, handle(0, 9000));
    p32(&mut b, A1 + 0x1dc, 3);
    p64(&mut b, A1 + 0x420, i64::MAX as u64);
    p64(&mut b, A1 + 0x428, i64::MIN as u64);
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 3 });
    assert_eq!(
        out[0],
        Row {
            id: handle(0, 9000),
            roster_owner: 3,
            logical_owner: 2,
            expeditionary_owner: 3,
            province: 0,
            template: 0,
            hit_points: i64::MAX,
            organisation: i64::MIN,
            orders_group: handle(57, corders_group::REFERENCE_TYPE),
            reveal_skip: 0,
        }
    );
}

#[test]
fn raw_logical_and_expeditionary_country_bits_are_not_replaced_by_roster_owner() {
    let mut bytes = fixture();
    p32(&mut bytes, A1 + 0x1dc, 0xffff_ffff);
    p32(&mut bytes, A1 + 0x1e0, 0);
    let mut output = [Row::default(); 3];
    assert_eq!(run(&bytes, &mut output), Outcome::Rows { len: 3 });
    assert_eq!(output[0].roster_owner, 3);
    assert_eq!(output[0].logical_owner, 0);
    assert_eq!(output[0].expeditionary_owner, 0xffff_ffff);
}

#[test]
fn capacity_counts_nonnull_rows_across_later_countries_without_reading_their_fields() {
    let mut b = fixture();
    let mut out = [Row::default(); 1];
    p64(&mut b, A2, 0); // Beyond capacity: its fields must not be dereferenced.
    assert_eq!(
        run(&b, &mut out),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    assert_eq!(out[0], expected(handle(9, 9000), 3));
    assert_eq!(
        run(&b, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    // A failed later roster pointer read cannot become an apparently complete row count.
    p64(&mut b, C2 + 0x290, 0xffff_0000);
    assert!(matches!(run(&b, &mut out), Outcome::Unavailable { .. }));
}

#[test]
fn invalid_identity_owner_and_array_structure_fail_the_whole_table() {
    let cases: [(usize, u64, bool); 15] = [
        (A1, 0, true),                            // Division vtable.
        (A1 + 0x18, 0, true),                     // Both handle words zero.
        (A1 + 0x1d8, 2, false),                   // Owner resolves to another roster country.
        (C1, 0, true),                            // Country vtable.
        (C1 + 8, 0, false),                       // Country object id.
        (LINKS + 3 * 4, 2, false),                // Alias slot and object no longer agree.
        (COUNTRIES + 8, C2 as u64, true),         // Canonical pointer differs from alias.
        (C1 + 0x29c, 4, false),                   // Roster count exceeds capacity.
        (A1 + 0x1f0, u64::MAX, true),             // Non-null unreadable province.
        (P1, 0, true),                            // Province vtable.
        (P1 + 0xa4, 0, false),                    // Non-null province cannot have id zero.
        (PROVINCES + 8, P1 as u64 + 0x100, true), // Province identity must match array pointer.
        (A1 + 0x3b8, u64::MAX, true),             // Non-null unreadable template.
        (T1, 0, true),                            // Template vtable.
        (T1 + 8, 0, true),                        // Non-null template has null handle.
    ];
    for (at, value, wide) in cases {
        let mut b = fixture();
        let mut out = [Row::default(); 3];
        if wide {
            p64(&mut b, at, value);
        } else {
            p32(&mut b, at, value as u32);
        }
        assert!(
            matches!(run(&b, &mut out), Outcome::Unavailable { .. }),
            "address {at:x}"
        );
    }
}

#[test]
fn duplicate_full_handles_are_left_for_spool_validation() {
    let mut b = fixture();
    let mut out = [Row::default(); 3];
    p64(&mut b, A2 + 0x18, handle(9, 9000));
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 3 });
    assert_eq!(out[0].id, out[1].id);
}

#[test]
fn legal_empty_rosters_return_zero_rows() {
    let mut b = fixture();
    let mut out = [Row::default(); 1];
    array(&mut b, C1 + 0x290, 0, 0);
    array(&mut b, C2 + 0x290, 0, 0);
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 0 });
}

struct PresentationMemory {
    bytes: Vec<u8>,
    globals: std::collections::BTreeMap<usize, Vec<u8>>,
}

impl crate::Memory for PresentationMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((&start, bytes)) = self.globals.range(..=address).next_back()
            && let Some(source) = address
                .checked_sub(start)
                .and_then(|offset| bytes.get(offset..offset.checked_add(output.len())?))
        {
            output.copy_from_slice(source);
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

struct ObservedMovementMemory<'a> {
    source: &'a PresentationMemory,
    reads: std::cell::RefCell<Vec<usize>>,
    fault: Option<usize>,
}

impl crate::Memory for ObservedMovementMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.fault != Some(address) && self.source.copy(address, output)
    }
}

struct ScalarBatchMemory<'a> {
    source: &'a PresentationMemory,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    batches: std::cell::Cell<usize>,
    fault: std::cell::Cell<Option<usize>>,
}
impl<'a> ScalarBatchMemory<'a> {
    fn new(source: &'a PresentationMemory) -> Self {
        Self {
            source,
            reads: Default::default(),
            batches: std::cell::Cell::new(0),
            fault: std::cell::Cell::new(None),
        }
    }
}
impl crate::Memory for ScalarBatchMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault.get() == Some(address) {
            output.fill(255);
            return false;
        }
        self.source.copy(address, output)
    }
    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &crate::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.batches.set(self.batches.get() + 1);
        // Exercise the existing ordered scalar fallback while observing
        // the one batch requested by production. No alternate gather ABI.
        struct ScalarOnly<'a, 'b>(&'a ScalarBatchMemory<'b>);
        impl crate::Memory for ScalarOnly<'_, '_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                crate::Memory::copy(self.0, address, output)
            }
        }
        crate::Memory::copy_records(&ScalarOnly(self), sources, layout, destination)
    }
}

fn scalar_tail_test_read(
    memory: &impl crate::Memory,
    army: usize,
    kind: usize,
    packed: bool,
) -> Result<[i128; 4], &'static str> {
    if packed {
        match kind {
            0 => {
                let (a, b, c, d) = super::status_counter_scalars(memory, army)?;
                Ok([a as i128, b as i128, c as i128, d as i128])
            }
            _ => {
                let (a, b) = super::movement_scalar_tail(memory, army)?;
                Ok([a as i128, b as i128, 0, 0])
            }
        }
    } else {
        // Original field sequence, widths and conversions, independent of
        // the new RecordField declarations.
        match kind {
            0 => Ok([
                super::presentation_field::<i64>(memory, army, carmy::COUNTER_ATTRITION)? as i128,
                super::presentation_field::<i64>(memory, army, carmy::ENTRENCHMENT)? as i128,
                super::presentation_field::<i64>(memory, army, carmy::ENTRENCHMENT_CAP)? as i128,
                super::presentation_field::<i64>(memory, army, carmy::COUNTER_SUPPLY_STATUS)?
                    as i128,
            ]),
            _ => Ok([
                super::presentation_field::<u8>(memory, army, carmy::RETREAT)? as i128,
                super::presentation_field::<u8>(memory, army, carmy::STRATEGIC_REDEPLOYMENT)?
                    as i128,
                0,
                0,
            ]),
        }
    }
}

#[test]
fn scalar_tail_batches_match_native_field_order_and_every_fault_prefix() {
    let mut source = presentation_fixture();
    p64(
        &mut source.bytes,
        A1 + carmy::COUNTER_ATTRITION,
        i64::MIN as u64,
    );
    p64(
        &mut source.bytes,
        A1 + carmy::NAVAL_TRANSFER_MEMBERSHIP,
        u64::MAX,
    );
    p64(&mut source.bytes, A1 + crate::cunit::MAP_ENTITY, u64::MAX);
    source.bytes[A1 + carmy::RETREAT] = 0xfe;
    source.bytes[A1 + carmy::STRATEGIC_REDEPLOYMENT] = 0xff;
    for (kind, offsets) in [
        vec![
            carmy::COUNTER_ATTRITION,
            carmy::ENTRENCHMENT,
            carmy::ENTRENCHMENT_CAP,
            carmy::COUNTER_SUPPLY_STATUS,
        ],
        vec![carmy::RETREAT, carmy::STRATEGIC_REDEPLOYMENT],
    ]
    .into_iter()
    .enumerate()
    {
        for fault in std::iter::once(None).chain(offsets.iter().map(|&offset| Some(A1 + offset))) {
            let direct = ScalarBatchMemory::new(&source);
            let packed = ScalarBatchMemory::new(&source);
            direct.fault.set(fault);
            packed.fault.set(fault);
            let expected = scalar_tail_test_read(&direct, A1, kind, false);
            let actual = scalar_tail_test_read(&packed, A1, kind, true);
            assert_eq!(actual, expected);
            assert_eq!(&*packed.reads.borrow(), &*direct.reads.borrow());
            assert_eq!(packed.batches.get(), 1);
            assert_eq!(direct.batches.get(), 0);
            if let Some(fault) = fault {
                assert_eq!(actual, Err("division presentation field unreadable"));
                assert_eq!(packed.reads.borrow().last().unwrap().0, fault);
            } else {
                assert!(actual.is_ok());
                assert_eq!(packed.reads.borrow().len(), offsets.len());
            }
        }
    }
}

#[test]
fn scalar_tail_batches_preserve_address_overflow_prefix() {
    for (kind, first_offset, second_offset, first_len) in [
        (0, carmy::COUNTER_ATTRITION, carmy::ENTRENCHMENT, 8),
        (2, carmy::RETREAT, carmy::STRATEGIC_REDEPLOYMENT, 1),
    ] {
        let army = usize::MAX - second_offset + 1;
        let mut source = presentation_fixture();
        source
            .globals
            .insert(army + first_offset, vec![0; first_len]);
        let direct = ScalarBatchMemory::new(&source);
        let packed = ScalarBatchMemory::new(&source);
        assert_eq!(
            scalar_tail_test_read(&packed, army, kind, true),
            scalar_tail_test_read(&direct, army, kind, false)
        );
        assert_eq!(&*packed.reads.borrow(), &*direct.reads.borrow());
        assert_eq!(packed.reads.borrow()[0], (army + first_offset, first_len));
    }
}

#[test]
fn presentation_scalar_batches_preserve_success_trace_and_early_guards() {
    let source = normal_route_fixture();
    let direct = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let packed = ScalarBatchMemory::new(&source);
    let mut expected = [super::MovementRow::default(); 1];
    let mut actual = expected;
    let mut expected_paths = [0; 8];
    let mut actual_paths = expected_paths;
    let baseline = super::walk_movement(
        &direct,
        0,
        GS,
        &presentation_source(),
        Outcome::Rows { len: 1 },
        &mut expected,
        &mut expected_paths,
    );
    let result = super::walk_movement(
        &packed,
        0,
        GS,
        &presentation_source(),
        Outcome::Rows { len: 1 },
        &mut actual,
        &mut actual_paths,
    );
    assert_eq!(result, baseline);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(actual, expected);
    assert_eq!(actual_paths, expected_paths);
    assert_eq!(packed.batches.get(), 1);
    assert_eq!(
        packed
            .reads
            .borrow()
            .iter()
            .map(|&(address, _)| address)
            .collect::<Vec<_>>(),
        *direct.reads.borrow()
    );

    let direct = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let packed = ScalarBatchMemory::new(&source);
    let mut expected = [super::StatusRow::default(); 1];
    let mut actual = expected;
    assert_eq!(
        super::walk_status(
            &direct,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut expected
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        super::walk_status(
            &packed,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut actual
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(actual, expected);
    // Status gathers fixed seeds, the country's modifier snapshot and the
    // stat block's maxima and exfiltration byte once each, then the independent
    // supply-footer scalar spans in one additional gather.
    assert_eq!(packed.batches.get(), 4);
    for (offset, width) in [
        (crate::cunit_supply_inputs::ASKED, 8),
        (crate::cunit_supply_inputs::RECEIVED, 16),
        (crate::cunit_supply_inputs::FROM_STATE, 12),
        (crate::cunit_supply_inputs::DIVISION_DEMAND, 12),
        (crate::cunit_supply_inputs::AIRPLANE_DEMAND, 12),
        (crate::cunit_supply_inputs::SHIP_DEMAND, 16),
    ] {
        assert_eq!(
            packed
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, len)| address == A1 + offset && len == width)
                .count(),
            1,
            "each supply-footer span is acquired once"
        );
    }
    assert_eq!(
        packed
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| {
                address == 0x1c000 && len == crate::modifier_entry::STRIDE
            })
            .count(),
        1,
        "the modifier payload is acquired once"
    );
    assert_eq!(
        packed
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, _)| { address == A1 + carmy::COUNTER_ATTRITION })
            .count(),
        1,
        "Status consumes the shared counter scalar without rereading it"
    );
    assert_eq!(
        packed
            .reads
            .borrow()
            .iter()
            .map(|&(address, _)| address)
            .collect::<Vec<_>>(),
        *direct.reads.borrow()
    );

    // These test readers own their bounded source preparation. Empty
    // output skips the Status seed prefix and Movement's private tails.
    let packed = ScalarBatchMemory::new(&source);
    assert!(matches!(
        super::walk_status(
            &packed,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut []
        ),
        Outcome::Truncated { .. }
    ));
    assert!(matches!(
        super::walk_movement(
            &packed,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut [],
            &mut []
        )
        .outcome,
        Outcome::Truncated { .. }
    ));
    assert_eq!(packed.batches.get(), 0);
    assert!(
        packed.reads.borrow().iter().all(|&(address, _)| {
            address != 0x11000
                && address != C1 + ccountry::MODIFIER_ACCUMULATOR
                && address != A1 + carmy::COUNTER_ATTRITION
                && address != A1 + carmy::DEPLOYMENT_TYPE
        }),
        "omitted rows never acquire private Status dependencies"
    );

    for (offset, batches) in [(carmy::MOVEMENT_TARGET, 0), (carmy::MOVEMENT_SOURCE, 0)] {
        let mut source = normal_route_fixture();
        p64(&mut source.bytes, A1 + offset, 0xdead_beef);
        let packed = ScalarBatchMemory::new(&source);
        let mut rows = [super::MovementRow::default(); 1];
        assert!(matches!(
            super::walk_movement(
                &packed,
                0,
                GS,
                &presentation_source(),
                Outcome::Rows { len: 1 },
                &mut rows,
                &mut [0; 8]
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(packed.batches.get(), batches);
        assert_eq!(rows, [super::MovementRow::default(); 1]);
    }
    let mut source = normal_route_fixture();
    p64(&mut source.bytes, A1 + carmy::ORDERS_GROUP, 0xdead_beef);
    let packed = ScalarBatchMemory::new(&source);
    let mut rows = [super::StatusRow::default(); 1];
    assert!(matches!(
        super::walk_status(
            &packed,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Unavailable { .. }
    ));
    // Fixed seeds, the stat block and the single country modifier capture
    // precede group admission; an invalid group does not retry any source.
    assert_eq!(packed.batches.get(), 3);
    assert_eq!(
        packed
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| {
                address == 0x1c000 && len == crate::modifier_entry::STRIDE
            })
            .count(),
        1
    );
    assert!(
        packed.reads.borrow().iter().all(|&(address, _)| {
            address != 0xdead_beef + corders_group::TRAINING
                && address != A1 + carmy::DEPLOYMENT_TYPE
                && address != carmy::EXERCISE_MAXIMUM_LEVEL_RVA
        }),
        "group authentication excludes private training and exercise dependencies"
    );
    assert_eq!(rows, [super::StatusRow::default(); 1]);
}

fn presentation_fixture() -> PresentationMemory {
    use crate::{
        cgame_state, csub_unit_definition as stats, cunit, province_definition as definition,
    };
    let mut bytes = fixture();
    bytes.resize(0x20000, 0);
    for army in [A1, A2, A3] {
        p64(
            &mut bytes,
            army + carmy::ACCLIMATIZATION,
            crate::acclimatization_data::VTABLE_RVA as u64,
        );
    }
    p64(&mut bytes, A1 + cunit::STAT_BLOCK, 0x11000);
    p64(&mut bytes, 0x11000, stats::VTABLE_RVA as u64);
    p64(&mut bytes, 0x11000 + stats::MAXIMUM_STRENGTH, 200_000);
    p64(
        &mut bytes,
        0x11000 + stats::MAXIMUM_ORGANISATION,
        i64::MAX as u64,
    );
    p64(&mut bytes, A1 + carmy::STRENGTH, 150_000);
    p64(&mut bytes, A1 + carmy::ENTRENCHMENT, i64::MIN as u64);
    p64(&mut bytes, A1 + carmy::ENTRENCHMENT_CAP, i64::MAX as u64);
    p64(
        &mut bytes,
        A1 + carmy::COUNTER_SUPPLY_STATUS,
        -25_i64 as u64,
    );
    p64(&mut bytes, A1 + carmy::COUNTER_ATTRITION, 501);
    p64(&mut bytes, P1 + cprovince::DEFINITION, 0x12000);
    p32(&mut bytes, 0x12000 + definition::ID, 1);
    p64(&mut bytes, A1 + cunit::MAP_ENTITY, 0x14000);
    p64(&mut bytes, 0x14000, A1 as u64);
    array(&mut bytes, GS + cgame_state::HUMAN_STATUS, 0x15000, 4);
    array(&mut bytes, GS + cgame_state::AUTONOMOUS_STATUS, 0x15010, 4);
    let mut thresholds_header = vec![0_u8; 16];
    array(&mut thresholds_header, 0, 0x15100, 3);
    for (i, threshold) in [100_u64, 200, 300].into_iter().enumerate() {
        p64(&mut bytes, 0x15100 + i * 8, threshold);
    }
    PresentationMemory {
        bytes,
        globals: [
            (
                carmy::COUNTER_ATTRITION_THRESHOLD_RVA,
                500_i64.to_le_bytes().to_vec(),
            ),
            (
                carmy::EXERCISE_MAXIMUM_LEVEL_RVA,
                2_i32.to_le_bytes().to_vec(),
            ),
            (
                carmy::EXERCISE_MINIMUM_STRENGTH_RVA,
                50_000_i64.to_le_bytes().to_vec(),
            ),
            (carmy::EXPERIENCE_THRESHOLDS_RVA, thresholds_header),
        ]
        .into(),
    }
}

#[test]
fn transport_callback_keeps_raw_keys_per_edge_and_native_outer_scale() {
    let mut memory = presentation_fixture();
    memory
        .globals
        .insert(0x0331_efe0, 33_333_i64.to_le_bytes().to_vec());
    let mut keys = Vec::new();
    for (owner, region, flags, transfer) in
        [(2_u32, 0x18000_usize, 0_u8, 0_u64), (3, 0x18100, 1, 1)]
    {
        p32(&mut memory.bytes, A1 + crate::cunit::LOGICAL_OWNER, owner);
        p64(
            &mut memory.bytes,
            P1 + cprovince::STRATEGIC_REGION,
            region as u64,
        );
        memory.bytes[0x12000 + crate::province_definition::TERRAIN_FLAGS] = flags;
        p64(
            &mut memory.bytes,
            A1 + carmy::NAVAL_TRANSFER_MEMBERSHIP,
            transfer,
        );
        let speed = super::effective_movement_speed_with_transport(
            &memory,
            0,
            GS,
            A1,
            P1,
            0,
            |owner, region| {
                keys.push((owner, region));
                Ok(100_001)
            },
        );
        assert_eq!(speed, Ok(33_333));
    }
    assert_eq!(keys, [(2, 0x18000), (3, 0x18100)]);
    assert_eq!(
        super::effective_movement_speed_with_transport(&memory, 0, GS, A1, P1, 0, |_, _| Err(
            "transport source changed"
        )),
        Err("transport source changed")
    );
    p64(&mut memory.bytes, A1, 0);
    assert!(
        super::effective_movement_speed_with_transport(&memory, 0, GS, A1, P1, 0, |_, _| panic!(
            "bad army must not reuse transport"
        ))
        .is_err()
    );
}

fn normal_route_fixture() -> PresentationMemory {
    let mut memory = presentation_fixture();
    memory.bytes.resize(0x30000, 0);
    let b = &mut memory.bytes;
    p64(b, A1 + carmy::ORDERS_GROUP, 0);
    p32(b, A1 + crate::cunit::LOGICAL_OWNER, 3);
    p64(b, A1 + carmy::ORGANISATION, 200_000);
    p64(b, A1 + carmy::FUEL_CANDIDATE, 50_000);
    b[A1 + 0x2ae] = 1;
    b[A1 + carmy::RETREAT] = 1;
    p64(b, 0x11000 + 0x120, 100_000);
    p64(b, 0x11000 + 0x280, 200_000);
    p64(b, 0x11000 + 0x290, 400_003);
    array(b, 0x11000 + 0x3f8, 0x16000, 2);
    for (terrain, entry, token, index, speed) in [
        (0x17000, 0x16000, 123_u32, 1_u32, 12_345_i64),
        (0x17400, 0x16028, 124, 2, -10_001),
    ] {
        p64(b, terrain, 0x029a_a2b0);
        p32(b, terrain + 8, token);
        p32(b, terrain + 0x88, index);
        p64(b, terrain + 0x168, 0x0277_3a70);
        p64(b, terrain + 0x188, (-2_345_i64) as u64);
        p64(b, entry, 0x0277_3a70);
        p32(b, entry + 8, token);
        p64(b, entry + 0x20, speed as u64);
    }
    array(b, GS + crate::cgame_state::PROVINCES, PROVINCES, 4);
    for (id, province, definition, terrain) in [
        (1_u32, P1, 0x12000, 0x17000),
        (2, 0x18200, 0x18600, 0x17000),
        (3, 0x18400, 0x18700, 0x17400),
    ] {
        p64(b, PROVINCES + id as usize * 8, province as u64);
        p64(b, province, cprovince::VTABLE_RVA as u64);
        p32(b, province + cprovince::ID, id);
        p64(b, province + cprovince::DEFINITION, definition as u64);
        p32(b, definition + crate::province_definition::ID, id);
        p64(b, definition + 0xa8, terrain as u64);
        b[definition + crate::province_definition::TERRAIN_FLAGS] = 1;
    }
    array(b, A1 + crate::cunit::ORDINARY_ROUTE_DATA, 0x1f000, 2);
    p32(b, 0x1f000, 2);
    p32(b, 0x1f004, 3);
    p64(b, GS + crate::cgame_state::WEATHER_MANAGER, 0x19000);
    p64(b, 0x19000, crate::cweather_manager::VTABLE_RVA as u64);
    p64(
        b,
        T1 + creferenced_division_template::PRIORITY_UNIT,
        0x1a000,
    );
    p64(b, 0x1a000, crate::csub_unit_definition::VTABLE_RVA as u64);
    p64(b, C1 + ccountry::DIPLOMACY_STATUS, 0x1b000);
    array(b, C1 + ccountry::MODIFIER_ACCUMULATOR, 0x1c000, 1);
    p32(b, 0x1c000, 0x2b);
    p64(b, 0x1c008, 10_001);
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA,
        0x1d000_u64.to_le_bytes().to_vec(),
    );
    memory.globals.insert(
        crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA,
        7_i64.to_le_bytes().to_vec(),
    );
    for (rva, value) in [
        (0x0331_cec0, 50_000_i64),
        (0x0331_97d0, 100_000),
        (0x0331_9f90, 25_000),
        (0x0331_d688, 100_003),
        (0x0331_9758, 25_003),
        (0x0331_efe0, 33_333),
    ] {
        memory.globals.insert(rva, value.to_le_bytes().to_vec());
    }
    memory
}

fn strategic_route_fixture() -> PresentationMemory {
    let mut memory = normal_route_fixture();
    memory.bytes[A1 + carmy::STRATEGIC_REDEPLOYMENT] = 1;
    p64(
        &mut memory.bytes,
        GS + crate::cgame_state::RAIL_MANAGER,
        0x22000,
    );
    p64(
        &mut memory.bytes,
        0x22000,
        crate::crailway_manager::VTABLE_RVA as u64,
    );
    array(&mut memory.bytes, 0x22008, 0x22800, 4);
    for (rva, value) in [
        (0x0331_cf40, 500_000_i64),
        (0x0331_cfd0, 0),
        (0x0331_d068, 100_000),
        (0x0331_d0e8, 0),
    ] {
        memory.globals.insert(rva, value.to_le_bytes().to_vec());
    }
    memory
        .globals
        .insert(0x0331_bfe0, 1_i32.to_le_bytes().to_vec());
    memory
}

struct SeededSpeedGeometry<'a, M> {
    memory: &'a M,
    packet: cprovince::GeometryPacket,
}
impl<'a, M: crate::Memory> SeededSpeedGeometry<'a, M> {
    fn new(memory: &'a M) -> Self {
        let mut packet =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        packet.begin(memory, 0, GS).unwrap();
        Self { memory, packet }
    }
}
impl<M: crate::Memory> super::MovementGeometry for SeededSpeedGeometry<'_, M> {
    fn province_id(&mut self, pointer: usize) -> Result<u32, &'static str> {
        self.packet.acquire_province_id(self.memory, pointer)
    }
    fn route_pointer(&mut self, id: u32) -> Result<usize, &'static str> {
        self.packet.acquire_route_pointer(self.memory, id)
    }
    fn definition(&mut self, id: u32) -> Result<usize, &'static str> {
        self.packet.acquire_definition(self.memory, id)
    }
    fn flags(&mut self, id: u32) -> Result<u8, &'static str> {
        self.packet.acquire_flags(self.memory, id)
    }
    fn crossing(&mut self, from: u32, to: u32) -> Result<Option<u8>, &'static str> {
        self.packet.acquire_crossing(self.memory, from, to)
    }
}
fn speed_seed(memory: &impl crate::Memory) -> super::DivisionSeed {
    let mut seeds = [super::DivisionSeed {
        pointer: A1,
        division: handle(9, 9000),
        ..Default::default()
    }];
    super::capture_division_seeds(memory, &mut seeds);
    seeds[0]
}

#[test]
fn eta_inputs_compose_the_speed_graph_and_one_entry_per_route_leg() {
    let memory = normal_route_fixture();
    let seed = speed_seed(&memory);
    let mut geometry = cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
        provinces: 8,
        neighbour_entries: 8,
    });
    geometry.begin(&memory, 0, GS).unwrap();
    let mut statics = cprovince::StaticGeometry::default();
    let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &[seed],
        0..1,
        Outcome::Rows { len: 1 },
        4,
        16,
        16,
        &mut geometry,
        &mut statics,
        &mut packet,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let row = packet.rows()[0];
    assert_eq!(row.division, seed.division);
    assert_eq!((row.leg_start, row.leg_len), (0, 2));
    // Neither distance nor crossing kind is stored; matching map content
    // supplies both when replay evaluates the admitted legs.
    let legs = packet.legs();
    assert_eq!(packet.leg_count(), 2);
    assert_eq!(
        (legs.river, legs.transport, legs.rail),
        ([0, 0].as_slice(), [0, 0].as_slice(), [0, 0].as_slice())
    );
    assert!(!row.any_river && !row.any_transport && !row.any_rail);
    // The attribution counts this walk's admitted rows and recorded legs,
    // and the geometry memo answered some lookups without reading.
    let counters = packet.counters();
    assert_eq!(
        (counters.walks, counters.divisions, counters.legs),
        (1, 1, 2)
    );
    assert!(counters.geometry_hits > 0 && counters.geometry_misses > 0);
    // Adjuster entry +0x20 plus terrain +0x188, one entry per distinct
    // native terrain index the route touches.
    assert_eq!(packet.terrain(), [(1, 10_000), (2, -12_346)]);
    assert_eq!((row.terrain_start, row.terrain_len), (0, 2));
    assert_eq!(
        row.current_speed,
        super::effective_movement_speed(&memory, 0, GS, A1, P1, 0x18200).unwrap()
    );
    assert_eq!(row.speed_base, 400_003);

    // A stationary division produces no row at all, and a failed census
    // never becomes an empty table.
    let mut stationary = seed;
    stationary.ordinary_route = Some([0; 16]);
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &[stationary],
        0..1,
        Outcome::Rows { len: 1 },
        4,
        16,
        16,
        &mut geometry,
        &mut statics,
        &mut packet,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
    assert!(packet.rows().is_empty());
    let unavailable = Outcome::Unavailable { reason: "census" };
    assert_eq!(
        super::capture_eta_inputs(
            &memory,
            0,
            GS,
            &[],
            0..0,
            unavailable,
            4,
            16,
            16,
            &mut geometry,
            &mut statics,
            &mut packet,
        )
        .outcome,
        unavailable
    );
    // The index is the database's i32 count, not a byte: a mod's 300th
    // category (KaiserreduX declares 1,581) is captured as it stands.
    let mut memory = normal_route_fixture();
    p32(&mut memory.bytes, 0x17400 + 0x88, 300);
    // The sub-unit's adjuster array holds one entry per category, the
    // 300th at element 299.
    const ADJUSTERS: usize = 0x30000;
    memory.bytes.resize(ADJUSTERS + 300 * 0x28, 0);
    memory.bytes.copy_within(0x16000..0x16028, ADJUSTERS);
    memory
        .bytes
        .copy_within(0x16028..0x16050, ADJUSTERS + 299 * 0x28);
    array(&mut memory.bytes, 0x11000 + 0x3f8, ADJUSTERS, 300);
    let mut geometry = cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
        provinces: 8,
        neighbour_entries: 8,
    });
    geometry.begin(&memory, 0, GS).unwrap();
    let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &[speed_seed(&memory)],
        0..1,
        Outcome::Rows { len: 1 },
        4,
        16,
        16,
        &mut geometry,
        &mut cprovince::StaticGeometry::default(),
        &mut packet,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(packet.terrain(), [(1, 10_000), (300, -12_346)]);
}
/// Two moving divisions of different armies cost one composed graph each:
/// releasing an army never re-reads the branches it just acquired.
#[test]
fn eta_inputs_read_each_division_graph_once() {
    let mut source = normal_route_fixture();
    // Exercise the proved ordinary scale; exceptional scales separately
    // test the exact per-edge overflow fallback.
    source.globals.insert(
        super::OUTER_SCALE_RVA,
        super::MOVEMENT_ONE.to_le_bytes().to_vec(),
    );
    let state = 0x28000;
    p64(&mut source.bytes, P1 + cprovince::STATE, state as u64);
    p64(&mut source.bytes, state, crate::cstate::VTABLE_RVA as u64);
    p32(&mut source.bytes, state + crate::cstate::CONTROLLER, 3);
    p64(
        &mut source.bytes,
        state + crate::cstate::COUNTRY_MODIFIERS,
        0x29000,
    );
    let army = source.bytes[A1..A1 + 0x700].to_vec();
    source.bytes[A2..A2 + 0x700].copy_from_slice(&army);
    p64(
        &mut source.bytes,
        A2 + crate::cunit::HANDLE,
        handle(10, 9000),
    );
    let memory = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let mut seeds = [
        super::DivisionSeed {
            pointer: A1,
            division: handle(9, 9000),
            ..Default::default()
        },
        super::DivisionSeed {
            pointer: A2,
            division: handle(10, 9000),
            ..Default::default()
        },
    ];
    super::capture_division_seeds(&memory, &mut seeds);
    let mut geometry = cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
        provinces: 8,
        neighbour_entries: 8,
    });
    geometry.begin(&memory, 0, GS).unwrap();
    let mut statics = cprovince::StaticGeometry::default();
    let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
    memory.reads.borrow_mut().clear();
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &seeds,
        0..2,
        Outcome::Rows { len: 2 },
        4,
        16,
        16,
        &mut geometry,
        &mut statics,
        &mut packet,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    let reads = memory.reads.borrow();
    for army in [A1, A2] {
        let fuel = reads
            .iter()
            .filter(|&&at| at == army + carmy::FUEL_CANDIDATE)
            .count();
        assert_eq!(fuel, 1, "army {army:#x} composed its speed graph once");
    }
    assert_eq!(
        (
            packet.counters().state_modifier_requests,
            packet.counters().state_modifier_hits
        ),
        (4, 2)
    );
    assert_eq!(
        reads
            .iter()
            .filter(|&&at| at == state + crate::cstate::COUNTRY_MODIFIERS)
            .count(),
        2
    );
    // The first normal edge and its ETA leg share their terrain result;
    // distinct terrain on the second leg still has its own acquisition.
    for entry in [0x16000, 0x16028] {
        assert_eq!(reads.iter().filter(|&&at| at == entry + 0x20).count(), 2);
    }
}

#[test]
fn eta_state_query_memo_preserves_raw_owners_failures_collisions_and_new_captures() {
    let mut source = normal_route_fixture();
    let state = 0x28000;
    p64(&mut source.bytes, state, crate::cstate::VTABLE_RVA as u64);
    p64(
        &mut source.bytes,
        state + crate::cstate::COUNTRY_MODIFIERS,
        0x29000,
    );
    array(
        &mut source.bytes,
        state + crate::cstate::RESOURCE_MODIFIER + 0x10,
        0x2a000,
        2,
    );
    for (index, key, value) in [(0, 0x2b, i64::MIN), (1, 0x1e7, i64::MAX)] {
        p32(&mut source.bytes, 0x2a000 + index * 16, key);
        p64(&mut source.bytes, 0x2a008 + index * 16, value as u64);
    }
    let observed = ScalarBatchMemory::new(&source);
    let mut memo = super::MovementStateModifiers::new(4).unwrap();
    for (owner, key, expected) in [
        (3, 0x2b, i64::MIN),
        (1, 0x2b, i64::MIN),
        (3, 0x1e7, i64::MAX),
    ] {
        observed.reads.borrow_mut().clear();
        assert_eq!(
            crate::cstate::movement_modifier(&observed, 0, GS, state, owner, key),
            Ok(expected)
        );
        let direct = observed.reads.borrow().clone();
        observed.reads.borrow_mut().clear();
        memo.clear();
        assert_eq!(memo.get(&observed, 0, GS, state, owner, key), Ok(expected));
        assert_eq!(*observed.reads.borrow(), direct);
        observed.reads.borrow_mut().clear();
        assert_eq!(memo.get(&observed, 0, GS, state, owner, key), Ok(expected));
        assert!(observed.reads.borrow().is_empty());
        assert_eq!((memo.requests, memo.hits), (2, 1));
        // Every original fault on a cold query retains its exact read
        // prefix and reason. No failed result is installed in the memo.
        for &(fault, _) in &direct {
            memo.clear();
            observed.fault.set(Some(fault));
            observed.reads.borrow_mut().clear();
            let expected = crate::cstate::movement_modifier(&observed, 0, GS, state, owner, key);
            assert!(expected.is_err());
            let prefix = observed.reads.borrow().clone();
            for _ in 0..2 {
                observed.reads.borrow_mut().clear();
                assert_eq!(memo.get(&observed, 0, GS, state, owner, key), expected);
                assert_eq!(*observed.reads.borrow(), prefix);
            }
            assert_eq!(memo.hits, 0);
        }
        observed.fault.set(None);
    }
    // One cell forces replacement: every changed raw owner, key, state,
    // module, and game root must miss, even for aliased native countries.
    let mut one = super::MovementStateModifiers::new(1).unwrap();
    for (base, game, object, owner, key) in [
        (0, GS, state, 3, 0x2b),
        (0, GS, state, 1, 0x2b),
        (0, GS, state, 3, 0x1e7),
        (0, GS, state, 3, 0x2b),
        (1, GS, state, 3, 0x2b),
        (0, 0, state, 3, 0x2b),
        (0, GS, 0, 3, 0x2b),
    ] {
        observed.reads.borrow_mut().clear();
        let expected = crate::cstate::movement_modifier(&observed, base, game, object, owner, key);
        let direct = observed.reads.borrow().clone();
        observed.reads.borrow_mut().clear();
        assert_eq!(one.get(&observed, base, game, object, owner, key), expected);
        assert_eq!(*observed.reads.borrow(), direct);
    }
    assert_eq!(one.hits, 0);
    drop(observed);
    p64(&mut source.bytes, 0x2a008, (-73_i64) as u64);
    one.clear();
    assert_eq!(one.get(&source, 0, GS, state, 3, 0x2b), Ok(-73));
    let mut vacant = super::MovementStateModifiers::new(0).unwrap();
    assert_eq!(vacant.allocated_bytes(), 0);
    assert_eq!(vacant.get(&source, 0, GS, state, 3, 0x2b), Ok(-73));
    assert_eq!(vacant.allocated_bytes(), 0);
    vacant.reserve(3).unwrap();
    assert!(vacant.allocated_bytes() >= 3 * size_of::<Option<super::MovementStateModifier>>());
    assert_eq!(vacant.get(&source, 0, GS, state, 3, 0x2b), Ok(-73));
    assert_eq!(vacant.hits, 0);
}

#[test]
fn the_static_map_memo_reuses_primary_inputs_until_the_session_changes() {
    let mut memory = normal_route_fixture();
    // Province 1 neighbours province 2, while the session memo retains
    // only province properties needed to acquire ETA operands.
    let neighbour = 0x1e000;
    array(
        &mut memory.bytes,
        0x12000 + crate::province_definition::NEIGHBOURS,
        neighbour,
        1,
    );
    p32(
        &mut memory.bytes,
        neighbour + crate::province_neighbour::PROVINCE_ID,
        2,
    );
    let seed = speed_seed(&memory);
    let mut statics = cprovince::StaticGeometry::default();
    let walk = |memory: &PresentationMemory,
                statics: &mut cprovince::StaticGeometry,
                packet: &mut super::EtaInputPacket| {
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        geometry.enable_growth();
        geometry.begin(memory, 0, GS).unwrap();
        super::capture_eta_inputs(
            memory,
            0,
            GS,
            &[seed],
            0..1,
            Outcome::Rows { len: 1 },
            4,
            16,
            16,
            &mut geometry,
            statics,
            packet,
        )
    };
    let mut cold = super::EtaInputPacket::new(4, 16, 16).unwrap();
    assert_eq!(
        walk(&memory, &mut statics, &mut cold).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(cold.leg_count(), 2);
    // A cold walk reads each distinct province once; repeated lookups
    // inside the same route are already hits.
    assert!(cold.counters().static_misses > 0);
    // The next hour reuses immutable provinces while the route's live
    // acquisition still produces the same number of admitted legs.
    let mut warm = super::EtaInputPacket::new(4, 16, 16).unwrap();
    walk(&memory, &mut statics, &mut warm);
    assert_eq!(cold.outcome(), warm.outcome());
    assert_eq!(cold.rows(), warm.rows());
    assert_eq!(cold.terrain(), warm.terrain());
    assert_eq!(cold.leg_count(), warm.leg_count());
    assert!(warm.counters().static_hits > 0);
    assert!(warm.counters().static_misses < cold.counters().static_misses);
    // A reload changes the play session, which clears the whole memo.
    memory.globals.insert(
        crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA,
        8_i64.to_le_bytes().to_vec(),
    );
    let mut reloaded = super::EtaInputPacket::new(4, 16, 16).unwrap();
    walk(&memory, &mut statics, &mut reloaded);
    assert_eq!(reloaded.counters().static_cleared, 1);
    assert_eq!(reloaded.leg_count(), warm.leg_count());
}

#[test]
fn embark_define_refreshes_without_a_session_or_province_census_change() {
    let mut memory = normal_route_fixture();
    let define = crate::cunit::EMBARK_MOVEMENT_DISTANCE_RVA;
    let mut statics = cprovince::StaticGeometry::default();
    memory
        .globals
        .insert(define, 12_345_i64.to_le_bytes().to_vec());
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.embark(&memory, 0), Ok(12_345));
    let key = memory.globals[&crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA].clone();
    // Reuse within one frozen acquisition, then emulate define reload
    // between hours without replacing any key field or province array.
    memory
        .globals
        .insert(define, 67_890_i64.to_le_bytes().to_vec());
    assert_eq!(statics.embark(&memory, 0), Ok(12_345));
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.counters().cleared, 0);
    assert_eq!(statics.embark(&memory, 0), Ok(67_890));
    assert_eq!(
        memory.globals[&crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA],
        key
    );

    let unreadable = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: Some(define),
    };
    assert!(statics.begin(&unreadable, 0, GS));
    assert!(statics.embark(&unreadable, 0).is_err());
    assert!(unreadable.reads.borrow().contains(&define));
}

#[test]
fn adjacency_reload_refreshes_crossings_absence_and_reallocated_arrays() {
    let mut memory = normal_route_fixture();
    let header = 0x12000 + crate::province_definition::NEIGHBOURS;
    let stride = crate::province_neighbour::STRIDE;
    let id = crate::province_neighbour::PROVINCE_ID;
    let distance = crate::province_neighbour::MOVEMENT_DISTANCE;
    let crossing = crate::province_neighbour::CROSSING_KIND;
    array(&mut memory.bytes, header, 0x1e000, 1);
    p32(&mut memory.bytes, 0x1e000 + id, 2);
    p64(&mut memory.bytes, 0x1e000 + distance, 4_321);
    memory.bytes[0x1e000 + crossing] = 1;
    let mut statics = cprovince::StaticGeometry::default();
    assert!(statics.begin(&memory, 0, GS));
    let old = statics.edge(&memory, 0, 1, 2).unwrap();
    assert_eq!((old.distance, old.crossing), (4_321, Some(1)));
    assert_eq!(statics.edge(&memory, 0, 1, 3).unwrap().crossing, None);

    // A straits reload can modify an existing crossing and append an
    // adjacency, reallocating only the definition's neighbour array.
    array(&mut memory.bytes, header, 0x1e100, 2);
    for (slot, target, length, kind) in [(0, 2, 9_876, 3), (1, 3, 4_567, 2)] {
        let entry = 0x1e100 + slot * stride;
        p32(&mut memory.bytes, entry + id, target);
        p64(&mut memory.bytes, entry + distance, length);
        memory.bytes[entry + crossing] = kind;
    }
    assert_eq!(statics.edge(&memory, 0, 1, 2), Ok(old));
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.counters().cleared, 0);
    let changed = statics.edge(&memory, 0, 1, 2).unwrap();
    assert_eq!((changed.distance, changed.crossing), (9_876, Some(3)));
    let appended = statics.edge(&memory, 0, 1, 3).unwrap();
    assert_eq!((appended.distance, appended.crossing), (4_567, Some(2)));

    let unreadable = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: Some(0x1e100 + id),
    };
    assert!(statics.begin(&unreadable, 0, GS));
    assert!(statics.edge(&unreadable, 0, 1, 2).is_err());
    assert!(unreadable.reads.borrow().contains(&(0x1e100 + id)));
}

#[test]
fn contiguous_ranges_concatenate_into_the_unsplit_rows() {
    // The stop fixture zeroes the outer scale, so its route scan ends at
    // the first leg; both censuses must survive the same cuts.
    let mut stopped = normal_route_fixture();
    stopped
        .globals
        .insert(0x0331_d688, 0_i64.to_le_bytes().to_vec());
    for memory in [normal_route_fixture(), stopped, strategic_route_fixture()] {
        let moving = speed_seed(&memory);
        let mut stationary = moving;
        stationary.ordinary_route = Some([0; 16]);
        let seeds: Vec<_> = (0..8).flat_map(|_| [moving, stationary]).collect();
        // Ranges own separate arenas, so rebase each row onto the running
        // concatenation exactly as the table conversion does.
        let walk = |spans: &[std::ops::Range<usize>]| {
            let (mut rows, mut terrain) = (Vec::new(), Vec::new());
            let (mut river, mut transport, mut rail) = (Vec::new(), Vec::new(), Vec::new());
            let mut leg_count = 0;
            for span in spans {
                let mut geometry =
                    cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                        provinces: 8,
                        neighbour_entries: 8,
                    });
                geometry.begin(&memory, 0, GS).unwrap();
                let mut statics = cprovince::StaticGeometry::default();
                let mut packet = super::EtaInputPacket::new(64, 128, 256).unwrap();
                let walk = super::capture_eta_inputs(
                    &memory,
                    0,
                    GS,
                    &seeds,
                    span.clone(),
                    Outcome::Rows { len: seeds.len() },
                    64,
                    128,
                    256,
                    &mut geometry,
                    &mut statics,
                    &mut packet,
                );
                assert!(matches!(walk.outcome, Outcome::Rows { .. }), "{walk:?}");
                for row in packet.rows() {
                    let mut row = *row;
                    row.terrain_start += terrain.len();
                    row.leg_start += leg_count;
                    rows.push(row);
                }
                terrain.extend_from_slice(packet.terrain());
                leg_count += packet.leg_count();
                let legs = packet.legs();
                river.extend_from_slice(legs.river);
                transport.extend_from_slice(legs.transport);
                rail.extend_from_slice(legs.rail);
            }
            (rows, terrain, leg_count, river, transport, rail)
        };
        let single = walk(std::slice::from_ref(&(0..seeds.len())));
        assert_eq!(single.0.len(), 8);
        for ranges in [1_usize, 3, 8] {
            let mut spans = Vec::new();
            let mut start = 0;
            for part in 0..ranges {
                let end = seeds.len() * (part + 1) / ranges;
                spans.push(start..end);
                start = end;
            }
            assert_eq!(walk(&spans), single, "{ranges} ranges");
        }
        // A range that cannot read its own seeds fails alone; the ranges
        // beside it still produce their rows, and the caller denies the
        // table because one of them is unavailable.
        let mut corrupt = seeds.clone();
        corrupt[9].primary_vtable = Some(1);
        let range = |span: std::ops::Range<usize>| {
            let mut geometry =
                cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                    provinces: 8,
                    neighbour_entries: 8,
                });
            geometry.begin(&memory, 0, GS).unwrap();
            let mut statics = cprovince::StaticGeometry::default();
            let mut packet = super::EtaInputPacket::new(64, 128, 256).unwrap();
            super::capture_eta_inputs(
                &memory,
                0,
                GS,
                &corrupt,
                span,
                Outcome::Rows { len: corrupt.len() },
                64,
                128,
                256,
                &mut geometry,
                &mut statics,
                &mut packet,
            )
            .outcome
        };
        assert_eq!(range(0..8), Outcome::Rows { len: 4 });
        assert!(matches!(range(8..16), Outcome::Unavailable { .. }));
    }
}

#[test]
fn the_route_scan_stops_where_the_native_walk_stops() {
    let capture = |memory: &PresentationMemory, packet: &mut super::EtaInputPacket| {
        let seed = speed_seed(memory);
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        geometry.enable_growth();
        geometry.begin(memory, 0, GS).unwrap();
        let mut statics = cprovince::StaticGeometry::default();
        super::capture_eta_inputs(
            memory,
            0,
            GS,
            &[seed],
            0..1,
            Outcome::Rows { len: 1 },
            4,
            16,
            16,
            &mut geometry,
            &mut statics,
            packet,
        )
    };
    // The second leg's terrain is the null slot, so reading that leg
    // fails. The native walk reaches it, so the table is denied.
    let mut memory = normal_route_fixture();
    p32(&mut memory.bytes, 0x17400 + 0x88, 0);
    let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
    assert!(matches!(
        capture(&memory, &mut packet).outcome,
        Outcome::Unavailable { .. }
    ));
    // A zero outer scale zeroes the first leg's speed, so the native walk
    // never evaluates the second and neither does the scan: the same
    // unreadable tail now costs nothing.
    memory
        .globals
        .insert(0x0331_d688, 0_i64.to_le_bytes().to_vec());
    assert_eq!(
        capture(&memory, &mut packet).outcome,
        Outcome::Rows { len: 1 }
    );
    let row = packet.rows()[0];
    assert_eq!(row.leg_len, 1);
    assert_eq!(packet.leg_count(), 1);
    assert_eq!(packet.terrain(), [(1, 10_000)]);
    // A readable tail past the stop is still not recorded.
    let mut memory = normal_route_fixture();
    memory
        .globals
        .insert(0x0331_d688, 0_i64.to_le_bytes().to_vec());
    assert_eq!(
        capture(&memory, &mut packet).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(packet.rows()[0].leg_len, 1);
}

#[test]
fn normal_scale_zero_proof_preserves_wrapping_boundaries() {
    assert_eq!(super::normal_speed_zero_from_scale(0), Some(true));
    for scale in [-100_000_i64, -99_999, -1, 1, 99_999, 100_000] {
        assert_eq!(super::normal_speed_zero_from_scale(scale), Some(false));
        for speed in [100_000, 100_001, i64::MAX / 100_000] {
            assert_ne!(super::movement_product(speed, scale), 0);
        }
    }
    for scale in [i64::MIN, -100_001, 100_001, 1_i64 << 59, i64::MAX] {
        assert_eq!(super::normal_speed_zero_from_scale(scale), None);
    }
    assert_eq!(super::movement_product(100_000, 1_i64 << 59), 0);
}

#[test]
fn eta_normal_wrapping_zero_skips_first_and_later_unreadable_tails() {
    for first in [true, false] {
        let mut source = normal_route_fixture();
        source
            .globals
            .insert(super::OUTER_SCALE_RVA, (1_i64 << 59).to_le_bytes().to_vec());
        let fault = if first {
            // The first speed is floored at 100000, whose final scaled
            // product wraps to zero despite the nonzero outer scale.
            p64(&mut source.bytes, 0x11000 + 0x290, 0);
            0x17400 + crate::cterrain_type::DATABASE_INDEX
        } else {
            // Keep the first edge moving; the second terrain penalty
            // floors that edge at 100000. Only the third leg is unreadable.
            p64(&mut source.bytes, 0x16028 + 0x20, (-10_000_000_i64) as u64);
            array(
                &mut source.bytes,
                GS + crate::cgame_state::PROVINCES,
                PROVINCES,
                5,
            );
            p64(&mut source.bytes, PROVINCES + 4 * 8, 0x18800);
            p64(&mut source.bytes, 0x18800, cprovince::VTABLE_RVA as u64);
            p32(&mut source.bytes, 0x18800 + cprovince::ID, 4);
            p64(&mut source.bytes, 0x18800 + cprovince::DEFINITION, 0x18900);
            p32(
                &mut source.bytes,
                0x18900 + crate::province_definition::ID,
                4,
            );
            source.bytes[0x18900 + crate::province_definition::TERRAIN_FLAGS] = 1;
            p64(
                &mut source.bytes,
                0x18900 + crate::province_definition::TERRAIN,
                0x17800,
            );
            array(
                &mut source.bytes,
                A1 + crate::cunit::ORDINARY_ROUTE_DATA,
                0x1f000,
                3,
            );
            p32(&mut source.bytes, 0x1f008, 4);
            0x17800 + crate::cterrain_type::DATABASE_INDEX
        };
        let first_speed = super::effective_movement_speed(&source, 0, GS, A1, P1, 0x18200).unwrap();
        assert_eq!(first_speed == 0, first);
        if !first {
            assert_eq!(
                super::effective_movement_speed(&source, 0, GS, A1, 0x18200, 0x18400),
                Ok(0)
            );
        }
        let memory = ObservedMovementMemory {
            source: &source,
            reads: Default::default(),
            fault: Some(fault),
        };
        let seed = speed_seed(&memory);
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        geometry.begin(&memory, 0, GS).unwrap();
        let mut statics = cprovince::StaticGeometry::default();
        let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
        let walk = super::capture_eta_inputs(
            &memory,
            0,
            GS,
            &[seed],
            0..1,
            Outcome::Rows { len: 1 },
            4,
            16,
            16,
            &mut geometry,
            &mut statics,
            &mut packet,
        );
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        assert_eq!(packet.rows()[0].leg_len, if first { 1 } else { 2 });
        assert!(!memory.reads.borrow().contains(&fault));
    }
}

#[test]
fn eta_transport_does_not_require_unconsumed_land_or_rail_inputs() {
    for speed in [0_i64, 600_000] {
        let mut source = normal_route_fixture();
        p64(&mut source.bytes, A1 + super::NAVAL_TRANSFER_MEMBERSHIP, 1);
        for province in [P1, 0x18200] {
            p64(
                &mut source.bytes,
                province + cprovince::STRATEGIC_REGION,
                0x28000,
            );
        }
        p64(
            &mut source.bytes,
            0x28000,
            crate::cstrategic_region::VTABLE_RVA as u64,
        );
        p32(&mut source.bytes, 0x28000 + crate::cstrategic_region::ID, 1);
        p32(
            &mut source.bytes,
            0x28000 + crate::cstrategic_region::RUNTIME_SLOT,
            1,
        );
        array(
            &mut source.bytes,
            GS + crate::cgame_state::STRATEGIC_REGIONS,
            0x28800,
            2,
        );
        p64(&mut source.bytes, 0x28808, 0x28000);
        source.globals.insert(
            crate::cnaval_unit_transfer::BASE_SPEED_RVA,
            speed.to_le_bytes().to_vec(),
        );
        source.globals.insert(
            crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
            0_u32.to_le_bytes().to_vec(),
        );
        let fault = P1 + cprovince::STATE;
        let memory = ObservedMovementMemory {
            source: &source,
            reads: Default::default(),
            fault: Some(fault),
        };
        let mut seed = speed_seed(&memory);
        seed.expeditionary_owner = None;
        seed.roster_owner = None;
        seed.strategic_redeployment = None;
        seed.stat_block = None;
        seed.organisation = None;
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        geometry.begin(&memory, 0, GS).unwrap();
        let mut statics = cprovince::StaticGeometry::default();
        let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
        let walk = super::capture_eta_inputs(
            &memory,
            0,
            GS,
            &[seed],
            0..1,
            Outcome::Rows { len: 1 },
            4,
            16,
            16,
            &mut geometry,
            &mut statics,
            &mut packet,
        );
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        let row = packet.rows()[0];
        assert_eq!(row.leg_len, if speed == 0 { 1 } else { 2 });
        assert!(row.any_transport);
        assert!(!row.any_rail);
        assert!(!memory.reads.borrow().contains(&fault));
    }
}

#[test]
fn eta_rail_graph_and_scan_share_directed_queries_and_refresh_each_capture() {
    let mut source = strategic_route_fixture();
    array(
        &mut source.bytes,
        A1 + crate::cunit::ORDINARY_ROUTE_DATA,
        0x1f000,
        4,
    );
    for (index, id) in [2, 1, 2, 1].into_iter().enumerate() {
        p32(&mut source.bytes, 0x1f000 + index * 4, id);
    }
    let army = source.bytes[A1..A1 + 0x700].to_vec();
    source.bytes[A2..A2 + 0x700].copy_from_slice(&army);
    p64(
        &mut source.bytes,
        A2 + crate::cunit::HANDLE,
        handle(10, 9000),
    );
    let mut seeds = [
        super::DivisionSeed {
            pointer: A1,
            division: handle(9, 9000),
            ..Default::default()
        },
        super::DivisionSeed {
            pointer: A2,
            division: handle(10, 9000),
            ..Default::default()
        },
    ];
    super::capture_division_seeds(&source, &mut seeds);
    let mut expected_outputs = [None, None];
    for capacity in [16, 1, 0] {
        let mut packet = super::EtaInputPacket::new(4, 16, 16).unwrap();
        packet.rail = crate::crailway_manager::MovementSpeeds::new(capacity);
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 8,
                neighbour_entries: 8,
            });
        let mut statics = cprovince::StaticGeometry::default();
        for (capture, (raw_speed, scaled_speed)) in [(500_003_i64, 500_018_i64), (700_009, 700_030)]
            .into_iter()
            .enumerate()
        {
            source
                .globals
                .insert(0x0331_cf40, raw_speed.to_le_bytes().to_vec());
            let observed = ScalarBatchMemory::new(&source);
            geometry.begin(&observed, 0, GS).unwrap();
            observed.reads.borrow_mut().clear();
            let walk = super::capture_eta_inputs(
                &observed,
                0,
                GS,
                &seeds,
                0..2,
                Outcome::Rows { len: 2 },
                4,
                16,
                16,
                &mut geometry,
                &mut statics,
                &mut packet,
            );
            assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
            for (index, row) in packet.rows().iter().enumerate() {
                assert_eq!(row.division, seeds[index].division);
                assert_eq!((row.leg_start, row.leg_len), (index * 4, 4));
                assert_eq!(row.current_speed, scaled_speed);
                assert!(row.any_rail && !row.any_transport && !row.any_river);
                assert_eq!(row.rail_country_modifier, Some(0));
                assert!(row.rail_state_modifier.is_none_or(|value| value == 0));
                assert!(
                    row.rail_friendly_state_modifier
                        .is_none_or(|value| value == 0)
                );
            }
            let legs = packet.legs();
            assert_eq!(legs.rail, [raw_speed; 8]);
            let output = (
                packet.rows().to_vec(),
                packet.terrain().to_vec(),
                packet.leg_count(),
                legs.river.to_vec(),
                legs.transport.to_vec(),
                legs.rail.to_vec(),
            );
            if let Some(expected) = &expected_outputs[capture] {
                assert_eq!(&output, expected);
            } else {
                expected_outputs[capture] = Some(output);
            }
            // Current roots and defines are now shared across different
            // directed edges as well. Vacant storage retains direct reads;
            // every admitted memo reacquires these markers once per hour.
            for address in [GS + crate::cgame_state::RAIL_MANAGER, 0x0331_cf40] {
                assert_eq!(
                    observed
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(at, _)| at == address)
                        .count(),
                    1,
                    "capacity={capacity} capture={capture} marker={address:#x}"
                );
            }
        }
    }
}

#[test]
fn seeded_rail_reuse_keeps_fault_order_independent_gates_and_self_edges() {
    let source = strategic_route_fixture();
    let run = |source: &PresentationMemory,
               fault: Option<usize>,
               memo: Option<&mut crate::crailway_manager::MovementSpeeds>,
               from: usize,
               to: usize| {
        let seed = speed_seed(source);
        let observed = ScalarBatchMemory::new(source);
        let mut geometry = SeededSpeedGeometry::new(&observed);
        observed.reads.borrow_mut().clear();
        observed.fault.set(fault);
        let value = super::movement_speed_inputs_seeded_in_pass(
            &observed,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
                geometry: &mut geometry,
                rail_speeds: memo,
                state_modifiers: None,
                category_contributions: None,
                supply_templates: None,
                air_prefixes: None,
            },
            from,
            to,
            &mut super::NormalMovementContext::default(),
            |_, _| unreachable!("strategic fixture must not select transport"),
        );
        (value, observed.reads.take())
    };
    let (direct, direct_reads) = run(&source, None, None, P1, 0x18200);
    assert!(direct.is_ok());
    let mut memo = crate::crailway_manager::MovementSpeeds::new(1);
    let (cold, cold_reads) = run(&source, None, Some(&mut memo), P1, 0x18200);
    assert_eq!(cold, direct);
    assert_eq!(cold_reads, direct_reads);
    let (warm, warm_reads) = run(&source, None, Some(&mut memo), P1, 0x18200);
    assert_eq!(warm, direct);
    assert!(!warm_reads.contains(&(GS + crate::cgame_state::RAIL_MANAGER, 8)));
    assert!(!warm_reads.contains(&(0x0331_cf40, 8)));
    assert!(warm_reads.contains(&(super::OUTER_SCALE_RVA, 8)));

    // A new context must still authenticate this graph before the rail hit;
    // the independent outer scale must still be read after that hit.
    for address in [
        P1 + cprovince::STATE,
        C1 + ccountry::MODIFIER_ACCUMULATOR,
        super::OUTER_SCALE_RVA,
    ] {
        let (expected, direct_prefix) = run(&source, Some(address), None, P1, 0x18200);
        let (actual, prefix) = run(&source, Some(address), Some(&mut memo), P1, 0x18200);
        assert!(expected.is_err(), "fault={address:#x}");
        assert_eq!(actual, expected);
        assert!(!prefix.contains(&(GS + crate::cgame_state::RAIL_MANAGER, 8)));
        if address == super::OUTER_SCALE_RVA {
            assert_eq!(prefix.last(), Some(&(address, 8)));
            assert!(direct_prefix.contains(&(GS + crate::cgame_state::RAIL_MANAGER, 8)));
        } else {
            assert_eq!(prefix, direct_prefix);
        }
    }

    // Existing railway fixtures cover every low-level fault. These two
    // late query failures verify graph integration and no failed admission,
    // with both vacant and bounded storage, without duplicating that matrix.
    for address in [GS + crate::cgame_state::RAIL_MANAGER, 0x0331_bfe0] {
        let (expected, prefix) = run(&source, Some(address), None, P1, 0x18200);
        assert!(expected.is_err());
        for capacity in [0, 1] {
            let mut failed = crate::crailway_manager::MovementSpeeds::new(capacity);
            for _ in 0..2 {
                let (actual, reads) = run(&source, Some(address), Some(&mut failed), P1, 0x18200);
                assert_eq!(actual, expected);
                assert_eq!(reads, prefix);
            }
        }
    }

    // The route scan omits self edges. They must therefore stay direct in
    // the graph and leave a bounded slot free for a real directed edge.
    let mut self_edges = crate::crailway_manager::MovementSpeeds::new(1);
    let (same, same_reads) = run(&source, None, None, P1, P1);
    assert!(same.is_ok());
    for _ in 0..2 {
        let (actual, reads) = run(&source, None, Some(&mut self_edges), P1, P1);
        assert_eq!(actual, same);
        assert_eq!(reads, same_reads);
    }
    let (actual, reads) = run(&source, None, Some(&mut self_edges), P1, 0x18200);
    assert_eq!(actual, direct);
    assert_eq!(reads, direct_reads);
    let (actual, reads) = run(&source, None, Some(&mut self_edges), P1, 0x18200);
    assert_eq!(actual, direct);
    assert!(!reads.contains(&(GS + crate::cgame_state::RAIL_MANAGER, 8)));

    // A retained edge result cannot override a freshly blocked rail branch.
    let mut blocked = strategic_route_fixture();
    array(
        &mut blocked.bytes,
        C1 + ccountry::MODIFIER_ACCUMULATOR,
        0x1c000,
        2,
    );
    p32(&mut blocked.bytes, 0x1c010, 0x1ea);
    p64(&mut blocked.bytes, 0x1c018, 1);
    let (expected, direct_reads) = run(&blocked, None, None, P1, 0x18200);
    let (actual, reads) = run(&blocked, None, Some(&mut memo), P1, 0x18200);
    assert!(expected.is_ok());
    assert_eq!(actual, expected);
    assert_eq!(reads, direct_reads);
    assert!(!reads.contains(&(GS + crate::cgame_state::RAIL_MANAGER, 8)));
}

#[test]
fn seeded_speed_matches_normal_transport_invasion_and_rail_branches() {
    for branch in 0..4 {
        let mut memory = normal_route_fixture();
        let (from, to) = if branch == 2 {
            (0x18200, 0x18400)
        } else {
            (P1, 0x18200)
        };
        match branch {
            1 => p64(&mut memory.bytes, A1 + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1),
            2 => {
                memory.bytes[0x12000 + crate::province_definition::TERRAIN_FLAGS] = 0;
                memory
                    .globals
                    .insert(0x0331_ead8, 123_457_i64.to_le_bytes().to_vec());
            }
            3 => memory = strategic_route_fixture(),
            _ => {}
        }
        let expected =
            super::effective_movement_speed_with_transport(&memory, 0, GS, A1, from, to, |_, _| {
                Ok(200_007)
            })
            .unwrap();
        let mut seed = speed_seed(&memory);
        if branch != 0 {
            // These branches must remain independent of the normal stat graph.
            seed.stat_block = None;
            seed.organisation = None;
        }
        let mut geometry = SeededSpeedGeometry::new(&memory);

        let mut context = super::NormalMovementContext::default();
        let inputs = super::movement_speed_inputs_seeded_in_pass(
            &memory,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
                geometry: &mut geometry,
                rail_speeds: None,
                state_modifiers: None,
                category_contributions: None,
                supply_templates: None,
                air_prefixes: None,
            },
            from,
            to,
            &mut context,
            |_, _| Ok(200_007),
        )
        .unwrap();
        assert_eq!(inputs.evaluate(), expected, "branch {branch}");
        assert_eq!(context.normal.is_some(), branch == 0);
    }
}

#[test]
fn seeded_normal_speed_uses_owned_fields_without_retrying_live_faults() {
    let memory = normal_route_fixture();
    let seed = speed_seed(&memory);
    let expected = super::effective_movement_speed(&memory, 0, GS, A1, P1, 0x18200).unwrap();
    for offset in [
        crate::cunit::STAT_BLOCK,
        carmy::ORGANISATION,
        crate::cunit::ORDINARY_ROUTE_DATA,
    ] {
        let observed = ObservedMovementMemory {
            source: &memory,
            reads: Default::default(),
            fault: Some(A1 + offset),
        };
        let mut geometry = SeededSpeedGeometry::new(&memory);

        let inputs = super::movement_speed_inputs_seeded_in_pass(
            &observed,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
                geometry: &mut geometry,
                rail_speeds: None,
                state_modifiers: None,
                category_contributions: None,
                supply_templates: None,
                air_prefixes: None,
            },
            P1,
            0x18200,
            &mut super::NormalMovementContext::default(),
            |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(inputs.evaluate(), expected);

        assert!(!observed.reads.borrow().contains(&(A1 + offset)));
    }
    let mut seed = seed;
    seed.stat_block = None;
    let mut geometry = SeededSpeedGeometry::new(&memory);

    assert_eq!(
        super::movement_speed_inputs_seeded_in_pass(
            &memory,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
                geometry: &mut geometry,
                rail_speeds: None,
                state_modifiers: None,
                category_contributions: None,
                supply_templates: None,
                air_prefixes: None,
            },
            P1,
            0x18200,
            &mut super::NormalMovementContext::default(),
            |_, _| unreachable!(),
        ),
        Err("division presentation field unreadable")
    );
}

#[test]
fn seeded_zero_speed_preserves_nullable_and_id_zero_ordinary_tail_predicates() {
    for tail_id in [3_u32, 0] {
        let mut memory = normal_route_fixture();
        if tail_id == 3 {
            p64(&mut memory.bytes, PROVINCES + 3 * 8, 0);
        } else {
            p64(&mut memory.bytes, PROVINCES, 0x18400);
            p32(&mut memory.bytes, 0x18400 + cprovince::ID, 0);
            p32(
                &mut memory.bytes,
                0x18700 + crate::province_definition::ID,
                0,
            );
            p32(&mut memory.bytes, 0x1f004, 0);
        }
        memory
            .globals
            .insert(0x0331_d688, 0_i64.to_le_bytes().to_vec());
        assert_eq!(
            super::effective_movement_speed(&memory, 0, GS, A1, P1, 0x18200),
            Ok(0)
        );
        let seed = speed_seed(&memory);
        let mut geometry = SeededSpeedGeometry::new(&memory);

        let mut context = super::NormalMovementContext::default();
        let inputs = super::movement_speed_inputs_seeded_in_pass(
            &memory,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, tail_id]),
                geometry: &mut geometry,
                rail_speeds: None,
                state_modifiers: None,
                category_contributions: None,
                supply_templates: None,
                air_prefixes: None,
            },
            P1,
            0x18200,
            &mut context,
            |_, _| unreachable!(),
        )
        .unwrap();
        assert!(inputs.is_zero());
        assert_eq!(inputs.evaluate(), 0);
    }
}

#[test]
fn normal_context_matches_uncached_route_and_is_fresh_in_each_pass() {
    let mut memory = normal_route_fixture();
    for expected in [[362_522, 306_655], [375_023, 319_155]] {
        let mut context = super::NormalMovementContext::default();
        for ((from, to), expected) in [(P1, 0x18200), (0x18200, 0x18400)]
            .into_iter()
            .zip(expected)
        {
            let direct = super::effective_movement_speed(&memory, 0, GS, A1, from, to);
            let shared = super::effective_movement_speed_in_pass(
                &memory,
                0,
                GS,
                A1,
                from,
                to,
                &mut context,
                |_, _| panic!("ordinary edge selected transport"),
            );
            assert_eq!(direct, Ok(expected));
            assert_eq!(shared, direct);
            assert!(context.normal.is_some());
        }
        assert!(context.normal.is_some());
        // The independent next pass must acquire the changed common term.
        p64(&mut memory.bytes, 0x1c008, 15_001);
    }
}

#[test]
fn normal_context_switches_armies_before_the_next_first_read() {
    let mut memory = normal_route_fixture();
    p64(&mut memory.bytes, A2 + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
    let mut context = super::NormalMovementContext::default();
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| unreachable!()
        ),
        Ok(362_522)
    );
    let observed = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: Some(A1),
    };
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &observed,
            0,
            GS,
            A2,
            P1,
            0x18200,
            &mut context,
            |_, _| Err("transport acquisition failed")
        ),
        Err("transport acquisition failed")
    );
    assert_eq!(observed.reads.borrow()[0], A2);
    assert!(!observed.reads.borrow().contains(&A1));
    assert_eq!(context.army, A2);
    assert!(context.normal.is_none());
}

#[test]
fn branch_context_reuses_first_army_inputs_across_route_edges() {
    let memory = normal_route_fixture();
    let observed = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: None,
    };
    let edges = [(P1, 0x18200), (0x18200, 0x18400), (P1, 0x18200)];
    for _ in 0..2 {
        for (from, to) in edges {
            assert!(super::effective_movement_speed(&observed, 0, GS, A1, from, to).is_ok());
        }
    }
    let direct_reads = observed.reads.replace(Vec::new());
    let mut context = super::NormalMovementContext::default();
    for _ in 0..2 {
        for (from, to) in edges {
            assert_eq!(
                super::effective_movement_speed_in_pass(
                    &observed,
                    0,
                    GS,
                    A1,
                    from,
                    to,
                    &mut context,
                    |_, _| unreachable!()
                ),
                super::effective_movement_speed(&memory, 0, GS, A1, from, to)
            );
        }
    }
    let shared_reads = observed.reads.borrow();
    let membership = A1 + carmy::NAVAL_TRANSFER_MEMBERSHIP;
    assert_eq!(direct_reads.iter().filter(|&&p| p == membership).count(), 6);
    assert_eq!(shared_reads.iter().filter(|&&p| p == membership).count(), 1);
    assert!(shared_reads.len() < direct_reads.len());
    // Destination identity remains independently read on every route/current
    // edge, even though the army-level decision graph is shared.
    assert!(shared_reads.iter().filter(|&&p| p == 0x18400).count() >= 2);
}

#[test]
fn branch_context_invasion_stays_lazy_and_retains_its_first_constant() {
    let mut memory = normal_route_fixture();
    memory.bytes[0x12000 + crate::province_definition::TERRAIN_FLAGS] = 0;
    p64(&mut memory.bytes, A1 + crate::cunit::STAT_BLOCK, u64::MAX);
    memory
        .globals
        .insert(0x0331_ead8, 123_457_i64.to_le_bytes().to_vec());
    let mut context = super::NormalMovementContext::default();
    // Invasion selection uses the actual location/first route province,
    // independently of the two hypothetical land provinces supplied here.
    for _ in 0..2 {
        assert_eq!(
            super::effective_movement_speed_in_pass(
                &memory,
                0,
                GS,
                A1,
                0x18200,
                0x18400,
                &mut context,
                |_, _| panic!("invasion must not acquire transport")
            ),
            super::effective_movement_speed(&memory, 0, GS, A1, 0x18200, 0x18400)
        );
    }
    assert_eq!(context.invasion_speed, Some(Ok(123_457)));
    assert!(context.roots.is_none());
    assert!(context.rail.is_none());
    assert!(context.normal.is_none());
}

#[test]
fn branch_context_blocked_rail_keeps_its_first_branch_operands() {
    let mut memory = normal_route_fixture();
    memory.bytes[A1 + carmy::STRATEGIC_REDEPLOYMENT] = 1;
    array(
        &mut memory.bytes,
        C1 + ccountry::MODIFIER_ACCUMULATOR,
        0x1c000,
        2,
    );
    p32(&mut memory.bytes, 0x1c010, 0x1ea);
    p64(&mut memory.bytes, 0x1c018, 1);
    let mut context = super::NormalMovementContext::default();
    let direct = super::effective_movement_speed(&memory, 0, GS, A1, P1, 0x18200);
    assert!(direct.is_ok());
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| unreachable!()
        ),
        direct
    );
    let rail = context
        .rail
        .expect("strategic branch must initialize selector")
        .expect("strategic controls must be available");
    assert!(rail.blocked());
    assert!(rail.state.is_none());
    assert!(rail.friendly_state.is_none());
}

#[test]
fn branch_context_transport_fault_does_not_initialize_later_or_land_stages() {
    let mut memory = normal_route_fixture();
    p64(&mut memory.bytes, A1 + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
    p64(&mut memory.bytes, P1 + cprovince::STRATEGIC_REGION, 0x21000);
    p64(&mut memory.bytes, A1 + crate::cunit::LOCATION, u64::MAX);
    let mut context = super::NormalMovementContext::default();
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |owner, region| {
                assert_eq!((owner, region), (3, 0x21000));
                Err("transport source fault")
            }
        ),
        Err("transport source fault")
    );
    assert_eq!(context.transport_owner, Some(Ok(3)));
    assert!(context.transport_scale.is_none());
    assert!(context.land.is_none());
    assert!(context.roots.is_none());
    assert!(context.normal.is_none());
    // Even membership-selected transport cannot skip edge authentication
    // on the next request merely because army-level keys are cached.
    p64(&mut memory.bytes, 0x18200, 0);
    assert!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| panic!("bad destination must fail before transport callback")
        )
        .is_err()
    );
    assert_eq!(context.transport_owner, Some(Ok(3)));
}

#[test]
fn normal_context_is_lazy_and_keeps_mixed_transport_edges_independent() {
    let mut memory = normal_route_fixture();
    let mut context = super::NormalMovementContext::default();
    // A transport edge cannot depend on an unreadable normal stat object.
    p64(&mut memory.bytes, A1 + crate::cunit::STAT_BLOCK, u64::MAX);
    memory.bytes[0x18600 + crate::province_definition::TERRAIN_FLAGS] = 0;
    p64(&mut memory.bytes, P1 + cprovince::STRATEGIC_REGION, 0x21000);
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |owner, region| {
                assert_eq!((owner, region), (3, 0x21000));
                Ok(200_007)
            }
        ),
        Ok(66_668)
    );
    assert!(context.normal.is_none());
    memory.bytes[0x18600 + crate::province_definition::TERRAIN_FLAGS] = 1;
    p64(&mut memory.bytes, P1 + cprovince::STRATEGIC_REGION, 0);
    assert!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| panic!("normal edge selected transport")
        )
        .is_err()
    );
    assert!(matches!(context.normal, Some(Err(_))));
    p64(&mut memory.bytes, A1 + crate::cunit::STAT_BLOCK, 0x11000);
    assert!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| unreachable!(),
        )
        .is_err()
    );
    // A new pass may acquire the restored getter; this pass retains failure.
    context = super::NormalMovementContext::default();
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            P1,
            0x18200,
            &mut context,
            |_, _| unreachable!()
        ),
        Ok(362_522)
    );
    memory.bytes[0x18700 + crate::province_definition::TERRAIN_FLAGS] = 0;
    p64(
        &mut memory.bytes,
        0x18200 + cprovince::STRATEGIC_REGION,
        0x22000,
    );
    assert_eq!(
        super::effective_movement_speed_in_pass(
            &memory,
            0,
            GS,
            A1,
            0x18200,
            0x18400,
            &mut context,
            |owner, region| {
                assert_eq!((owner, region), (3, 0x22000));
                Ok(300_009)
            }
        ),
        Ok(100_001)
    );
    assert!(context.normal.is_some());
}

#[test]
fn native_speed_without_destination_preserves_fuel_scale_and_minimum() {
    let mut memory = presentation_fixture();
    memory.bytes[0x12000 + crate::province_definition::TERRAIN_FLAGS] = 1;
    p64(&mut memory.bytes, 0x11000 + 0x290, 400_000);
    p64(&mut memory.bytes, 0x11000 + 0x120, 100_000);
    p64(&mut memory.bytes, A1 + carmy::FUEL_CANDIDATE, 50_000);
    for (rva, value) in [
        (0x0331_cec0, 50_000_i64),
        (0x0331_97d0, 100_000),
        (0x0331_9f90, 25_000),
        (0x0331_d688, 100_000),
    ] {
        memory.globals.insert(rva, value.to_le_bytes().to_vec());
    }
    assert_eq!(
        super::effective_movement_speed(&memory, 0, GS, A1, P1, 0),
        Ok(250_000)
    );
    assert_eq!(
        super::effective_movement_speed_with_transport(&memory, 0, GS, A1, P1, 0, |_, _| panic!(
            "normal branch must not consult transport cache"
        )),
        Ok(250_000)
    );
    p64(&mut memory.bytes, 0x11000 + 0x290, 0);
    assert_eq!(
        super::effective_movement_speed(&memory, 0, GS, A1, P1, 0),
        Ok(100_000)
    );
    memory
        .globals
        .insert(0x0331_d688, 25_001_i64.to_le_bytes().to_vec());
    assert_eq!(
        super::effective_movement_speed(&memory, 0, GS, A1, P1, 0),
        Ok(25_001)
    );
    p64(&mut memory.bytes, 0x11000, 0);
    assert!(super::effective_movement_speed(&memory, 0, GS, A1, P1, 0).is_err());
}

#[test]
fn category_contribution_keeps_cold_order_duplicates_failures_and_capture_bounds() {
    const SELECTED: usize = 0x1a000;
    const CATEGORIES: usize = 0x25000;
    const CATEGORY: usize = 0x25200;
    let mut source = normal_route_fixture();
    source.bytes[SELECTED + 0x5a8] = 4;
    p32(&mut source.bytes, SELECTED + 0x5d8, 0x2b);
    array(&mut source.bytes, SELECTED + 0x508, CATEGORIES, 2);
    p64(&mut source.bytes, CATEGORIES, CATEGORY as u64);
    p64(&mut source.bytes, CATEGORIES + 8, CATEGORY as u64);
    p32(&mut source.bytes, CATEGORY + 0x60, 0x2b);
    let accumulator = C1 + ccountry::MODIFIER_ACCUMULATOR;
    let mut packet = super::EtaInputPacket::new(1, 0, 0).unwrap();
    {
        let observed = ScalarBatchMemory::new(&source);
        fn read(
            memory: &ScalarBatchMemory<'_>,
            memo: Option<&std::cell::RefCell<super::MovementCategoryContributions>>,
        ) -> Result<i64, &'static str> {
            let accumulator = C1 + ccountry::MODIFIER_ACCUMULATOR;
            super::movement_category_contribution(
                memory,
                0,
                SELECTED,
                accumulator,
                accumulator,
                1,
                memo,
            )
        }
        let modifier_reads = [
            (accumulator, 16),
            (0x1c000, 16),
            (crate::modifier_definition::TABLE_RVA, 8),
            (
                0x1d000 + 0x2b * crate::modifier_definition::STRIDE + 0x68,
                4,
            ),
            (
                0x1d000 + 0x2b * crate::modifier_definition::STRIDE + 0x60,
                4,
            ),
        ];
        let mut expected_reads = vec![(SELECTED + 0x5a8, 1)];
        // Leader and country alias one accumulator, but their absent 0x2c
        // queries remain separate and precede the selected-token read.
        for _ in 0..2 {
            expected_reads.extend([(accumulator, 16), (0x1c000, 16)]);
        }
        expected_reads.push((SELECTED + 0x5d8, 4));
        expected_reads.extend(modifier_reads);
        expected_reads.push((SELECTED + 0x508, 16));
        for slot in [CATEGORIES, CATEGORIES + 8] {
            expected_reads.extend([(slot, 8), (CATEGORY + 0x60, 4)]);
            expected_reads.extend(modifier_reads);
        }
        let allocated = packet.allocated_bytes();
        assert_eq!(
            read(&observed, Some(&packet.category_contributions)),
            Ok(30_003)
        );
        assert_eq!(*observed.reads.borrow(), expected_reads);
        observed.reads.borrow_mut().clear();
        assert_eq!(
            read(&observed, Some(&packet.category_contributions)),
            Ok(30_003)
        );
        assert!(observed.reads.borrow().is_empty());
        assert_eq!(packet.allocated_bytes(), allocated);
        // Every cold failure stays at its original interleaved position;
        // repeated category pointers/tokens must not hide it or be merged.
        for &(fault, _) in &expected_reads {
            packet.clear();
            observed.fault.set(Some(fault));
            observed.reads.borrow_mut().clear();
            let expected = read(&observed, None);
            assert!(expected.is_err());
            let prefix = observed.reads.borrow().clone();
            for _ in 0..2 {
                observed.reads.borrow_mut().clear();
                assert_eq!(
                    read(&observed, Some(&packet.category_contributions)),
                    expected
                );
                assert_eq!(*observed.reads.borrow(), prefix);
                let memo = packet.category_contributions.borrow();
                assert!(
                    memo.entries
                        .iter()
                        .all(|entry| entry.generation != memo.generation)
                );
            }
        }
        observed.fault.set(None);
        let mut bounded = super::EtaInputPacket::new(0, 0, 0).unwrap();
        for (rows, slots) in [(0, 0), (1, 1), (3, 4), (5, 8), (8, 8)] {
            bounded.reserve(rows, 0, 0).unwrap();
            bounded.clear();
            assert_eq!(bounded.category_contributions.borrow().entries.len(), slots);
            let allocated = bounded.allocated_bytes();
            for attempt in 0..2 {
                observed.reads.borrow_mut().clear();
                assert_eq!(
                    read(&observed, Some(&bounded.category_contributions)),
                    Ok(30_003)
                );
                if rows == 0 || attempt == 0 {
                    assert_eq!(*observed.reads.borrow(), expected_reads);
                } else {
                    assert!(observed.reads.borrow().is_empty());
                }
                assert_eq!(bounded.allocated_bytes(), allocated);
            }
        }
    }
    // A new capture must reacquire changed values and keep wrapping adds.
    p64(&mut source.bytes, 0x1c008, i64::MAX as u64);
    packet.clear();
    let observed = ScalarBatchMemory::new(&source);
    assert_eq!(
        super::movement_category_contribution(
            &observed,
            0,
            SELECTED,
            accumulator,
            accumulator,
            1,
            Some(&packet.category_contributions),
        ),
        Ok(i64::MAX - 2)
    );
    assert!(observed.reads.borrow().contains(&(accumulator, 16)));
    assert!(
        observed
            .reads
            .borrow()
            .contains(&(crate::modifier_definition::TABLE_RVA, 8))
    );
}

#[test]
fn category_direct_map_collisions_keep_full_keys_successes_and_faults() {
    const SELECTED: usize = 0x1a000;
    const OTHER_SELECTED: usize = 0x25000;
    const OTHER_COUNTRY: usize = 0x26000;
    const OTHER_LEADER: usize = 0x26200;
    const OTHER_BASE: usize = 0x100000;
    let mut source = normal_route_fixture();
    for selected in [SELECTED, OTHER_SELECTED] {
        source.bytes[selected + 0x5a8] = 4;
        p32(&mut source.bytes, selected + 0x5d8, 0x2b);
    }
    array(&mut source.bytes, OTHER_COUNTRY, 0x26100, 1);
    p32(&mut source.bytes, 0x26100, 0x2b);
    p64(&mut source.bytes, 0x26108, 20_003);
    array(&mut source.bytes, OTHER_LEADER, 0x26300, 1);
    p32(&mut source.bytes, 0x26300, 0x2c);
    p64(&mut source.bytes, 0x26308, 123);
    source.globals.insert(
        OTHER_BASE + crate::modifier_definition::TABLE_RVA,
        0x1d000_u64.to_le_bytes().to_vec(),
    );
    let accumulator = C1 + ccountry::MODIFIER_ACCUMULATOR;
    let original = super::MovementCategoryKey {
        base: 0,
        selected: SELECTED,
        country_accumulator: accumulator,
        leader_accumulator: accumulator,
        leader: 1,
    };
    let variants = [
        super::MovementCategoryKey {
            base: OTHER_BASE,
            ..original
        },
        super::MovementCategoryKey {
            selected: OTHER_SELECTED,
            ..original
        },
        super::MovementCategoryKey {
            country_accumulator: OTHER_COUNTRY,
            ..original
        },
        super::MovementCategoryKey {
            leader_accumulator: OTHER_LEADER,
            ..original
        },
        super::MovementCategoryKey {
            leader: 0,
            ..original
        },
    ];
    let observed = ScalarBatchMemory::new(&source);
    let read =
        |key: super::MovementCategoryKey,
         memo: Option<&std::cell::RefCell<super::MovementCategoryContributions>>| {
            super::movement_category_contribution(
                &observed,
                key.base,
                key.selected,
                key.country_accumulator,
                key.leader_accumulator,
                key.leader,
                memo,
            )
        };
    assert_eq!(read(original, None), Ok(10_001));
    let original_reads = observed.reads.borrow().clone();
    let mut packet = super::EtaInputPacket::new(1, 0, 0).unwrap();
    // One slot forces collisions independently of the index mixer.
    assert_eq!(packet.category_contributions.borrow().entries.len(), 1);
    let allocated = packet.allocated_bytes();
    for (variant, expected) in variants
        .into_iter()
        .zip([10_001, 10_001, 20_003, 10_124, 10_001])
    {
        observed.reads.borrow_mut().clear();
        assert_eq!(read(variant, None), Ok(expected));
        let variant_reads = observed.reads.borrow().clone();
        assert!(!variant_reads.is_empty());
        for &(fault, _) in &variant_reads {
            packet.clear();
            observed.fault.set(None);
            assert_eq!(
                read(original, Some(&packet.category_contributions)),
                Ok(10_001)
            );
            observed.fault.set(Some(fault));
            observed.reads.borrow_mut().clear();
            let failure = read(variant, None);
            assert!(failure.is_err());
            let prefix = observed.reads.borrow().clone();
            for _ in 0..2 {
                observed.reads.borrow_mut().clear();
                assert_eq!(read(variant, Some(&packet.category_contributions)), failure);
                assert_eq!(*observed.reads.borrow(), prefix);
                observed.reads.borrow_mut().clear();
                assert_eq!(
                    read(original, Some(&packet.category_contributions)),
                    Ok(10_001)
                );
                assert!(observed.reads.borrow().is_empty());
            }
        }
        observed.fault.set(None);
        observed.reads.borrow_mut().clear();
        assert_eq!(
            read(variant, Some(&packet.category_contributions)),
            Ok(expected)
        );
        assert_eq!(*observed.reads.borrow(), variant_reads);
        observed.reads.borrow_mut().clear();
        assert_eq!(
            read(variant, Some(&packet.category_contributions)),
            Ok(expected)
        );
        assert!(observed.reads.borrow().is_empty());
        observed.reads.borrow_mut().clear();
        assert_eq!(
            read(original, Some(&packet.category_contributions)),
            Ok(10_001)
        );
        assert_eq!(*observed.reads.borrow(), original_reads);
        assert_eq!(packet.allocated_bytes(), allocated);
    }
    // Changing the slot mask must invalidate entries made under the old mask.
    packet.reserve(3, 0, 0).unwrap();
    assert_eq!(packet.category_contributions.borrow().entries.len(), 4);
    {
        let memo = packet.category_contributions.borrow();
        assert!(
            memo.entries
                .iter()
                .all(|entry| entry.generation != memo.generation)
        );
    }
    let allocated = packet.allocated_bytes();
    observed.reads.borrow_mut().clear();
    assert_eq!(
        read(original, Some(&packet.category_contributions)),
        Ok(10_001)
    );
    assert_eq!(*observed.reads.borrow(), original_reads);
    observed.reads.borrow_mut().clear();
    assert_eq!(
        read(original, Some(&packet.category_contributions)),
        Ok(10_001)
    );
    assert!(observed.reads.borrow().is_empty());
    assert_eq!(packet.allocated_bytes(), allocated);
}

#[test]
fn category_generations_reacquire_after_clear_wrap_and_growth() {
    const SELECTED: usize = 0x1a000;
    let mut source = normal_route_fixture();
    p32(&mut source.bytes, SELECTED + 0x5d8, 0x2b);
    let accumulator = C1 + ccountry::MODIFIER_ACCUMULATOR;
    let key = super::MovementCategoryKey {
        base: 0,
        selected: SELECTED,
        country_accumulator: accumulator,
        leader_accumulator: accumulator,
        leader: 1,
    };
    let mut packet = super::EtaInputPacket::new(2, 0, 0).unwrap();
    let read = |source: &PresentationMemory, packet: &super::EtaInputPacket, expected| {
        let observed = ScalarBatchMemory::new(source);
        let acquire = || {
            super::movement_category_contribution(
                &observed,
                key.base,
                key.selected,
                key.country_accumulator,
                key.leader_accumulator,
                key.leader,
                Some(&packet.category_contributions),
            )
        };
        let allocated = packet.allocated_bytes();
        assert_eq!(acquire(), Ok(expected));
        assert!(!observed.reads.borrow().is_empty());
        observed.reads.borrow_mut().clear();
        assert_eq!(acquire(), Ok(expected));
        assert!(observed.reads.borrow().is_empty());
        assert_eq!(packet.allocated_bytes(), allocated);
    };
    read(&source, &packet, 10_001);
    let previous = packet.category_contributions.borrow().generation;
    let tags: Vec<_> = packet
        .category_contributions
        .borrow()
        .entries
        .iter()
        .map(|entry| entry.generation)
        .collect();
    packet.clear();
    {
        let memo = packet.category_contributions.borrow();
        assert_eq!(memo.generation, previous + 1);
        // Ordinary reset does not visit or rewrite the slot array.
        assert!(memo.entries.iter().map(|entry| entry.generation).eq(tags));
    }
    p64(&mut source.bytes, 0x1c008, 20_003);
    read(&source, &packet, 20_003);
    for grow in [false, true] {
        {
            let memo = packet.category_contributions.get_mut();
            let slot = memo.slot(key).unwrap();
            memo.generation = u64::MAX;
            for (index, entry) in memo.entries.iter_mut().enumerate() {
                // The old generation-one entry would resurrect on wrap
                // unless every slot tag is invalidated first.
                *entry = super::MovementCategoryContribution {
                    key,
                    value: 77_777,
                    generation: if index == slot { 1 } else { u64::MAX },
                };
            }
        }
        if grow {
            packet.reserve(3, 0, 0).unwrap();
        } else {
            packet.clear();
        }
        {
            let memo = packet.category_contributions.borrow();
            assert_eq!(memo.generation, 1);
            assert_eq!(memo.entries.len(), if grow { 4 } else { 2 });
            assert!(memo.entries.iter().all(|entry| entry.generation == 0));
        }
        p64(
            &mut source.bytes,
            0x1c008,
            if grow { 40_009 } else { 30_007 },
        );
        read(&source, &packet, if grow { 40_009 } else { 30_007 });
    }
}

const AIR_TEST_BASE: usize = 0x400000;
const AIR_TEST_DIPLOMACY: usize = 0x22000;
const AIR_TEST_RELATIONS: usize = 0x22200;
const AIR_TEST_FIRST: usize = 0x22400;
const AIR_TEST_MATCH: usize = 0x22500;
const AIR_TEST_REGION: usize = 0x23000;
const AIR_TEST_OTHER_REGION: usize = 0x23200;
const AIR_TEST_MANAGER: usize = 0x24000;
const AIR_TEST_COUNTRIES: usize = 0x24200;
const AIR_TEST_COUNTRY: usize = 0x24400;
const AIR_TEST_ENTRIES: usize = 0x24800;
const AIR_TEST_STATS: usize = 0x11000;
const AIR_TEST_OTHER_STATS: usize = 0x29000;
const AIR_TEST_SCALE: usize = 0x0331_ddd8;

fn air_prefix_fixture() -> PresentationMemory {
    let mut source = normal_route_fixture();
    let bytes = &mut source.bytes;
    // Nonzero defense reads module-relative defines through a reader
    // that rejects null bases, so authenticate these objects at one.
    for owner in [C1, C2] {
        p64(bytes, owner, (AIR_TEST_BASE + ccountry::VTABLE_RVA) as u64);
        p64(bytes, owner + 0xf88, AIR_TEST_DIPLOMACY as u64);
    }
    p32(bytes, AIR_TEST_DIPLOMACY + 0xa4, 0);
    array(bytes, AIR_TEST_DIPLOMACY + 0x20, AIR_TEST_RELATIONS, 3);
    p64(bytes, AIR_TEST_RELATIONS, AIR_TEST_FIRST as u64);
    p64(bytes, AIR_TEST_RELATIONS + 8, AIR_TEST_MATCH as u64);
    // The first matching relation must stop before this invalid pointer.
    p64(bytes, AIR_TEST_RELATIONS + 16, u64::MAX);
    p32(bytes, AIR_TEST_FIRST + 8, 0x3809);
    p32(bytes, AIR_TEST_MATCH + 8, 0x380a);
    p64(
        bytes,
        P1 + cprovince::STRATEGIC_REGION,
        AIR_TEST_REGION as u64,
    );
    for region in [AIR_TEST_REGION, AIR_TEST_OTHER_REGION] {
        p32(bytes, region + 0x60, 0);
    }
    p64(
        bytes,
        GS + crate::cgame_state::STRATEGIC_AIR_MANAGER,
        AIR_TEST_MANAGER as u64,
    );
    array(bytes, AIR_TEST_MANAGER + 0x30, AIR_TEST_COUNTRIES, 4);
    for canonical in [1, 2] {
        p64(
            bytes,
            AIR_TEST_COUNTRIES + canonical * 8,
            AIR_TEST_COUNTRY as u64,
        );
    }
    array(bytes, AIR_TEST_COUNTRY + 0xe0, AIR_TEST_ENTRIES, 2);
    p64(bytes, AIR_TEST_ENTRIES, 300_000);
    p64(bytes, AIR_TEST_ENTRIES + 8, 100_000);
    p64(bytes, AIR_TEST_ENTRIES + 0x18, -30_001_i64 as u64);
    p64(bytes, AIR_TEST_ENTRIES + 0xa0, 100_000);
    p64(bytes, AIR_TEST_ENTRIES + 0xa8, 300_000);
    p64(bytes, AIR_TEST_ENTRIES + 0xb8, 40_003);
    for (stats, defense) in [(AIR_TEST_STATS, 100_000), (AIR_TEST_OTHER_STATS, 200_000)] {
        p64(
            bytes,
            stats,
            (AIR_TEST_BASE + crate::csub_unit_definition::VTABLE_RVA) as u64,
        );
        p64(
            bytes,
            stats + crate::csub_unit_definition::MOVEMENT_AIR_DEFENSE,
            defense,
        );
    }
    for (rva, value) in [
        (AIR_TEST_SCALE, 100_007_i64),
        (
            crate::csub_unit_definition::AIR_DEFENSE_DENOMINATOR_RVA,
            100_000,
        ),
        (
            crate::csub_unit_definition::AIR_DEFENSE_REDUCTION_RVA,
            50_000,
        ),
    ] {
        source
            .globals
            .insert(AIR_TEST_BASE + rva, value.to_le_bytes().to_vec());
    }
    source
}

fn air_prefix_key() -> super::MovementAirKey {
    super::MovementAirKey {
        base: AIR_TEST_BASE,
        state: GS,
        owner_raw: 3,
        owner_pointer: C1,
        canonical: 1,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AirPrefixTestValue {
    Active(bool),
    Region(Option<[i64; 3]>),
}

fn air_prefix_test_read(
    memory: &impl crate::Memory,
    key: super::MovementAirKey,
    region: Option<usize>,
    memo: Option<&std::cell::RefCell<super::MovementAirMemo>>,
) -> Result<AirPrefixTestValue, &'static str> {
    match region {
        None => super::movement_air_active(memory, key, memo).map(AirPrefixTestValue::Active),
        Some(region) => super::movement_air_region(memory, key, region, memo).map(|prefix| {
            AirPrefixTestValue::Region(prefix.map(|p| [p.fraction, p.impact, p.scale]))
        }),
    }
}

fn air_prefix_active_trace() -> Vec<(usize, usize)> {
    vec![
        (C1 + 0xf88, 8),
        (AIR_TEST_DIPLOMACY + 0xa4, 4),
        (AIR_TEST_DIPLOMACY + 0x20, 16),
        (AIR_TEST_RELATIONS, 8),
        (AIR_TEST_FIRST + 8, 4),
        (AIR_TEST_RELATIONS + 8, 8),
        (AIR_TEST_MATCH + 8, 4),
    ]
}

fn air_prefix_region_trace() -> Vec<(usize, usize)> {
    vec![
        (GS + crate::cgame_state::STRATEGIC_AIR_MANAGER, 8),
        (AIR_TEST_MANAGER + 0x30, 16),
        (AIR_TEST_COUNTRIES + 8, 8),
        (AIR_TEST_REGION + 0x60, 4),
        (AIR_TEST_COUNTRY + 0xe0, 16),
        (AIR_TEST_ENTRIES, 8),
        (AIR_TEST_ENTRIES + 8, 8),
        (AIR_TEST_ENTRIES + 0x18, 8),
        (AIR_TEST_BASE + AIR_TEST_SCALE, 8),
    ]
}

#[test]
fn air_prefix_cold_queries_preserve_exact_reads_and_every_failed_prefix() {
    let source = air_prefix_fixture();
    for (region, expected, trace) in [
        (
            None,
            AirPrefixTestValue::Active(true),
            air_prefix_active_trace(),
        ),
        (
            Some(AIR_TEST_REGION),
            AirPrefixTestValue::Region(Some([25_000, -30_001, 100_007])),
            air_prefix_region_trace(),
        ),
    ] {
        let observed = ScalarBatchMemory::new(&source);
        assert_eq!(
            air_prefix_test_read(&observed, air_prefix_key(), region, None),
            Ok(expected)
        );
        assert_eq!(*observed.reads.borrow(), trace);
        for capacity in [0, 1, 4] {
            let memo = std::cell::RefCell::new(super::MovementAirMemo::new(capacity).unwrap());
            let allocated = memo.borrow().allocated_bytes();
            for attempt in 0..2 {
                observed.reads.borrow_mut().clear();
                let expected = air_prefix_test_read(&source, air_prefix_key(), region, None);
                assert_eq!(
                    air_prefix_test_read(&observed, air_prefix_key(), region, Some(&memo)),
                    expected,
                );
                if capacity == 0 || attempt == 0 {
                    assert_eq!(*observed.reads.borrow(), trace);
                } else {
                    assert!(observed.reads.borrow().is_empty());
                }
                assert_eq!(memo.borrow().allocated_bytes(), allocated);
            }
            for &(fault, _) in &trace {
                memo.borrow_mut().clear();
                observed.fault.set(Some(fault));
                observed.reads.borrow_mut().clear();
                let expected = air_prefix_test_read(&observed, air_prefix_key(), region, None);
                assert!(expected.is_err(), "fault {fault:#x}");
                let prefix = observed.reads.take();
                let ordinal = trace.iter().position(|&(at, _)| at == fault).unwrap();
                assert_eq!(prefix, trace[..=ordinal]);
                for _ in 0..2 {
                    assert_eq!(
                        air_prefix_test_read(&observed, air_prefix_key(), region, Some(&memo)),
                        expected,
                    );
                    assert_eq!(observed.reads.take(), prefix);
                    assert_eq!(memo.borrow().allocated_bytes(), allocated);
                }
            }
            observed.fault.set(None);
        }
    }
}

#[test]
fn air_prefix_direct_map_collisions_keep_every_identity_and_prior_success() {
    const OTHER_BASE: usize = 0x100000;
    const OTHER_STATE: usize = 0x200000;
    let mut source = air_prefix_fixture();
    source.globals.insert(
        OTHER_BASE + AIR_TEST_SCALE,
        200_009_i64.to_le_bytes().to_vec(),
    );
    source.globals.insert(
        OTHER_STATE + crate::cgame_state::STRATEGIC_AIR_MANAGER,
        (AIR_TEST_MANAGER as u64).to_le_bytes().to_vec(),
    );
    let original = air_prefix_key();
    let variants = [
        super::MovementAirKey {
            base: OTHER_BASE,
            ..original
        },
        super::MovementAirKey {
            state: OTHER_STATE,
            ..original
        },
        super::MovementAirKey {
            owner_raw: 1,
            ..original
        },
        super::MovementAirKey {
            owner_pointer: C2,
            ..original
        },
        super::MovementAirKey {
            canonical: 2,
            ..original
        },
    ];
    // Identical results from aliases still require different cold queries.
    // One slot makes all distinct full keys collide regardless of hashing.
    for region in [None, Some(AIR_TEST_REGION)] {
        for (variant, variant_region) in variants.into_iter().map(|key| (key, region)).chain(
            region
                .into_iter()
                .map(|_| (original, Some(AIR_TEST_OTHER_REGION))),
        ) {
            let observed = ScalarBatchMemory::new(&source);
            let memo = std::cell::RefCell::new(super::MovementAirMemo::new(1).unwrap());
            let allocated = memo.borrow().allocated_bytes();
            let original_value = air_prefix_test_read(&observed, original, region, None);
            let original_trace = observed.reads.take();
            let variant_value = air_prefix_test_read(&observed, variant, variant_region, None);
            let variant_trace = observed.reads.take();
            assert!(original_value.is_ok() && variant_value.is_ok());
            for &(fault, _) in &variant_trace {
                memo.borrow_mut().clear();
                observed.fault.set(None);
                assert_eq!(
                    air_prefix_test_read(&observed, original, region, Some(&memo)),
                    original_value
                );
                observed.reads.borrow_mut().clear();
                observed.fault.set(Some(fault));
                let failure = air_prefix_test_read(&observed, variant, variant_region, None);
                assert!(failure.is_err());
                let prefix = observed.reads.take();
                for _ in 0..2 {
                    assert_eq!(
                        air_prefix_test_read(&observed, variant, variant_region, Some(&memo)),
                        failure
                    );
                    assert_eq!(observed.reads.take(), prefix);
                    assert_eq!(
                        air_prefix_test_read(&observed, original, region, Some(&memo)),
                        original_value
                    );
                    assert!(observed.reads.borrow().is_empty());
                }
            }
            observed.fault.set(None);
            observed.reads.borrow_mut().clear();
            assert_eq!(
                air_prefix_test_read(&observed, variant, variant_region, Some(&memo)),
                variant_value
            );
            assert_eq!(observed.reads.take(), variant_trace);
            assert_eq!(
                air_prefix_test_read(&observed, variant, variant_region, Some(&memo)),
                variant_value
            );
            assert!(observed.reads.borrow().is_empty());
            assert_eq!(
                air_prefix_test_read(&observed, original, region, Some(&memo)),
                original_value
            );
            assert_eq!(observed.reads.take(), original_trace);
            assert_eq!(memo.borrow().allocated_bytes(), allocated);
        }
    }
}

#[test]
fn air_prefix_clear_and_growth_reacquire_changed_inputs_without_query_allocation() {
    let mut source = air_prefix_fixture();
    let memo = std::cell::RefCell::new(super::MovementAirMemo::new(0).unwrap());
    assert_eq!(memo.borrow().allocated_bytes(), 0);
    for rows in [0, 1, 3, 5] {
        memo.borrow_mut().reserve(rows).unwrap();
        for (active, hostile) in [(false, 0_u64), (true, 300_000)] {
            memo.borrow_mut().clear();
            p32(
                &mut source.bytes,
                AIR_TEST_DIPLOMACY + 0xa4,
                u32::from(active),
            );
            array(&mut source.bytes, AIR_TEST_DIPLOMACY + 0x20, 0, 0);
            p64(&mut source.bytes, AIR_TEST_ENTRIES + 8, hostile);
            let observed = ScalarBatchMemory::new(&source);
            let allocated = memo.borrow().allocated_bytes();
            for region in [None, Some(AIR_TEST_REGION)] {
                let expected = air_prefix_test_read(&source, air_prefix_key(), region, None);
                for attempt in 0..2 {
                    assert_eq!(
                        air_prefix_test_read(&observed, air_prefix_key(), region, Some(&memo)),
                        expected
                    );
                    let reads = observed.reads.take();
                    assert_eq!(reads.is_empty(), rows != 0 && attempt == 1);
                    assert_eq!(memo.borrow().allocated_bytes(), allocated);
                }
            }
        }
    }
    // Growing storage is itself an invalidation boundary, independent of
    // clear; the new hash mask must never turn an old tag into a hit.
    p64(&mut source.bytes, AIR_TEST_ENTRIES + 8, 900_000);
    memo.borrow_mut().reserve(17).unwrap();
    let observed = ScalarBatchMemory::new(&source);
    assert_eq!(
        air_prefix_test_read(
            &observed,
            air_prefix_key(),
            Some(AIR_TEST_REGION),
            Some(&memo)
        ),
        Ok(AirPrefixTestValue::Region(Some([75_000, -30_001, 100_007]))),
    );
    assert_eq!(observed.reads.take(), air_prefix_region_trace());
}

type AirPrefixFullRead = (Result<[i64; 4], &'static str>, Vec<(usize, usize)>);

fn air_prefix_full_read(
    source: &PresentationMemory,
    fault: Option<usize>,
    memo: Option<&std::cell::RefCell<super::MovementAirMemo>>,
    stats: usize,
) -> AirPrefixFullRead {
    let seed = speed_seed(source);
    let observed = ScalarBatchMemory::new(source);
    let mut geometry = SeededSpeedGeometry::new(&observed);
    observed.reads.borrow_mut().clear();
    observed.fault.set(fault);
    let value = super::movement_air_inputs(
        &observed,
        AIR_TEST_BASE,
        GS,
        A1,
        stats,
        P1,
        Some(&mut super::MovementSharedInputs {
            seed: &seed,
            route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
            geometry: &mut geometry,
            rail_speeds: None,
            state_modifiers: None,
            category_contributions: None,
            supply_templates: None,
            air_prefixes: memo,
        }),
    )
    .map(|air| [air.fraction, air.impact, air.scale, air.defense]);
    (value, observed.reads.take())
}

#[test]
fn air_prefix_warm_hits_keep_owner_region_and_division_defense_reads_fresh() {
    let source = air_prefix_fixture();
    let memo = std::cell::RefCell::new(super::MovementAirMemo::new(4).unwrap());
    let (direct, direct_trace) = air_prefix_full_read(&source, None, None, AIR_TEST_STATS);
    assert!(direct.is_ok(), "{direct:?}");
    let mut expected_warm = direct_trace.clone();
    for prefix in [air_prefix_active_trace(), air_prefix_region_trace()] {
        let start = expected_warm
            .windows(prefix.len())
            .position(|window| window == prefix.as_slice())
            .expect("direct read must contain the complete memoized prefix");
        expected_warm.drain(start..start + prefix.len());
    }
    assert_eq!(
        air_prefix_full_read(&source, None, Some(&memo), AIR_TEST_STATS),
        (direct, direct_trace)
    );
    let (warm, warm_trace) = air_prefix_full_read(&source, None, Some(&memo), AIR_TEST_STATS);
    assert_eq!(warm, direct);
    // Keep every remaining occurrence and its order, including repeated
    // canonical-link validation in the authenticated country reader.
    assert_eq!(warm_trace, expected_warm);
    assert!(!warm_trace.contains(&(C1 + 0xf88, 8)));
    assert!(!warm_trace.contains(&(GS + crate::cgame_state::STRATEGIC_AIR_MANAGER, 8)));
    for (fault, len) in [
        (GS + crate::cgame_state::COUNTRIES, 16),
        (GS + crate::cgame_state::COUNTRY_LINKS, 16),
        (LINKS + 3 * 4, 4),
        (LINKS + 4, 4),
        (COUNTRIES + 8, 8),
        (C1, 8),
        (C1 + ccountry::ID, 4),
        (P1 + cprovince::STRATEGIC_REGION, 8),
        (AIR_TEST_STATS, 8),
        (
            AIR_TEST_STATS + crate::csub_unit_definition::MOVEMENT_AIR_DEFENSE,
            8,
        ),
        (
            AIR_TEST_BASE + crate::csub_unit_definition::AIR_DEFENSE_DENOMINATOR_RVA,
            8,
        ),
        (
            AIR_TEST_BASE + crate::csub_unit_definition::AIR_DEFENSE_REDUCTION_RVA,
            8,
        ),
    ] {
        assert!(warm_trace.contains(&(fault, len)), "fresh read {fault:#x}");
        let (expected, _) = air_prefix_full_read(&source, Some(fault), None, AIR_TEST_STATS);
        let (actual, trace) =
            air_prefix_full_read(&source, Some(fault), Some(&memo), AIR_TEST_STATS);
        assert!(expected.is_err(), "fault {fault:#x}");
        assert_eq!(actual, expected, "fault {fault:#x}");
        let ordinal = warm_trace
            .iter()
            .position(|&read| read == (fault, len))
            .unwrap();
        assert_eq!(trace, warm_trace[..=ordinal], "fault {fault:#x}");
    }
    let (other, trace) = air_prefix_full_read(&source, None, Some(&memo), AIR_TEST_OTHER_STATS);
    assert_eq!(
        other,
        air_prefix_full_read(&source, None, None, AIR_TEST_OTHER_STATS).0
    );
    assert_ne!(other.unwrap()[3], direct.unwrap()[3]);
    assert!(trace.contains(&(AIR_TEST_OTHER_STATS, 8)));
    assert!(trace.contains(&(
        AIR_TEST_OTHER_STATS + crate::csub_unit_definition::MOVEMENT_AIR_DEFENSE,
        8
    )));
    assert!(!trace.contains(&(AIR_TEST_ENTRIES, 8)));
}

#[test]
fn air_prefix_default_branches_keep_location_reads_and_skip_defense() {
    for branch in 0..3 {
        let mut source = air_prefix_fixture();
        match branch {
            0 => array(&mut source.bytes, AIR_TEST_DIPLOMACY + 0x20, 0, 0),
            1 => p64(&mut source.bytes, P1 + cprovince::STRATEGIC_REGION, 0),
            _ => p64(&mut source.bytes, AIR_TEST_ENTRIES + 8, 0),
        }
        let memo = std::cell::RefCell::new(super::MovementAirMemo::new(4).unwrap());
        for _ in 0..2 {
            let (actual, trace) =
                air_prefix_full_read(&source, Some(AIR_TEST_STATS), Some(&memo), AIR_TEST_STATS);
            assert_eq!(actual, Ok([0; 4]));
            assert!(trace.contains(&(P1 + cprovince::STRATEGIC_REGION, 8)));
            assert!(!trace.contains(&(AIR_TEST_STATS, 8)));
            if branch != 2 {
                assert!(!trace.contains(&(GS + crate::cgame_state::STRATEGIC_AIR_MANAGER, 8)));
            }
        }
        let fault = P1 + cprovince::STRATEGIC_REGION;
        let (expected, _) = air_prefix_full_read(&source, Some(fault), None, AIR_TEST_STATS);
        let (actual, trace) =
            air_prefix_full_read(&source, Some(fault), Some(&memo), AIR_TEST_STATS);
        assert!(expected.is_err());
        assert_eq!(actual, expected);
        assert_eq!(trace.last(), Some(&(fault, 8)));
    }
}

#[test]
fn category_hits_keep_prior_authentication_and_later_air_supply_failures() {
    use crate::{carmy_leader as leader, cunit_leader};
    const LEADER: usize = 0x23000;
    const SELECTED: usize = 0x1a000;
    let mut source = normal_route_fixture();
    p64(&mut source.bytes, A1 + carmy::ORDERS_GROUP, G1 as u64);
    p64(&mut source.bytes, G1 + corders_group::LEADER, LEADER as u64);
    p64(&mut source.bytes, LEADER, leader::VTABLE_RVA as u64);
    p32(
        &mut source.bytes,
        LEADER + cunit_leader::TYPE,
        cunit_leader::REFERENCE_TYPE,
    );
    array(
        &mut source.bytes,
        LEADER + leader::MOVEMENT_TERRAIN_DATA,
        0x24000,
        2,
    );
    p32(&mut source.bytes, SELECTED + 0x5d8, 0x2b);
    source.bytes[A1 + 0x2ae] = 0;
    source.bytes[A1 + 0x2e] = 1;
    source.globals.insert(
        crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
        0_u32.to_le_bytes().to_vec(),
    );
    source
        .globals
        .insert(0x0331_94e0, 100_000_i64.to_le_bytes().to_vec());
    source
        .globals
        .insert(0x0331_9394, 2_i32.to_le_bytes().to_vec());
    let seed = speed_seed(&source);
    let packet = super::EtaInputPacket::new(1, 0, 0).unwrap();
    let run = |fault: Option<usize>, cached: bool| {
        let observed = ScalarBatchMemory::new(&source);
        observed.fault.set(fault);
        let mut geometry = SeededSpeedGeometry::new(&observed);
        let value = super::movement_speed_inputs_seeded_in_pass(
            &observed,
            0,
            GS,
            &mut super::MovementSharedInputs {
                seed: &seed,
                route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[2, 3]),
                geometry: &mut geometry,
                rail_speeds: None,
                state_modifiers: None,
                category_contributions: cached.then_some(&packet.category_contributions),
                supply_templates: None,
                air_prefixes: None,
            },
            P1,
            0x18200,
            &mut super::NormalMovementContext::default(),
            |_, _| unreachable!(),
        );
        (value, observed.reads.into_inner())
    };
    let (cold, _) = run(None, true);
    assert!(cold.is_ok());
    {
        let memo = packet.category_contributions.borrow();
        assert_eq!(
            memo.entries
                .iter()
                .filter(|entry| entry.generation == memo.generation)
                .count(),
            1
        );
    }
    let (warm, reads) = run(None, true);
    assert_eq!(warm, cold);
    assert!(!reads.contains(&(SELECTED + 0x5a8, 1)));
    for (fault, before) in [
        (C1, true),
        (G1, true),
        (LEADER, true),
        (T1, true),
        (SELECTED, true),
        (C1 + ccountry::DIPLOMACY_STATUS, false),
        (A1 + 0x600, false),
    ] {
        let (direct, direct_reads) = run(Some(fault), false);
        let (cached, cached_reads) = run(Some(fault), true);
        assert!(direct.is_err(), "fault {fault:x}");
        assert_eq!(cached, direct, "fault {fault:x}");
        assert_eq!(cached_reads.last().unwrap().0, fault);
        if before {
            assert_eq!(cached_reads, direct_reads);
        } else {
            assert!(direct_reads.contains(&(SELECTED + 0x5a8, 1)));
            assert!(!cached_reads.contains(&(SELECTED + 0x5a8, 1)));
        }
        {
            let memo = packet.category_contributions.borrow();
            assert_eq!(
                memo.entries
                    .iter()
                    .filter(|entry| entry.generation == memo.generation)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn supply_template_reuse_keeps_ordered_aliases_faults_wrapping_and_capacity() {
    const TEMPLATE: usize = 0x17000;
    const ALIAS: usize = 0x17200;
    const UNITS: usize = 0x19000;
    let mut source = presentation_fixture();
    array(&mut source.bytes, TEMPLATE + 0xc0, UNITS, 3);
    array(&mut source.bytes, ALIAS + 0xc0, UNITS, 3);
    for (index, unit) in [0x1a000, 0x1b000, 0x1a000].into_iter().enumerate() {
        p64(&mut source.bytes, UNITS + index * 16, unit);
    }
    source.bytes[0x1a000 + 0x5b7] = 1;
    let expected_reads = [
        (TEMPLATE + 0xc0, 16),
        (UNITS, 8),
        (UNITS + 8, 4),
        (0x1a000 + 0x5b7, 1),
        (UNITS + 16, 8),
        (UNITS + 24, 4),
        (0x1b000 + 0x5b7, 1),
        (UNITS + 32, 8),
        (UNITS + 40, 4),
        (0x1a000 + 0x5b7, 1),
    ];
    let mut packet = super::EtaInputPacket::new(2, 0, 0).unwrap();
    let allocated = packet.allocated_bytes();
    for (counts, expected) in [
        ([10_i32, 30, 10], 40_000),
        ([i32::MAX, 0, i32::MAX], 100_000),
        ([i32::MIN, -1, i32::MIN], 0),
        ([1, -2, 1], 0),
        ([-3, 2, -3], 150_000),
    ] {
        for (index, count) in counts.into_iter().enumerate() {
            p32(&mut source.bytes, UNITS + index * 16 + 8, count as u32);
        }
        // A new capture must see changed counts, even at the same pointer.
        packet.clear();
        let observed = ScalarBatchMemory::new(&source);
        let memo = Some(&packet.supply_templates);
        assert_eq!(
            super::movement_supply_template_ratio(&observed, TEMPLATE, memo),
            Ok(expected)
        );
        assert_eq!(*observed.reads.borrow(), expected_reads);
        observed.reads.borrow_mut().clear();
        assert_eq!(
            super::movement_supply_template_ratio(&observed, TEMPLATE, memo),
            Ok(expected)
        );
        assert!(observed.reads.borrow().is_empty());
        // Equal array pointers do not merge independently named templates.
        assert_eq!(
            super::movement_supply_template_ratio(&observed, ALIAS, memo),
            Ok(expected)
        );
        let mut alias_reads = expected_reads;
        alias_reads[0].0 = ALIAS + 0xc0;
        assert_eq!(*observed.reads.borrow(), alias_reads);
        assert_eq!(packet.allocated_bytes(), allocated);
    }

    let observed = ScalarBatchMemory::new(&source);
    for (fault, _) in expected_reads {
        packet.clear();
        observed.fault.set(Some(fault));
        observed.reads.borrow_mut().clear();
        let expected = super::movement_supply_template_ratio(&observed, TEMPLATE, None);
        assert!(expected.is_err());
        let prefix = observed.reads.borrow().clone();
        for _ in 0..2 {
            observed.reads.borrow_mut().clear();
            assert_eq!(
                super::movement_supply_template_ratio(
                    &observed,
                    TEMPLATE,
                    Some(&packet.supply_templates),
                ),
                expected
            );
            assert_eq!(*observed.reads.borrow(), prefix);
            assert!(packet.supply_templates.borrow().is_empty());
        }
    }
    observed.fault.set(None);
    for rows in [0, 1] {
        let mut packet = super::EtaInputPacket::new(rows, 0, 0).unwrap();
        let templates = packet.supply_templates.get_mut();
        for key in 1..=templates.capacity() {
            templates.insert(key, 0);
        }
        let allocated = packet.allocated_bytes();
        observed.reads.borrow_mut().clear();
        assert_eq!(
            super::movement_supply_template_ratio(
                &observed,
                TEMPLATE,
                Some(&packet.supply_templates),
            ),
            Ok(150_000)
        );
        assert_eq!(*observed.reads.borrow(), expected_reads);
        assert_eq!(packet.allocated_bytes(), allocated);
        assert!(!packet.supply_templates.borrow().contains_key(&TEMPLATE));
    }
    let mut vacant = super::EtaInputPacket::new(0, 0, 0).unwrap();
    assert_eq!(vacant.supply_templates.borrow().capacity(), 0);
    vacant.reserve(4, 0, 0).unwrap();
    assert!(vacant.supply_templates.borrow().capacity() >= 4);
    let allocated = vacant.allocated_bytes();
    assert_eq!(
        super::movement_supply_template_ratio(&observed, TEMPLATE, Some(&vacant.supply_templates),),
        Ok(150_000)
    );
    assert_eq!(vacant.allocated_bytes(), allocated);
}

#[test]
fn supply_template_hits_keep_division_gates_and_later_modifier_faults_fresh() {
    let mut source = normal_route_fixture();
    array(&mut source.bytes, T1 + 0xc0, 0x22000, 1);
    p64(&mut source.bytes, 0x22000, 0x23000);
    p32(&mut source.bytes, 0x22008, 2);
    source.bytes[0x23000 + 0x5b7] = 1;
    p32(&mut source.bytes, A1 + 0x498, 1);
    source
        .globals
        .insert(0x0331_9394, 2_i32.to_le_bytes().to_vec());
    let seed = speed_seed(&source);
    let observed = ScalarBatchMemory::new(&source);
    let mut geometry = SeededSpeedGeometry::new(&source);
    let packet = super::EtaInputPacket::new(1, 0, 0).unwrap();
    let mut shared = super::MovementSharedInputs {
        seed: &seed,
        route_ids: super::fresh_counter_eta::MovementRouteIds::Full(&[]),
        geometry: &mut geometry,
        rail_speeds: None,
        state_modifiers: None,
        category_contributions: None,
        supply_templates: Some(&packet.supply_templates),
        air_prefixes: Some(&packet.air_prefixes),
    };
    let prefix = [(A1 + 0x600, 8), (A1 + 0x498, 4), (0x0331_9394, 4)];
    assert_eq!(
        super::movement_supply_penalty(&observed, 0, A1, 0, 0, Some(&mut shared)),
        Ok(50_000)
    );
    assert_eq!(observed.reads.borrow().len(), prefix.len() + 4);
    observed.reads.borrow_mut().clear();
    assert_eq!(
        super::movement_supply_penalty(&observed, 0, A1, 0, 0, Some(&mut shared)),
        Ok(50_000)
    );
    assert_eq!(*observed.reads.borrow(), prefix);
    for (index, &(fault, _)) in prefix.iter().enumerate() {
        observed.fault.set(Some(fault));
        observed.reads.borrow_mut().clear();
        assert!(super::movement_supply_penalty(&observed, 0, A1, 0, 0, Some(&mut shared)).is_err());
        assert_eq!(*observed.reads.borrow(), prefix[..=index]);
    }
    observed.fault.set(Some(0x24000));
    observed.reads.borrow_mut().clear();
    assert!(
        super::movement_supply_penalty(&observed, 0, A1, 0x24000, 0, Some(&mut shared)).is_err()
    );
    assert_eq!(
        *observed.reads.borrow(),
        [prefix[0], prefix[1], prefix[2], (0x24000, 16)]
    );
    let transferring = super::DivisionSeed {
        naval_transfer_membership: Some(1),
        ..seed
    };
    shared.seed = &transferring;
    observed.reads.borrow_mut().clear();
    assert_eq!(
        super::movement_supply_penalty(&observed, 0, A1, 0, 0, Some(&mut shared)),
        Ok(0)
    );
    assert!(observed.reads.borrow().is_empty());
    let null_template = super::DivisionSeed {
        standard_template: Some(0),
        ..seed
    };
    shared.seed = &null_template;
    assert_eq!(
        super::movement_supply_penalty(&observed, 0, A1, 0, 0, Some(&mut shared)),
        Err("movement supply template is null")
    );
    assert_eq!(*observed.reads.borrow(), prefix);
}

#[test]
fn native_acclimatization_keeps_first_strict_maximum_and_signed_rounding() {
    let mut memory = presentation_fixture();
    let object = A1 + carmy::ACCLIMATIZATION;
    array(&mut memory.bytes, object + 8, 0x17000, 2);
    p64(&mut memory.bytes, object + 0x20, 0x18000);
    p64(&mut memory.bytes, object + 0x38, 100_000);
    for (at, category) in [(0x17000, 0x18000), (0x17010, 0x18100)] {
        p64(&mut memory.bytes, at, category);
        p64(&mut memory.bytes, at + 8, 50_001);
    }
    p64(&mut memory.bytes, 0x18000 + 0x30, 50_001);
    assert_eq!(super::movement_acclimatization(&memory, A1), Ok(74_999));
    p64(&mut memory.bytes, object + 0x20, 0x18100);
    assert_eq!(super::movement_acclimatization(&memory, A1), Ok(100_000));
    p64(&mut memory.bytes, object + 0x20, 0x18000);
    p64(&mut memory.bytes, 0x18000 + 0x30, (-1_i64) as u64);
    assert_eq!(super::movement_acclimatization(&memory, A1), Ok(100_000));
}

#[test]
fn native_speed_products_do_not_cancel_scale_or_use_wider_multiplication() {
    assert_eq!(super::movement_product(100_001, -50_001), -50_001);
    assert_eq!(super::movement_product(i64::MAX, 100_000), -1);
    assert_eq!(super::movement_product(i64::MIN, -1), i64::MIN / 100_000);
    assert!(super::movement_fraction(1_i64 << 58, -1).is_err());
    assert_eq!(super::movement_fraction(1, 0), Ok(0xffff_ffff));
}

fn presentation_source() -> [crate::cequipment_variant_pool::DivisionSource; 1] {
    [crate::cequipment_variant_pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        template: handle(0, 52),
        country: 3,
    }]
}

#[test]
fn shared_presentation_inputs_keep_exact_keys_fallback_faults_and_allocation() {
    let source = presentation_fixture();
    let memory = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: Some(0xdead),
    };
    let mut cache = super::PresentationInputs::new(1).unwrap();
    let allocated = cache.allocated_bytes();
    for _ in 0..4 {
        assert_eq!(cache.read::<u64>(&memory, P1), crate::read(&source, P1));
    }
    assert_eq!(
        memory.reads.borrow().iter().filter(|&&a| a == P1).count(),
        1
    );
    // A different width/address cannot use or evict the retained key.
    assert_eq!(cache.read::<u32>(&memory, P1), crate::read(&source, P1));
    assert_eq!(cache.read::<u32>(&memory, P1 + cprovince::ID), Some(1));
    assert_eq!(cache.read::<u64>(&memory, 0xdead), None);
    assert_eq!(cache.entries.len(), 3);
    cache.clear();
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&a| a == 0xdead)
            .count(),
        1
    );
    assert!(cache.entries.is_empty());
    assert!(cache.allocated_bytes() >= allocated);
    assert_eq!(cache.read::<u64>(&memory, 0xdead), None);
    assert!(
        cache.entries.is_empty(),
        "a failed vacant-slot read is not admitted"
    );
    cache.clear();
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&a| a == 0xdead)
            .count(),
        2
    );
    let mut empty = super::PresentationInputs::new(0).unwrap();
    assert_eq!(empty.read::<u64>(&memory, P1), crate::read(&source, P1));
    empty.clear();
}

fn shared_geometry_fixture() -> PresentationMemory {
    let mut memory = presentation_fixture();
    array(
        &mut memory.bytes,
        A1 + crate::cunit::ORDINARY_ROUTE_DATA,
        0x16000,
        2,
    );
    p32(&mut memory.bytes, 0x16000, 1);
    p32(&mut memory.bytes, 0x16004, 1);
    array(
        &mut memory.bytes,
        0x12000 + crate::province_definition::NEIGHBOURS,
        0x18000,
        3,
    );
    for (index, id) in [2, 1, 99].into_iter().enumerate() {
        p32(
            &mut memory.bytes,
            0x18000
                + index * crate::province_neighbour::STRIDE
                + crate::province_neighbour::PROVINCE_ID,
            id,
        );
    }
    p64(
        &mut memory.bytes,
        0x18000 + crate::province_neighbour::STRIDE + crate::province_neighbour::MOVEMENT_DISTANCE,
        200_000,
    );
    memory
}

fn owned_movement_fixture_run(
    memory: &impl crate::Memory,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    output: &mut [super::MovementRow],
    paths: &mut [u32],
) -> super::MovementWalk {
    let mut seeds: Vec<_> = sources
        .iter()
        .map(|source| super::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..super::DivisionSeed::default()
        })
        .collect();
    super::capture_division_seeds(memory, &mut seeds);
    // Worker geometry grows from the fixture's original province headers.
    let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
        crate::cprovince::GeometryCapacities::default(),
    );
    geometry.enable_growth();
    geometry.begin(memory, 0, GS).unwrap();
    let mut packet = super::MovementPacket::new(sources.len(), paths.len());
    let allocated = packet.allocated_bytes();
    super::capture_movement_inputs(
        memory,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: sources.len() },
        output.len(),
        paths.len(),
        &mut geometry,
        &mut packet,
    );
    let result = super::project_movement(&packet, output, paths);
    assert_eq!(packet.allocated_bytes(), allocated);
    result
}

fn movement_combat_fixture() -> (
    PresentationMemory,
    [crate::cequipment_variant_pool::DivisionSource; 3],
) {
    use crate::{cland_combat as combat, cland_combatant as side, cunit};
    let mut source = presentation_fixture();
    let army = source.bytes[A1..A1 + 0x700].to_vec();
    let sources = std::array::from_fn(|index| {
        let pointer = [A1, A2, A3][index];
        source.bytes[pointer..pointer + army.len()].copy_from_slice(&army);
        let division = handle(9 + index as u32, 9000);
        p64(&mut source.bytes, pointer + cunit::HANDLE, division);
        p32(&mut source.bytes, pointer + cunit::CLASS, 1);
        p32(&mut source.bytes, pointer + cunit::LOGICAL_OWNER, 3);
        p64(&mut source.bytes, pointer + cunit::MAP_ENTITY, 0);
        let references = 0x19000 + index * 0x10;
        array(
            &mut source.bytes,
            pointer + cunit::COMBAT_REFERENCE_DATA,
            references,
            1,
        );
        p64(&mut source.bytes, references, 0x16000);
        crate::cequipment_variant_pool::DivisionSource {
            pointer,
            division,
            ..presentation_source()[0]
        }
    });
    p64(&mut source.bytes, 0x16000, combat::VTABLE_RVA as u64);
    p64(&mut source.bytes, 0x16000 + combat::ATTACKER, 0x17000);
    p64(&mut source.bytes, 0x16000 + combat::PROVINCE, P1 as u64);
    p64(&mut source.bytes, 0x17000, side::VTABLE_RVA as u64);
    p64(&mut source.bytes, 0x17000 + side::COMBAT, 0x16000);
    p64(&mut source.bytes, 0x17000 + side::COUNTRIES, 0x18000);
    p32(&mut source.bytes, 0x18000, 3);
    (source, sources)
}

/// Both movement paths record whether CUnit+0x298 is set, and a failed
/// read of it fails the table rather than recording a guess.
#[test]
fn movement_records_the_map_entity_bit_and_fails_closed_on_its_read() {
    use crate::cunit;
    let (mut source, sources) = movement_combat_fixture();
    p64(&mut source.bytes, A2 + cunit::MAP_ENTITY, 0x14000);
    let run = |fault: Option<usize>| {
        let direct = ScalarBatchMemory::new(&source);
        let owned = ScalarBatchMemory::new(&source);
        direct.fault.set(fault);
        owned.fault.set(fault);
        let mut expected = [super::MovementRow::default(); 3];
        let mut actual = expected;
        let expected_walk = super::walk_movement(
            &direct,
            0,
            GS,
            &sources,
            Outcome::Rows { len: 3 },
            &mut expected,
            &mut [],
        );
        let actual_walk = owned_movement_fixture_run(&owned, &sources, &mut actual, &mut []);
        (
            (expected_walk.outcome, expected),
            (actual_walk.outcome, actual),
        )
    };
    let (expected, actual) = run(None);
    assert_eq!(actual, expected);
    assert_eq!(actual.0, Outcome::Rows { len: 3 });
    assert_eq!(actual.1.map(|row| row.map_entity), [0, 1, 0]);
    let (expected, actual) = run(Some(A2 + cunit::MAP_ENTITY));
    for outcome in [expected.0, actual.0] {
        assert!(
            matches!(outcome, Outcome::Unavailable { .. }),
            "{outcome:?}"
        );
    }
}

#[test]
fn optional_arrow_styles_preserve_all_movement_rows_and_read_after_the_prefix() {
    let (mut source, sources) = movement_combat_fixture();
    for (index, pointer) in [A1, A2, A3].into_iter().enumerate() {
        source.bytes[pointer + crate::cunit::WITHDRAWAL] = 0xfe - index as u8;
        p32(
            &mut source.bytes,
            pointer + crate::cunit::MOVEMENT_PRIORITY,
            (i32::MIN + index as i32) as u32,
        );
    }
    let run = |memory: &ScalarBatchMemory<'_>, rows: &mut [super::MovementRow; 3], owned| {
        if owned {
            owned_movement_fixture_run(memory, &sources, rows, &mut [])
        } else {
            super::walk_movement(
                memory,
                0,
                GS,
                &sources,
                Outcome::Rows { len: 3 },
                rows,
                &mut [],
            )
        }
    };
    let mut baseline = [super::MovementRow::default(); 3];
    let healthy = ScalarBatchMemory::new(&source);
    assert_eq!(
        run(&healthy, &mut baseline, true).outcome,
        Outcome::Rows { len: 3 }
    );
    let tail = [A1, A2, A3]
        .into_iter()
        .flat_map(|pointer| {
            [
                (pointer + crate::cunit::WITHDRAWAL, 1),
                (pointer + crate::cunit::MOVEMENT_PRIORITY, 4),
            ]
        })
        .collect::<Vec<_>>();
    assert!(healthy.reads.borrow().ends_with(&tail));
    for owned in [false, true] {
        let memory = ScalarBatchMemory::new(&source);
        let mut rows = [super::MovementRow::default(); 3];
        assert_eq!(
            run(&memory, &mut rows, owned).outcome,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(rows, baseline);
        assert!(memory.reads.borrow().ends_with(&tail));
        for fault in [
            A1 + crate::cunit::WITHDRAWAL,
            A1 + crate::cunit::MOVEMENT_PRIORITY,
        ] {
            let memory = ScalarBatchMemory::new(&source);
            memory.fault.set(Some(fault));
            let mut rows = [super::MovementRow::default(); 3];
            assert_eq!(
                run(&memory, &mut rows, owned).outcome,
                Outcome::Rows { len: 3 }
            );
            assert_eq!(
                memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(address, _)| address == fault)
                    .count(),
                1,
                "an optional field is read once without retry"
            );
            assert_eq!(
                rows[0].withdrawal,
                (fault != A1 + crate::cunit::WITHDRAWAL).then_some(0xfe)
            );
            assert_eq!(
                rows[0].movement_priority,
                (fault != A1 + crate::cunit::MOVEMENT_PRIORITY).then_some(i32::MIN)
            );
            assert_eq!(rows[1..], baseline[1..]);
            for (mut actual, mut expected) in rows.into_iter().zip(baseline) {
                actual.withdrawal = None;
                actual.movement_priority = None;
                expected.withdrawal = None;
                expected.movement_priority = None;
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn movement_combat_memo_preserves_rows_raw_owner_keys_and_first_pointer_reads() {
    use crate::{cland_combat as combat, cland_combatant as side, cunit};
    for (owner, groups) in [(3, 1), (1, 2), (2, 2)] {
        for border in [false, true] {
            let (mut source, sources) = movement_combat_fixture();
            p32(&mut source.bytes, A2 + cunit::LOGICAL_OWNER, owner);
            if border {
                p64(
                    &mut source.bytes,
                    0x16000,
                    crate::cland_border_war_combat::VTABLE_RVA as u64,
                );
                p64(
                    &mut source.bytes,
                    0x17000,
                    crate::cland_border_war_combatant::VTABLE_RVA as u64,
                );
            }
            let direct = ScalarBatchMemory::new(&source);
            let owned = ScalarBatchMemory::new(&source);
            let mut expected = [super::MovementRow::default(); 3];
            let mut actual = expected;
            let expected_walk = super::walk_movement(
                &direct,
                0,
                GS,
                &sources,
                Outcome::Rows { len: 3 },
                &mut expected,
                &mut [],
            );
            let actual_walk = owned_movement_fixture_run(&owned, &sources, &mut actual, &mut []);
            assert_eq!((actual_walk, actual), (expected_walk, expected));
            assert_eq!(actual_walk.outcome, Outcome::Rows { len: 3 });
            assert_eq!(
                actual.map(|row| row.combat_role),
                [1, if owner == 2 { 2 } else { 1 }, 1]
            );
            assert_eq!(actual.map(|row| row.combat_province), [1; 3]);
            let count = |memory: &ScalarBatchMemory<'_>, address| {
                memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == address)
                    .count()
            };
            for address in [0x19000, 0x19010, 0x19020] {
                assert_eq!(count(&direct, address), 1);
                assert_eq!(count(&owned, address), 1);
            }
            for address in [
                0x16000,
                0x16000 + combat::ATTACKER,
                0x17000,
                0x17000 + side::COMBAT,
                0x17000 + side::COUNTRIES,
                0x18000,
                0x16000 + combat::PROVINCE,
            ] {
                assert_eq!(count(&direct, address), 3);
                assert_eq!(count(&owned, address), groups);
            }
        }
    }
}

#[test]
fn movement_combat_memo_keeps_cold_prefixes_and_never_retains_failed_queries() {
    let (source, _) = movement_combat_fixture();
    let memory = ScalarBatchMemory::new(&source);
    let mut packet = super::MovementPacket::new(1, 0);
    let allocated = packet.allocated_bytes();
    let expected = [
        (0x16000, 8),
        (0x16028, 8),
        (0x17000, 8),
        (0x17018, 8),
        (GS + crate::cgame_state::COUNTRIES, 16),
        (GS + crate::cgame_state::COUNTRY_LINKS, 16),
        (LINKS + 12, 4),
        (COUNTRIES + 8, 8),
        (C1, 8),
        (C1 + 8, 4),
        (LINKS + 12, 4),
        (LINKS + 4, 4),
        (LINKS + 4, 4),
        (COUNTRIES + 8, 8),
        (0x17038, 8),
        (0x18000, 4),
        (0x16038, 8),
    ];
    let query = |memo: &mut super::MovementCombats| {
        memo.capture(&memory, 0, GS, 0x16000, Some(3), |province| {
            assert_eq!(province, P1);
            Ok(1)
        })
    };
    assert_eq!(query(&mut packet.combats), Ok((1, 1)));
    assert_eq!(*memory.reads.borrow(), expected);
    assert_eq!(expected.iter().map(|&(_, bytes)| bytes).sum::<usize>(), 128);
    memory.reads.borrow_mut().clear();
    assert_eq!(query(&mut packet.combats), Ok((1, 1)));
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(packet.allocated_bytes(), allocated);
    for &(fault, _) in &expected {
        packet.clear();
        memory.fault.set(Some(fault));
        let first = expected
            .iter()
            .position(|&(address, _)| address == fault)
            .unwrap();
        for _ in 0..2 {
            memory.reads.borrow_mut().clear();
            assert!(query(&mut packet.combats).is_err());
            assert_eq!(*memory.reads.borrow(), expected[..=first]);
            assert!(packet.combats.entries.is_empty());
        }
    }
    memory.fault.set(None);
    packet.clear();
    assert_eq!(
        packet
            .combats
            .capture(&memory, 0, GS, 0x16000, Some(3), |_| Err(
                "province failure"
            )),
        Err("province failure")
    );
    assert!(packet.combats.entries.is_empty());
    memory.reads.borrow_mut().clear();
    assert_eq!(query(&mut packet.combats), Ok((1, 1)));
    assert_eq!(*memory.reads.borrow(), expected);
    for full in [false, true] {
        let mut memo = super::MovementCombats::new(usize::from(full));
        for pointer in 0..memo.entries.capacity() {
            memo.entries.insert(
                super::MovementCombatKey {
                    base: 0,
                    game_state: GS,
                    pointer,
                    owner: 3,
                },
                (2, 0),
            );
        }
        let capacity = memo.entries.capacity();
        for _ in 0..2 {
            memory.reads.borrow_mut().clear();
            assert_eq!(query(&mut memo), Ok((1, 1)));
            assert_eq!(*memory.reads.borrow(), expected);
            assert_eq!(memo.entries.capacity(), capacity);
            assert_eq!(memo.entries.len(), capacity);
        }
    }
}

#[test]
fn movement_combat_memo_preserves_membership_laziness_and_context_changes() {
    let (mut source, _) = movement_combat_fixture();
    let mut packet = super::MovementPacket::new(8, 0);
    {
        let memory = ScalarBatchMemory::new(&source);
        memory.fault.set(Some(0x18010));
        for owner in [3, 1] {
            assert_eq!(
                packet
                    .combats
                    .capture(&memory, 0, GS, 0x16000, Some(owner), |_| Ok(1)),
                Ok((1, 1))
            );
            assert!(!memory.reads.borrow().contains(&(0x18010, 8)));
        }
        assert_eq!(packet.combats.entries.len(), 2);
        memory.reads.borrow_mut().clear();
        assert_eq!(
            packet
                .combats
                .capture(&memory, 0, GS, 0x16000, Some(2), |_| panic!(
                    "membership failure must precede province"
                )),
            Err("combat country next unreadable")
        );
        assert_eq!(memory.reads.borrow().last(), Some(&(0x18010, 8)));
        assert!(!memory.reads.borrow().contains(&(0x16038, 8)));
        assert_eq!(packet.combats.entries.len(), 2);
        for (base, state, pointer, owner, reason) in [
            (
                1,
                GS,
                0x16000,
                Some(3),
                "division first combat identity mismatch",
            ),
            (
                0,
                usize::MAX,
                0x16000,
                Some(3),
                "country array unreadable or invalid",
            ),
            (
                0,
                GS,
                0x16001,
                Some(3),
                "division first combat identity mismatch",
            ),
            (
                0,
                GS,
                0x16000,
                None,
                "division presentation field unreadable",
            ),
        ] {
            memory.reads.borrow_mut().clear();
            assert_eq!(
                packet
                    .combats
                    .capture(&memory, base, state, pointer, owner, |_| panic!(
                        "invalid context reached province"
                    )),
                Err(reason)
            );
            assert!(!memory.reads.borrow().is_empty());
            assert_eq!(packet.combats.entries.len(), 2);
        }
    }
    // Capture clear must reacquire membership at unchanged pointers.
    p32(&mut source.bytes, 0x18000, 2);
    packet.clear();
    let memory = ScalarBatchMemory::new(&source);
    assert_eq!(
        packet
            .combats
            .capture(&memory, 0, GS, 0x16000, Some(3), |_| Ok(1)),
        Ok((2, 1))
    );
    assert!(memory.reads.borrow().contains(&(0x18010, 8)));
    memory.reads.borrow_mut().clear();
    assert_eq!(
        packet
            .combats
            .capture(&memory, 0, GS, 0x16000, Some(3), |_| panic!(
                "complete hit must be owned"
            )),
        Ok((2, 1))
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn movement_combat_hits_do_not_hide_later_first_pointer_failure() {
    for invalid in [None, Some(0), Some(0x16001)] {
        let (mut source, sources) = movement_combat_fixture();
        if let Some(pointer) = invalid {
            p64(&mut source.bytes, 0x19010, pointer);
        }
        let direct = ScalarBatchMemory::new(&source);
        let owned = ScalarBatchMemory::new(&source);
        if invalid.is_none() {
            direct.fault.set(Some(0x19010));
            owned.fault.set(Some(0x19010));
        }
        let mut expected = [super::MovementRow::default(); 3];
        let mut actual = expected;
        let expected_walk = super::walk_movement(
            &direct,
            0,
            GS,
            &sources,
            Outcome::Rows { len: 3 },
            &mut expected,
            &mut [],
        );
        let actual_walk = owned_movement_fixture_run(&owned, &sources, &mut actual, &mut []);
        assert_eq!((actual_walk, actual), (expected_walk, expected));
        assert!(matches!(actual_walk.outcome, Outcome::Unavailable { .. }));
        assert_eq!(actual[0].combat_role, 1);
        assert!(!owned.reads.borrow().contains(&(0x19020, 8)));
        for memory in [&direct, &owned] {
            for pointer in [A1, A2, A3] {
                assert!(!memory.reads.borrow().iter().any(|&(address, _)| {
                    address == pointer + crate::cunit::WITHDRAWAL
                        || address == pointer + crate::cunit::MOVEMENT_PRIORITY
                }));
            }
        }
    }
}

#[test]
fn owned_movement_projection_matches_direct_route_caps_and_stack_branches() {
    for class in [0, 1, 13] {
        for moving in [0, 1] {
            let mut memory = shared_geometry_fixture();
            p32(&mut memory.bytes, A1 + crate::cunit::CLASS, class);
            p32(
                &mut memory.bytes,
                0x14000 + crate::unit_map_entity_layout::MOVEMENT_SELECTOR,
                moving,
            );
            let sources = [presentation_source()[0]; 3];
            for rows in [0, 1, 3] {
                for paths in [0, 1, 2, 5, 6] {
                    let mut expected = [super::MovementRow::default(); 3];
                    let mut actual = expected;
                    let mut expected_paths = [0; 6];
                    let mut actual_paths = expected_paths;
                    let direct = super::walk_movement(
                        &memory,
                        0,
                        GS,
                        &sources,
                        Outcome::Rows { len: sources.len() },
                        &mut expected[..rows],
                        &mut expected_paths[..paths],
                    );
                    let owned = owned_movement_fixture_run(
                        &memory,
                        &sources,
                        &mut actual[..rows],
                        &mut actual_paths[..paths],
                    );
                    assert_eq!(
                        (owned, actual, actual_paths),
                        (direct, expected, expected_paths)
                    );
                }
            }
        }
    }
}

#[test]
fn owned_movement_admits_later_short_route_and_ignores_private_status_fault() {
    let mut source = shared_geometry_fixture();
    let army = source.bytes[A1..A1 + 0x700].to_vec();
    source.bytes[A2..A2 + 0x700].copy_from_slice(&army);
    p64(
        &mut source.bytes,
        A2 + crate::cunit::HANDLE,
        handle(10, 9000),
    );
    p64(&mut source.bytes, A2 + crate::cunit::MAP_ENTITY, 0);
    array(
        &mut source.bytes,
        A2 + crate::cunit::ORDINARY_ROUTE_DATA,
        0x16000,
        1,
    );
    let sources = [
        presentation_source()[0],
        crate::cequipment_variant_pool::DivisionSource {
            pointer: A2,
            division: handle(10, 9000),
            ..presentation_source()[0]
        },
    ];
    let memory = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: Some(A2 + crate::cunit::STAT_BLOCK),
    };
    let mut rows = [super::MovementRow::default(); 2];
    let result = owned_movement_fixture_run(&memory, &sources, &mut rows, &mut [0]);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((result.paths_len, result.required_paths), (1, 3));
    assert_eq!(rows[0].division, handle(10, 9000));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == A2 + crate::cunit::STAT_BLOCK)
            .count(),
        1
    );
}

mod status_batches;

#[path = "tests/border_index/tests.rs"]
mod border_index;

mod movement_pairs;

#[test]
fn shared_movement_geometry_matches_direct_caps_and_reduces_repeated_guard_reads() {
    let mut source = shared_geometry_fixture();
    // The captured ETA denominator no longer consults this province. Give
    // the movement row non-null source/target references so this legacy
    // direct-reader comparison still exercises shared province identity.
    p64(&mut source.bytes, A1 + super::MOVEMENT_TARGET, P1 as u64);
    p64(&mut source.bytes, A1 + super::MOVEMENT_SOURCE, P1 as u64);
    let sources = [presentation_source()[0]; 8];
    for cache_capacity in [0, 1, 128] {
        let mut cache = super::MovementInputs::new(cache_capacity);
        let allocated = cache.allocated_bytes();
        for row_capacity in [0, 1, 8] {
            for path_capacity in [0, 1, 2, 15, 16] {
                let direct = ObservedMovementMemory {
                    source: &source,
                    reads: Default::default(),
                    fault: None,
                };
                let cached = ObservedMovementMemory {
                    source: &source,
                    reads: Default::default(),
                    fault: None,
                };
                let mut a = [super::MovementRow::default(); 8];
                let mut b = a;
                let mut ap = [0; 16];
                let mut bp = ap;
                let outcome = Outcome::Rows { len: sources.len() };
                let expected = super::walk_movement(
                    &direct,
                    0,
                    GS,
                    &sources,
                    outcome,
                    &mut a[..row_capacity],
                    &mut ap[..path_capacity],
                );
                let actual = super::walk_movement_with_inputs(
                    &cached,
                    0,
                    GS,
                    &sources,
                    outcome,
                    &mut b[..row_capacity],
                    &mut bp[..path_capacity],
                    &mut cache,
                );
                assert_eq!((actual, b, bp), (expected, a, ap));
                if cache_capacity == 128 && row_capacity == 8 && path_capacity == 16 {
                    assert_eq!(actual.outcome, Outcome::Rows { len: sources.len() });
                    assert!(cached.reads.borrow().len() < direct.reads.borrow().len());
                    assert_eq!(
                        cached.reads.borrow().iter().filter(|&&a| a == P1).count(),
                        1,
                        "the acquired province identity is reused"
                    );
                }
                assert!(cache.shared.borrow().entries.is_empty());
                assert!(cache.allocated_bytes() >= allocated);
            }
        }
    }
}

#[test]
fn shared_movement_geometry_preserves_first_faults_and_unvisited_neighbor_suffix() {
    let source = shared_geometry_fixture();
    let observed = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let sources = presentation_source();
    let outcome = Outcome::Rows { len: 1 };
    super::walk_movement(
        &observed,
        0,
        GS,
        &sources,
        outcome,
        &mut [super::MovementRow::default()],
        &mut [0; 2],
    );
    let addresses: std::collections::BTreeSet<_> =
        observed.reads.borrow().iter().copied().collect();
    for fault in addresses {
        let direct = ObservedMovementMemory {
            source: &source,
            reads: Default::default(),
            fault: Some(fault),
        };
        let cached = ObservedMovementMemory {
            source: &source,
            reads: Default::default(),
            fault: Some(fault),
        };
        let mut a = [super::MovementRow::default()];
        let mut b = a;
        let mut ap = [0; 2];
        let mut bp = ap;
        let expected = super::walk_movement(&direct, 0, GS, &sources, outcome, &mut a, &mut ap);
        let mut cache = super::MovementInputs::new(128);
        let actual = super::walk_movement_with_inputs(
            &cached, 0, GS, &sources, outcome, &mut b, &mut bp, &mut cache,
        );
        assert_eq!((actual, b, bp), (expected, a, ap), "fault {fault:x}");
        assert_eq!(
            cached
                .reads
                .borrow()
                .iter()
                .filter(|&&a| a == fault)
                .count(),
            1,
            "failed source is not retained or retried"
        );
        assert!(cache.shared.borrow().entries.is_empty());
    }
    let forbidden =
        0x18000 + 2 * crate::province_neighbour::STRIDE + crate::province_neighbour::PROVINCE_ID;
    let lazy = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: Some(forbidden),
    };
    let mut cache = super::MovementInputs::new(128);
    assert_eq!(
        super::walk_movement_with_inputs(
            &lazy,
            0,
            GS,
            &sources,
            outcome,
            &mut [super::MovementRow::default()],
            &mut [0; 2],
            &mut cache
        )
        .outcome,
        outcome
    );
    assert!(!lazy.reads.borrow().contains(&forbidden));
}

#[test]
fn movement_routes_are_whole_and_keep_whole_row_capacity() {
    use crate::{cunit, province_definition as definition, province_neighbour as neighbor};
    let mut memory = presentation_fixture();
    let b = &mut memory.bytes;
    array(b, A1 + cunit::ORDINARY_ROUTE_DATA, 0x16000, 300);
    array(b, A1 + cunit::ALTERNATE_ROUTE_DATA, 0x17000, 2);
    for i in 0..300 {
        p32(b, 0x16000 + i * 4, 1);
    }
    p32(b, 0x16000 + 299 * 4, 99);
    p32(b, 0x17000, 21);
    p32(b, 0x17004, 22);
    array(b, 0x12000 + definition::NEIGHBOURS, 0x18000, 2);
    p32(b, 0x18000 + neighbor::PROVINCE_ID, 2);
    p32(b, 0x18000 + neighbor::STRIDE + neighbor::PROVINCE_ID, 1);
    p64(
        b,
        0x18000 + neighbor::STRIDE + neighbor::MOVEMENT_DISTANCE,
        200_000,
    );
    p64(b, A1 + carmy::MOVEMENT_NUMERATOR, 300_000);
    let mut rows = [super::MovementRow::default(); 1];
    // Both routes are recorded whole; only the arena bounds them.
    let mut paths = [0; 302];
    let result = super::walk_movement(
        &memory,
        0,
        GS,
        &presentation_source(),
        Outcome::Rows { len: 1 },
        &mut rows,
        &mut paths,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!((result.paths_len, result.required_paths), (302, 302));
    assert_eq!(
        (paths[0], paths[299], rows[0].path_len, rows[0].path_total),
        (1, 99, 300, 300)
    );
    assert_eq!(rows[0].numerator, 300_000);
    assert_eq!(&paths[300..], &[21, 22]);
    for capacity in [0, 301] {
        let result = super::walk_movement(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2,
            },
            &mut rows,
            &mut paths[..capacity],
        );
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        assert_eq!((result.paths_len, result.required_paths), (0, 302));
    }
    p64(&mut memory.bytes, A1, 0);
    assert!(matches!(
        super::walk_movement(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows,
            &mut paths
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn division_status_keeps_the_model_number_and_acclimatization_and_fails_only_that_field() {
    use crate::{acclimatization_category, acclimatization_data, acclimatization_entry::Entry};
    let mut memory = presentation_fixture();
    p32(
        &mut memory.bytes,
        A1 + carmy::RETAINED_VISUAL_VARIANT,
        0x1ff,
    );
    let object = A1 + carmy::ACCLIMATIZATION;
    array(
        &mut memory.bytes,
        object + acclimatization_data::DATA,
        0x16000,
        2,
    );
    p64(
        &mut memory.bytes,
        object + acclimatization_data::MAXIMUM,
        250_000,
    );
    for (index, (category, token, weight)) in [(0x16100_usize, 7_u32, 90_000_i64), (0x16200, 8, -1)]
        .into_iter()
        .enumerate()
    {
        p64(&mut memory.bytes, 0x16000 + index * 16, category as u64);
        p64(&mut memory.bytes, 0x16008 + index * 16, weight as u64);
        p32(
            &mut memory.bytes,
            category + acclimatization_category::TOKEN,
            token,
        );
    }
    // The registry lists both categories, in a different order.
    memory.globals.insert(
        acclimatization_category::REGISTRY_RVA,
        0x16800_u64.to_le_bytes().to_vec(),
    );
    array(&mut memory.bytes, 0x16800, 0x16900, 2);
    p64(&mut memory.bytes, 0x16900, 0x16200);
    p64(&mut memory.bytes, 0x16908, 0x16100);
    let capture = |memory: &PresentationMemory,
                   packet: &mut super::StatusPacket,
                   rows: &mut [super::StatusRow; 1]| {
        super::status_test_capture(
            memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            rows,
            packet,
        )
    };
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        capture(&memory, &mut packet, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        (rows[0].model_number, rows[0].acclimatization_maximum),
        (0x1ff, Some(250_000))
    );
    assert_eq!(
        packet.acclimatization(&rows[0]),
        Some(
            &[
                Entry {
                    category: 7,
                    weight: 90_000
                },
                Entry {
                    category: 8,
                    weight: -1
                },
            ][..]
        )
    );
    let field_failed = |rows: &[super::StatusRow; 1], packet: &super::StatusPacket| {
        rows[0].model_number == 0x1ff
            && rows[0].acclimatization_maximum.is_none()
            && packet.acclimatization(&rows[0]) == Some(&[][..])
    };
    // Packets too small for the entries or the registry fail only the
    // field, and ask for room.
    for (capacities, required) in [
        (
            super::StatusCapacities {
                acclimatization: 1,
                acclimatization_categories: 2,
                modifiers: crate::modifier_accumulator::SnapshotCapacity {
                    nodes: 1,
                    entries: 1,
                },
                ..super::StatusCapacities::new(1)
            },
            (2, 2),
        ),
        (
            super::StatusCapacities {
                acclimatization: 2,
                acclimatization_categories: 1,
                modifiers: crate::modifier_accumulator::SnapshotCapacity {
                    nodes: 1,
                    entries: 1,
                },
                ..super::StatusCapacities::new(1)
            },
            (2, 2),
        ),
    ] {
        let mut small = super::StatusPacket::with_capacities(capacities).unwrap();
        assert_eq!(
            capture(&memory, &mut small, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert!(field_failed(&rows, &small));
        let needed = small.required_capacities();
        assert_eq!(
            (needed.acclimatization, needed.acclimatization_categories),
            required
        );
    }
    // An entry whose category is not registered is not a category.
    p64(&mut memory.bytes, 0x16010, 0x16300);
    assert_eq!(
        capture(&memory, &mut packet, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert!(field_failed(&rows, &packet));
    p64(&mut memory.bytes, 0x16010, 0x16200);
    // An unauthenticated object is not an empty acclimatization.
    p64(&mut memory.bytes, object, 0);
    assert_eq!(
        capture(&memory, &mut packet, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert!(field_failed(&rows, &packet));
}

#[test]
fn batched_retreat_bans_equal_one_group_reads_including_faults() {
    use crate::{carmy_leader, cunit_leader};
    const LEADER: usize = 0x20000;
    const OTHER: usize = 0x30000;
    let mut memory = presentation_fixture();
    memory.bytes.resize(0x40000, 0);
    let b = &mut memory.bytes;
    p64(b, G1 + corders_group::LEADER, LEADER as u64);
    p64(b, LEADER, carmy_leader::VTABLE_RVA as u64);
    p32(b, LEADER + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
    for (block, data, value) in [
        (carmy_leader::MOVEMENT_MODIFIER, 0x21000, 1_u64),
        (carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER, 0x21100, 0),
    ] {
        array(b, LEADER + block + 0x10, data, 2);
        p32(b, data, carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING);
        p64(b, data + 8, value);
        p32(b, data + 16, carmy_leader::CANNOT_RETREAT_WHILE_ATTACKING);
        p64(b, data + 24, 1);
    }
    // A second group whose commander lies outside readable memory.
    p64(b, OTHER, corders_group::VTABLE_RVA as u64);
    p32(
        b,
        OTHER + corders_group::HANDLE,
        corders_group::REFERENCE_TYPE,
    );
    p64(b, OTHER + corders_group::LEADER, 0x50000);
    // A group without a commander.
    p64(b, OTHER + 0x1000, corders_group::VTABLE_RVA as u64);
    p32(
        b,
        OTHER + 0x1000 + corders_group::HANDLE,
        corders_group::REFERENCE_TYPE,
    );
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA,
        0x22000_u64.to_le_bytes().to_vec(),
    );
    // Authenticated, no commander, two non-groups, an unreadable group,
    // an unreadable commander, and a repeat.
    let groups = [
        G1,
        OTHER + 0x1000,
        OTHER + 0x100,
        G1 + 0x10,
        0x60000,
        OTHER,
        G1,
    ];
    let mut batched = [Ok(None); 7];
    carmy_leader::retreat_bans_many(
        &memory,
        0,
        &groups,
        &mut batched,
        &crate::modifier_definition::DefinitionMemo::default(),
    );
    for (group, batched) in groups.into_iter().zip(batched) {
        assert_eq!(
            batched,
            carmy_leader::retreat_bans(&memory, 0, group),
            "{group:#x}"
        );
    }
    assert_eq!(
        batched[0]
            .unwrap()
            .map(|bans| (bans.ordinary, bans.alternate)),
        Some((
            carmy_leader::RETREAT_BANNED_DEFENDING | carmy_leader::RETREAT_BANNED_ATTACKING,
            carmy_leader::RETREAT_BANNED_ATTACKING
        ))
    );
}

#[test]
fn division_status_move_flags_read_exile_and_exfiltration_and_bans_are_their_own_read() {
    use crate::{carmy_leader, csub_unit_definition as stats, cunit, cunit_leader};
    const LEADER: usize = 0x20000;
    let mut memory = presentation_fixture();
    memory.bytes.resize(0x40000, 0);
    let run = |memory: &PresentationMemory| {
        let mut rows = [super::StatusRow::default(); 1];
        let outcome = super::walk_status(
            memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows,
        );
        (outcome, rows[0].move_flags)
    };
    // The daily bans read, for the owners `owner` maps.
    type Owner<'o> = &'o dyn Fn(u32) -> Result<Option<u32>, &'static str>;
    let seeds_of = |memory: &PresentationMemory| {
        let mut seeds: Vec<_> = presentation_source()
            .iter()
            .map(|source| super::DivisionSeed {
                pointer: source.pointer,
                division: source.division,
                ..super::DivisionSeed::default()
            })
            .collect();
        super::capture_division_seeds(memory, &mut seeds);
        seeds
    };
    let read = |memory: &PresentationMemory, seeds: &[super::DivisionSeed], owner: Owner<'_>| {
        let mut packet = super::StatusPacket::new(1).unwrap();
        let mut rows = [super::RetreatBanRow::default(); 1];
        let outcome =
            packet.retreat_bans(memory, 0, seeds, Outcome::Rows { len: 1 }, owner, &mut rows);
        (outcome, rows[0])
    };
    let bans =
        |memory: &PresentationMemory, owner: Owner<'_>| read(memory, &seeds_of(memory), owner);
    let rows = Outcome::Rows { len: 1 };
    assert_eq!(run(&memory), (rows, 0));
    memory.bytes[A1 + carmy::EXILED] = 1;
    memory.bytes[0x11000 + stats::CAN_EXFILTRATE_FROM_COAST] = 0x80;
    let base_flags =
        super::StatusRow::MOVE_EXILED | super::StatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST;
    assert_eq!(run(&memory), (rows, base_flags));
    // No commander bans nothing: no row.
    assert_eq!(bans(&memory, &|_| Ok(Some(3))).0, Outcome::Rows { len: 0 });
    // The group's commander bans retreat while defending in both blocks.
    let b = &mut memory.bytes;
    p64(b, G1 + corders_group::LEADER, LEADER as u64);
    p64(b, LEADER, carmy_leader::VTABLE_RVA as u64);
    p32(b, LEADER + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
    for (block, data) in [
        (carmy_leader::MOVEMENT_MODIFIER, 0x21000),
        (carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER, 0x21100),
    ] {
        // The CModifier's value accumulator sits at +0x10 (C0AB20 callers).
        array(b, LEADER + block + 0x10, data, 2);
        p32(b, data, carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING);
        p64(b, data + 8, 1);
        p32(b, data + 16, carmy_leader::CANNOT_RETREAT_WHILE_ATTACKING);
        p64(b, data + 24, 0);
    }
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA,
        0x22000_u64.to_le_bytes().to_vec(),
    );
    // The hourly status never reads the bans, in combat or not.
    assert_eq!(run(&memory), (rows, base_flags));
    array(
        &mut memory.bytes,
        A1 + cunit::COMBAT_REFERENCE_DATA,
        0x21800,
        1,
    );
    assert_eq!(run(&memory), (rows, base_flags));
    // The bans read has a row for a read owner, whatever its combats.
    let (outcome, row) = bans(&memory, &|_| Ok(Some(3)));
    assert_eq!(outcome, rows);
    assert_eq!(
        (row.owner, row.bans),
        (3, carmy_leader::RETREAT_BANNED_DEFENDING)
    );
    // An owner outside the read has none.
    assert_eq!(bans(&memory, &|_| Ok(None)).0, Outcome::Rows { len: 0 });
    // An unresolved owner could be a read owner: the read returns no
    // rows and is truncated, so no earlier ban is replaced.
    let (outcome, row) = bans(&memory, &|_| Err("country link unreadable"));
    assert_eq!(
        (outcome, row),
        (
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            super::RetreatBanRow::default()
        )
    );
    // So is an unreadable roster owner field.
    let mut seeds = seeds_of(&memory);
    seeds[0].roster_owner = None;
    assert_eq!(
        read(&memory, &seeds, &|_| Ok(Some(3))).0,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    // A commander that is not an army leader fails the read, never zeroes it.
    p64(&mut memory.bytes, LEADER, 0);
    assert!(matches!(
        bans(&memory, &|_| Ok(Some(3))).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn division_status_preserves_maxima_and_distinguishes_training_orders_and_attrition() {
    let mut memory = presentation_fixture();
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    let row = rows[0];
    assert_eq!(
        (row.maximum_hit_points, row.maximum_organisation),
        (200_000, i64::MAX)
    );
    assert_eq!(
        (row.entrenchment, row.entrenchment_cap, row.supply_ratio),
        (i64::MIN, i64::MAX, -25)
    );
    assert_eq!(
        (
            row.attrition_threshold,
            row.attrition,
            row.has_order,
            row.exercising
        ),
        (500, 1, 0, 0)
    );
    memory.bytes[G1 + corders_group::TRAINING] = 1;
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].exercising, 1);
    p64(&mut memory.bytes, A1 + carmy::STRENGTH, 100_000); // Equality is not enough.
    p64(&mut memory.bytes, A1 + carmy::COUNTER_ATTRITION, 500);
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].attrition, rows[0].exercising), (0, 0));
    p64(&mut memory.bytes, A1 + carmy::ORDERS_GROUP, 0);
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].has_order, 0);
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Truncated {
                len: 1,
                unread_rows: 3
            },
            &mut []
        ),
        Outcome::Truncated {
            len: 0,
            unread_rows: 4
        }
    );
}

#[test]
fn status_membership_packet_preserves_rows_gaps_and_later_scalar_faults() {
    use crate::{corder_instance as order, corders_group_member::MembershipPacket, cunit};
    let mut memory = presentation_fixture();
    array(
        &mut memory.bytes,
        G1 + corders_group::PRIMARY_ORDER_DATA,
        0x17000,
        1,
    );
    p64(&mut memory.bytes, 0x17000, 0x18000);
    p64(&mut memory.bytes, 0x18000, order::VTABLE_RVA as u64);
    p64(&mut memory.bytes, 0x18000 + order::OWNER, G1 as u64);
    array(
        &mut memory.bytes,
        0x18000 + order::ASSIGNED_MEMBER_DATA,
        0x19000,
        1,
    );
    p64(
        &mut memory.bytes,
        0x19000,
        (A1 + cunit::SCHEDULED_MEMBER) as u64,
    );
    let mut cache = MembershipPacket::new(8, 8, 8, 8);
    let mut direct = [super::StatusRow::default(); 1];
    let mut cached = direct;
    for outcome in [
        Outcome::Rows { len: 1 },
        Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        },
    ] {
        assert_eq!(
            super::walk_status_with_membership(
                &memory,
                0,
                GS,
                &presentation_source(),
                outcome,
                &mut cached,
                &mut cache
            ),
            super::walk_status(&memory, 0, GS, &presentation_source(), outcome, &mut direct),
        );
        assert_eq!(cached, direct);
        assert_eq!(cached[0].has_order, 1);
    }
    let observed = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: Some(A1 + carmy::COUNTER_ATTRITION),
    };
    assert!(matches!(
        super::walk_status_with_membership(
            &observed,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut cached,
            &mut cache
        ),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(
        observed
            .reads
            .borrow()
            .iter()
            .filter(|&&address| address == 0x19000)
            .count(),
        1,
        "membership retains its first acquired bytes before a later scalar fails"
    );
    p64(&mut memory.bytes, 0x19000, A1 as u64);
    assert_eq!(
        super::walk_status_with_membership(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut cached,
            &mut cache
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        cached[0].has_order, 0,
        "the failed prior pass cannot retain a cached member"
    );
    let unavailable = Outcome::Unavailable {
        reason: "source gap",
    };
    assert_eq!(
        super::walk_status_with_membership(
            &memory,
            0,
            GS,
            &[],
            unavailable,
            &mut cached,
            &mut cache
        ),
        unavailable
    );
}

#[test]
fn status_vacancy_has_no_retained_allocations_and_rejects_unprepared_inputs() {
    let memory = presentation_fixture();
    let mut packet = super::StatusPacket::vacant();
    assert_eq!(packet.allocated_bytes(), 0);
    assert_eq!(packet.project(&mut []), Outcome::Rows { len: 0 });
    packet.clear();
    packet
        .grow(
            0,
            crate::corders_group_member::MembershipCapacity::default(),
        )
        .unwrap();
    assert_eq!(packet.allocated_bytes(), 0);
    let unavailable = Outcome::Unavailable {
        reason: "source gap",
    };
    assert_eq!(
        packet.capture(&memory, 0, GS, &[], unavailable, 0),
        unavailable
    );
    assert_eq!(
        packet.capture(
            &memory,
            0,
            GS,
            &[super::DivisionSeed::default()],
            Outcome::Rows { len: 1 },
            1,
        ),
        Outcome::Unavailable {
            reason: "division status seed packet capacity exceeded",
        }
    );
    assert_eq!(packet.allocated_bytes(), 0);
}

#[test]
fn status_retains_first_pass_membership_growth_through_projection_and_recycling() {
    use crate::{corder_instance as order, cunit};
    let mut memory = presentation_fixture();
    array(
        &mut memory.bytes,
        G1 + corders_group::PRIMARY_ORDER_DATA,
        0x17000,
        1,
    );
    p64(&mut memory.bytes, 0x17000, 0x18000);
    p64(&mut memory.bytes, 0x18000, order::VTABLE_RVA as u64);
    p64(&mut memory.bytes, 0x18000 + order::OWNER, G1 as u64);
    array(
        &mut memory.bytes,
        0x18000 + order::ASSIGNED_MEMBER_DATA,
        0x19000,
        193,
    );
    p64(
        &mut memory.bytes,
        0x19000 + 192 * 8,
        (A1 + cunit::SCHEDULED_MEMBER) as u64,
    );
    let source = presentation_source()[0];
    let mut seeds = [super::DivisionSeed {
        pointer: source.pointer,
        division: source.division,
        ..super::DivisionSeed::default()
    }];
    super::capture_division_seeds(&memory, &mut seeds);
    let mut capacities = super::StatusCapacities::new(1);
    capacities.modifiers = crate::modifier_accumulator::SnapshotCapacity {
        nodes: 1,
        entries: 1,
    };
    capacities.membership.groups = 1;
    capacities.membership.roots = 1;
    capacities.membership.chunks = 2;
    let mut packet = super::StatusPacket::with_capacities(capacities).unwrap();
    let allocated = packet.allocated_bytes();
    assert_eq!(
        packet.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }, 1),
        Outcome::Rows { len: 1 }
    );
    assert!(packet.allocated_bytes() > allocated);
    assert_eq!(packet.required_capacities().membership.chunks, 4);
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(packet.project(&mut rows), Outcome::Rows { len: 1 });
    let mut expected = [super::StatusRow::default(); 1];
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut expected,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows, expected);
    assert_eq!(rows[0].has_order, 1);
    let allocated = packet.allocated_bytes();
    packet
        .grow(
            1,
            crate::corders_group_member::MembershipCapacity::default(),
        )
        .unwrap();
    capacities.membership.chunks = 4;
    assert_eq!(packet.capacities, capacities);
    assert!(packet.allocated_bytes() >= allocated);
}

#[test]
fn status_packet_preserves_raw_maxima_and_its_independent_source_prefix() {
    let mut memory = presentation_fixture();
    let source = presentation_source()[0];
    p64(
        &mut memory.bytes,
        0x11000 + crate::csub_unit_definition::MAXIMUM_STRENGTH,
        99,
    );
    let mut seeds = [super::DivisionSeed {
        pointer: source.pointer,
        division: source.division,
        ..super::DivisionSeed::default()
    }; 2];
    super::capture_division_seeds(&memory, &mut seeds);
    let mut first = super::StatusPacket::with_capacities(super::StatusCapacities {
        modifiers: crate::modifier_accumulator::SnapshotCapacity {
            nodes: 1,
            entries: 1,
        },
        ..super::StatusCapacities::new(2)
    })
    .unwrap();
    assert_eq!(
        first.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 1),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(first.maxima[0].strength, 99);
    let mut first_rows = [super::StatusRow::default(); 1];
    assert_eq!(first.project(&mut first_rows), first.outcome());
    p64(
        &mut memory.bytes,
        0x11000 + crate::csub_unit_definition::MAXIMUM_STRENGTH,
        50,
    );
    let mut second = super::StatusPacket::with_capacities(super::StatusCapacities {
        modifiers: crate::modifier_accumulator::SnapshotCapacity {
            nodes: 1,
            entries: 1,
        },
        ..super::StatusCapacities::new(2)
    })
    .unwrap();
    assert_eq!(
        second.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 1),
        first.outcome()
    );
    let mut second_rows = [super::StatusRow::default(); 1];
    second.project(&mut second_rows);
    assert_eq!(
        first_rows, second_rows,
        "both raw maxima clamp to the native minimum"
    );
    assert_eq!(second.maxima[0].strength, 50);
    let mut projected_again = [super::StatusRow::default(); 1];
    first.project(&mut projected_again);
    assert_eq!(
        projected_again, first_rows,
        "projection has only owned inputs"
    );
}

#[test]
fn division_status_supply_grace_preserves_signed_words_and_owned_seed_failures() {
    let mut memory = presentation_fixture();
    let source = presentation_source()[0];
    let maximum = 9_007_199_254_740_993_i64;
    let remaining = -1_234_567_i64;
    p64(
        &mut memory.bytes,
        A1 + carmy::MAXIMUM_SUPPLY_GRACE,
        maximum as u64,
    );
    p64(
        &mut memory.bytes,
        A1 + carmy::REMAINING_SUPPLY_GRACE,
        remaining as u64,
    );
    p64(
        &mut memory.bytes,
        A1 + crate::carmy_combat_inputs::NAME_GROUP_MEMBER,
        0x12345,
    );
    let mut seeds = [super::DivisionSeed {
        pointer: source.pointer,
        division: source.division,
        ..Default::default()
    }];
    super::capture_division_seeds(&memory, &mut seeds);
    assert_eq!(seeds[0].name_member, Some(0x12345));
    assert_eq!(seeds[0].supply_grace_maximum, Some(maximum));
    assert_eq!(seeds[0].supply_grace_remaining, Some(remaining));
    // Projection uses the captured words even when live memory changes.
    p64(&mut memory.bytes, A1 + carmy::MAXIMUM_SUPPLY_GRACE, 0);
    p64(&mut memory.bytes, A1 + carmy::REMAINING_SUPPLY_GRACE, 0);
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        packet.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }, 1),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(packet.project(&mut rows), Outcome::Rows { len: 1 });
    assert_eq!(
        (rows[0].supply_grace_maximum, rows[0].supply_grace_remaining),
        (maximum, remaining)
    );
    for (maximum_failed, reason) in [
        (true, "division maximum supply grace unreadable"),
        (false, "division remaining supply grace unreadable"),
    ] {
        let mut failed = seeds;
        if maximum_failed {
            failed[0].supply_grace_maximum = None;
        } else {
            failed[0].supply_grace_remaining = None;
        }
        assert_eq!(
            packet.capture(&memory, 0, GS, &failed, Outcome::Rows { len: 1 }, 1),
            Outcome::Unavailable { reason }
        );
    }
}

#[test]
fn division_status_invalid_order_reason_is_lazy_raw_and_field_local() {
    let mut memory = presentation_fixture();
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::StatusRow::default(); 1];
    let reason_address = A1 + cunit::INVALID_ORDERS_REASON;
    for raw in [i32::MIN, -1, 0, 1, 7, 8, i32::MAX] {
        memory.bytes[A1 + carmy::INVALID_ORDERS] = 0xff;
        p32(&mut memory.bytes, reason_address, raw as u32);
        let mut seeds = [super::DivisionSeed {
            pointer: A1,
            division: presentation_source()[0].division,
            ..Default::default()
        }];
        super::capture_division_seeds(&memory, &mut seeds);
        let observed = ScalarBatchMemory::new(&memory);
        assert_eq!(
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }, 1),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            observed
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == reason_address)
                .count(),
            1
        );
        drop(observed);
        p32(&mut memory.bytes, reason_address, 42);
        assert_eq!(packet.project(&mut rows), Outcome::Rows { len: 1 });
        assert_eq!(rows[0].invalid_orders_reason, Some(raw));

        let observed = ScalarBatchMemory::new(&memory);
        observed.fault.set(Some(reason_address));
        assert_eq!(
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }, 1),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(packet.project(&mut rows), Outcome::Rows { len: 1 });
        assert_eq!(rows[0].invalid_orders_reason, None);
        assert_eq!(rows[0].invalid_orders, 0xff);
        assert_eq!(rows[0].entrenchment, i64::MIN);
        assert_eq!(rows[0].maximum_hit_points, 200_000);

        seeds[0].invalid_orders = Some(0);
        observed.reads.borrow_mut().clear();
        assert_eq!(
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }, 1),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(packet.project(&mut rows), Outcome::Rows { len: 1 });
        assert_eq!(rows[0].invalid_orders_reason, None);
        assert!(
            observed
                .reads
                .borrow()
                .iter()
                .all(|&(at, _)| at != reason_address)
        );
    }
}

#[test]
fn exercise_level_gates_honor_ai_stop_at_maximum_and_wrapping_manpower() {
    let mut memory = presentation_fixture();
    memory.bytes[G1 + corders_group::TRAINING] = 1;
    array(&mut memory.bytes, A1 + carmy::MANPOWER_DATA, 0x16000, 2);
    p32(&mut memory.bytes, 0x16004, 40);
    p32(&mut memory.bytes, 0x1600c, 60);
    p64(&mut memory.bytes, A1 + carmy::EXPERIENCE, 20_000); // Exactly level2 threshold.
    assert_eq!(super::exercise_level(&memory, 0, A1), Ok(2));
    assert_eq!(super::exercising(&memory, 0, GS, A1, G1, 200_000), Ok(true));
    memory.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 1;
    assert_eq!(
        super::exercising(&memory, 0, GS, A1, G1, 200_000),
        Ok(false)
    );
    memory.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 0;
    memory.bytes[0x15011] = 1; // Owner3 aliases canonical1.
    assert_eq!(
        super::exercising(&memory, 0, GS, A1, G1, 200_000),
        Ok(false)
    );
    memory.bytes[0x15001] = 1; // A human controller disables the AI level gate.
    assert_eq!(super::exercising(&memory, 0, GS, A1, G1, 200_000), Ok(true));
    p32(&mut memory.bytes, 0x16004, i32::MAX as u32);
    p32(&mut memory.bytes, 0x1600c, 1); // Sum wraps negative, selects zero experience.
    assert_eq!(super::exercise_level(&memory, 0, A1), Ok(0));
}

#[test]
fn attrition_counter_uses_effective_country_modifier_and_rejects_unknown_feature_gates() {
    use crate::{modifier_definition as metadata, modifier_entry};
    let mut memory = presentation_fixture();
    array(
        &mut memory.bytes,
        C1 + ccountry::MODIFIER_ACCUMULATOR,
        0x18000,
        1,
    );
    p32(&mut memory.bytes, 0x18000, 0x54);
    p64(
        &mut memory.bytes,
        0x18000 + modifier_entry::VALUE,
        1_000_000,
    );
    let entry = 0x0100_0000 + 0x54 * metadata::STRIDE;
    memory
        .globals
        .insert(metadata::TABLE_RVA, 0x0100_0000_u64.to_le_bytes().to_vec());
    memory.globals.insert(
        entry + metadata::FLAGS,
        metadata::CLAMP_MAXIMUM.to_le_bytes().to_vec(),
    );
    memory.globals.insert(
        entry + metadata::FEATURE_SELECTOR,
        0_u32.to_le_bytes().to_vec(),
    );
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].attrition_threshold, rows[0].attrition), (1000, 0));
    memory
        .globals
        .insert(entry + metadata::FLAGS, 0_u32.to_le_bytes().to_vec());
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].attrition_threshold, 5500);
    p64(
        &mut memory.bytes,
        0x18000 + modifier_entry::VALUE,
        -200_000_i64 as u64,
    );
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].attrition_threshold, 0);
    memory.globals.insert(
        entry + metadata::FEATURE_SELECTOR,
        1_u32.to_le_bytes().to_vec(),
    );
    assert_eq!(
        super::walk_status(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows
        ),
        Outcome::Unavailable {
            reason: "attrition modifier needs unproved feature callback"
        }
    );
}

#[test]
fn presentation_supporting_reads_authenticate_maxima_country_sets_and_player_fallback() {
    use crate::{cgame_state, chuman, cland_combatant, csub_unit_definition as stats};
    let mut memory = presentation_fixture();
    p64(
        &mut memory.bytes,
        0x11000 + stats::MAXIMUM_STRENGTH,
        i64::MIN as u64,
    );
    p64(&mut memory.bytes, 0x11000 + stats::MAXIMUM_ORGANISATION, 99);
    assert_eq!(stats::division_maxima(&memory, 0, 0x11000), Ok((100, 100)));
    assert!(stats::division_maxima(&memory, 0, 0).is_err());
    assert!(stats::division_maxima(&memory, usize::MAX, 0x11000).is_err());
    p64(&mut memory.bytes, 0x11000, 0);
    assert!(stats::division_maxima(&memory, 0, 0x11000).is_err());
    p64(
        &mut memory.bytes,
        0x17000 + cland_combatant::COUNTRIES,
        0x18000,
    );
    p32(&mut memory.bytes, 0x18000, 3);
    assert_eq!(
        cland_combatant::contains_country(&memory, 0, GS, 0x17000, 1),
        Ok(true)
    );
    assert_eq!(
        cland_combatant::contains_country(&memory, 0, GS, 0x17000, 2),
        Ok(false)
    );
    p64(&mut memory.bytes, 0x18010, 0x18000);
    assert!(cland_combatant::contains_country(&memory, 0, GS, 0x17000, 2).is_err());
    p32(&mut memory.bytes, 0x18000, 12345);
    assert!(cland_combatant::contains_country(&memory, 0, GS, 0x17000, 2).is_err());
    array(&mut memory.bytes, GS + cgame_state::HUMAN_STATUS, 0, 0);
    array(&mut memory.bytes, GS + 0xf8, 0x19000, 1);
    p64(&mut memory.bytes, 0x19000, chuman::VTABLE_RVA as u64);
    p32(&mut memory.bytes, 0x19000 + chuman::COUNTRY_TAG, 3);
    assert_eq!(
        cgame_state::autonomous_country(&memory, 0, GS, 1),
        Ok(false)
    );
    assert_eq!(cgame_state::autonomous_country(&memory, 0, GS, 2), Ok(true));
    p64(&mut memory.bytes, 0x19000, 0);
    assert!(cgame_state::autonomous_country(&memory, 0, GS, 1).is_err());
}

#[test]
fn movement_rejects_a_null_next_province() {
    let mut memory = presentation_fixture();
    let mut rows = [super::MovementRow::default(); 1];
    array(
        &mut memory.bytes,
        A1 + crate::cunit::ORDINARY_ROUTE_DATA,
        0x16000,
        1,
    );
    assert_eq!(
        super::walk_movement(
            &memory,
            0,
            GS,
            &presentation_source(),
            Outcome::Rows { len: 1 },
            &mut rows,
            &mut [0]
        )
        .outcome,
        Outcome::Unavailable {
            reason: "nonempty movement route has null next province"
        }
    );
}

#[test]
fn division_seed_descriptors_preserve_original_prefix_and_append_names_and_grace() {
    use super::*;
    assert_eq!(DIVISION_SEED_STRIDE, 336);
    assert_eq!(
        DIVISION_SEED_FIELDS.map(|field| (
            field.source_offset,
            field.destination_offset,
            field.len
        )),
        [
            (0, 0, 8),
            (SECONDARY_BASE, 8, 8),
            (cunit::HANDLE, 16, 8),
            (cunit::ROSTER_OWNER, 24, 4),
            (cunit::LOGICAL_OWNER, 28, 4),
            (cunit::EXPEDITIONARY_OWNER, 32, 4),
            (cunit::LOCATION, 36, 8),
            (STANDARD_TEMPLATE, 44, 8),
            (STRENGTH, 52, 8),
            (ORGANISATION, 60, 8),
            (ORDERS_GROUP, 68, 8),
            (cunit::CLASS, 76, 4),
            (cunit::STAT_BLOCK, 80, 8),
            (cunit::ORDINARY_ROUTE_DATA, 88, 16),
            (cunit::ALTERNATE_ROUTE_DATA, 104, 16),
            (cunit::COMBAT_REFERENCE_DATA, 120, 16),
            (MOVEMENT_NUMERATOR, 136, 8),
            (MOVEMENT_TARGET, 144, 8),
            (MOVEMENT_SOURCE, 152, 8),
            (NAVAL_TRANSFER_MEMBERSHIP, 160, 8),
            (RETREAT, 168, 1),
            (STRATEGIC_REDEPLOYMENT, 169, 1),
            (COUNTER_ATTRITION, 170, 8),
            (ENTRENCHMENT, 178, 8),
            (ENTRENCHMENT_CAP, 186, 8),
            (COUNTER_SUPPLY_STATUS, 194, 8),
            (INVALID_ORDERS, 202, 1),
            (STRENGTH_FACTOR, 203, 8),
            (RETAINED_VISUAL_VARIANT, 211, 4),
            (ACCLIMATIZATION, 215, 0x40),
            (cunit::MAP_ENTITY, 279, 8),
            (EXILED, 287, 1),
            (crate::carmy_combat_inputs::NAME_GROUP_MEMBER, 288, 8),
            (MAXIMUM_SUPPLY_GRACE, 296, 8),
            (REMAINING_SUPPLY_GRACE, 304, 8),
            (MANPOWER_VALUES, 312, 24),
        ]
    );
}

#[test]
fn packet_capacity_failure_preserves_existing_storage() {
    assert_eq!(
        super::StatusPacket::new(usize::MAX).err().unwrap().kind(),
        std::io::ErrorKind::OutOfMemory,
    );
    let mut packet = super::EtaInputPacket::new(1, 2, 3).unwrap();
    let capacity = packet.allocated_bytes();
    assert_eq!(
        packet.reserve(usize::MAX, 2, 3).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory,
    );
    assert_eq!(packet.allocated_bytes(), capacity);
    assert!(packet.rows().is_empty());
    assert_eq!(super::EtaInputPacket::vacant().allocated_bytes(), 0);
    assert_eq!(super::StatusPacket::vacant().allocated_bytes(), 0);
}

#[test]
fn eta_route_staging_preserves_the_previous_capacity_trace() {
    let mut previous = Vec::<u32>::with_capacity(3);
    let mut packet = super::EtaInputPacket::new(1, 1, 3).unwrap();
    assert_eq!(packet.route.capacity(), previous.capacity());
    previous.push(1);
    packet.route.push(1);
    for legs in [1, 7, 5, 19] {
        if previous.capacity() < legs {
            previous.reserve_exact(legs - previous.len());
        }
        packet.reserve(1, 1, legs).unwrap();
        assert_eq!(packet.route.capacity(), previous.capacity());
    }
}

#[test]
fn eta_route_staging_uses_the_admitted_arena_capacity() {
    let memory = normal_route_fixture();
    let seed = speed_seed(&memory);
    let mut geometry = cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
        provinces: 8,
        neighbour_entries: 8,
    });
    geometry.begin(&memory, 0, GS).unwrap();
    let mut statics = cprovince::StaticGeometry::default();
    let mut packet = super::EtaInputPacket::new(1, 16, 2).unwrap();
    let route_pointer = packet.route.as_ptr();
    let route_capacity = packet.route.capacity();
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &[seed],
        0..1,
        Outcome::Rows { len: 1 },
        1,
        16,
        2,
        &mut geometry,
        &mut statics,
        &mut packet,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(packet.route.as_ptr(), route_pointer);
    assert_eq!(packet.route.capacity(), route_capacity);
    let walk = super::capture_eta_inputs(
        &memory,
        0,
        GS,
        &[seed],
        0..1,
        Outcome::Rows { len: 1 },
        1,
        16,
        1,
        &mut geometry,
        &mut statics,
        &mut packet,
    );
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(packet.route.as_ptr(), route_pointer);
    assert_eq!(packet.route.capacity(), route_capacity);
}

#[test]
fn strategic_action_owned_movement_matches_direct_and_withholds_only_the_glyph() {
    use crate::{cstate, cunit, modifier_definition, modifier_keys as keys};
    const STATE: usize = 0x1a000;
    const BUCKET: usize = 0x1b000;
    const VALUES: usize = 0x1c000;
    const DEFINITIONS: usize = 0x21000;
    let mut memory = shared_geometry_fixture();
    p64(&mut memory.bytes, A1 + cunit::MOVEMENT_TARGET, 0);
    p32(&mut memory.bytes, A1 + cunit::EXPEDITIONARY_OWNER, 2);
    memory.bytes[A1 + cunit::RETREAT] = 0;
    memory.bytes[A1 + cunit::STRATEGIC_REDEPLOYMENT] = 1;
    p64(&mut memory.bytes, P1 + cprovince::STATE, STATE as u64);
    p64(&mut memory.bytes, STATE, cstate::VTABLE_RVA as u64);
    p32(&mut memory.bytes, STATE + cstate::CONTROLLER, 2);
    p64(
        &mut memory.bytes,
        STATE + cstate::COUNTRY_MODIFIERS,
        BUCKET as u64,
    );
    array(&mut memory.bytes, STATE + cstate::BASE_MODIFIERS, VALUES, 2);
    p32(
        &mut memory.bytes,
        VALUES,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT,
    );
    p32(
        &mut memory.bytes,
        VALUES + 16,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER,
    );
    p64(&mut memory.bytes, VALUES + 24, 17);
    memory.globals.insert(
        modifier_definition::TABLE_RVA,
        (DEFINITIONS as u64).to_le_bytes().to_vec(),
    );
    memory.globals.insert(
        DEFINITIONS,
        vec![
            0;
            (keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER as usize + 1)
                * modifier_definition::STRIDE
        ],
    );
    let sources = presentation_source();
    let run = |fault| {
        let observed = ScalarBatchMemory::new(&memory);
        observed.fault.set(fault);
        let mut expected = [super::MovementRow::default(); 1];
        let mut actual = expected;
        let mut expected_paths = [0; 2];
        let mut actual_paths = expected_paths;
        let direct = super::walk_movement(
            &observed,
            0,
            GS,
            &sources,
            Outcome::Rows { len: 1 },
            &mut expected,
            &mut expected_paths,
        );
        let owned = owned_movement_fixture_run(&observed, &sources, &mut actual, &mut actual_paths);
        assert_eq!(
            (direct, expected, expected_paths),
            (owned, actual, actual_paths)
        );
        (owned.outcome, actual[0])
    };
    let (outcome, row) = run(None);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        row.strategic_action,
        super::StrategicAction::Available(super::StrategicRedeploymentInputs {
            country_modifier: 0,
            state_modifier: Some(0),
            friendly_state_modifier: Some(17),
        })
    );
    assert_eq!(run(Some(VALUES + 24)).0, Outcome::Rows { len: 1 });
    let (_, withheld) = run(Some(VALUES + 24));
    assert_eq!(
        withheld.strategic_action,
        super::StrategicAction::Withheld(super::StrategicActionFailure::FriendlyStateModifier)
    );
    assert_eq!(
        (
            withheld.source,
            withheld.target,
            withheld.path_total,
            withheld.strategic_redeployment
        ),
        (
            row.source,
            row.target,
            row.path_total,
            row.strategic_redeployment
        )
    );
}

mod unit_combat_reference_tests;
