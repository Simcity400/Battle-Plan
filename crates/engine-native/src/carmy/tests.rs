#[test]
fn experience_pending_conversion_reads_full_template_handle_and_null_absence() {
    use super::*;
    let mut bytes = fixture();
    let run =
        |bytes: &[u8]| pending_template(&crate::memory::SliceMemory { base: 0, bytes }, 0, A1);
    p64(&mut bytes, A1 + PREVIOUS_TEMPLATE, 0);
    assert_eq!(run(&bytes), Ok(0));
    p64(&mut bytes, A1 + PREVIOUS_TEMPLATE, T1 as u64);
    p64(&mut bytes, T1 + 8, handle(u32::MAX, u32::MAX));
    assert_eq!(run(&bytes), Ok(u64::MAX));
    p64(&mut bytes, T1, 0);
    assert_eq!(run(&bytes), Err("division template vtable mismatch"));
}

#[test]
fn icon_inputs_keep_raw_byte_and_full_alternate_template_handle() {
    use super::*;
    let mut bytes = fixture();
    let source = crate::cequipment_variant::pool::DivisionSource {
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
    let source = crate::cequipment_variant::pool::DivisionSource {
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
    let source = crate::cequipment_variant::pool::DivisionSource {
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
    let source = crate::cequipment_variant::pool::DivisionSource {
        pointer: A1,
        division: handle(9, 9000),
        country: 25,
        template: handle(17, 52),
    };
    p32(&mut bytes, A1 + cunit::ROSTER_OWNER, source.country);
    p64(&mut bytes, A1 + STANDARD_TEMPLATE, T1 as u64);
    p64(&mut bytes, T1 + 8, source.template);
    bytes[A1 + SEPARATE_COUNTER] = 255;
    let seed_of = |bytes: &[u8]| {
        let mut seeds = [DivisionSeed {
            pointer: A1,
            division: source.division,
            ..Default::default()
        }];
        capture_division_seeds(&crate::memory::SliceMemory { base: 0, bytes }, &mut seeds);
        seeds[0]
    };
    const ALTERNATE: usize = T1 + 0x100;
    p64(
        &mut bytes,
        ALTERNATE,
        creferenced_division_template::VTABLE_RVA.0 as u64,
    );
    p64(&mut bytes, ALTERNATE + 8, handle(23, 52));
    for (pointer, calls_saved) in [(0, 10), (T1, 12), (ALTERNATE, 10)] {
        p64(&mut bytes, A1 + FAKE_INTEL_TEMPLATE, pointer as u64);
        let seed = seed_of(&bytes);
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
    let seed = seed_of(&bytes);
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let owned = crate::read_census::ReadCensus::new(&memory);
    assert_eq!(
        read_icon_inputs(
            &owned,
            0,
            crate::cequipment_variant::pool::DivisionSource {
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
    let source = crate::cequipment_variant::pool::DivisionSource {
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
        p64(&mut b, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut b, country + 8, id);
        array(&mut b, country + 0x290, data, count);
    }
    for (slot, ptr) in [A1 as u64, 0, A2 as u64].into_iter().enumerate() {
        p64(&mut b, FIRST + slot * 8, ptr);
    }
    p64(&mut b, SECOND, 0);
    p64(&mut b, SECOND + 8, A3 as u64);
    p64(&mut b, PROVINCES, u64::MAX);
    p64(&mut b, PROVINCES + 8, P1 as u64);
    p64(&mut b, P1, cprovince::VTABLE_RVA.0 as u64);
    p32(&mut b, P1 + 0xa4, 1);
    p64(
        &mut b,
        T1,
        creferenced_division_template::VTABLE_RVA.0 as u64,
    );
    p64(&mut b, T1 + 8, handle(0, 52));
    p64(&mut b, G1, corders_group::VTABLE_RVA.0 as u64);
    p64(&mut b, G1 + 8, handle(57, corders_group::REFERENCE_TYPE));
    for (at, id, owner, group) in [
        (A1, handle(9, 9000), 3, G1),
        (A2, handle(9, 51), 3, G1),
        (A3, handle(1, 0), 2, 0),
    ] {
        p64(&mut b, at, carmy::VTABLE_RVA.0 as u64);
        p64(&mut b, at + 0x10, carmy::SECONDARY_VTABLE_RVA.0 as u64);
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
        theatre: Some(0),
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
}

#[test]
fn orders_group_parent_is_authenticated_and_a_null_parent_is_not_a_failure() {
    let mut b = fixture();
    p64(&mut b, G1, carmy_group::VTABLE_RVA.0 as u64);
    let mut out = [Row::default(); 5];
    assert_eq!(run(&b, &mut out), Outcome::Rows { len: 3 });
    assert_eq!(
        out[0].orders_group,
        handle(57, corders_group::REFERENCE_TYPE)
    );
    p64(&mut b, G1, cprovince::VTABLE_RVA.0 as u64);
    assert_eq!(
        run(&b, &mut out),
        Outcome::Unavailable {
            reason: "division orders group vtable mismatch"
        }
    );
    p64(&mut b, G1, corders_group::VTABLE_RVA.0 as u64);
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
    assert_eq!(count, 1);
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
            theatre: Some(0),
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
    p64(&mut b, A2, 0);
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
    p64(&mut b, C2 + 0x290, 0xffff_0000);
    assert!(matches!(run(&b, &mut out), Outcome::Unavailable { .. }));
}

#[test]
fn invalid_identity_owner_and_array_structure_fail_the_whole_table() {
    let cases: [(usize, u64, bool); 15] = [
        (A1, 0, true),
        (A1 + 0x18, 0, true),
        (A1 + 0x1d8, 2, false),
        (C1, 0, true),
        (C1 + 8, 0, false),
        (LINKS + 3 * 4, 2, false),
        (COUNTRIES + 8, C2 as u64, true),
        (C1 + 0x29c, 4, false),
        (A1 + 0x1f0, u64::MAX, true),
        (P1, 0, true),
        (P1 + 0xa4, 0, false),
        (PROVINCES + 8, P1 as u64 + 0x100, true),
        (A1 + 0x3b8, u64::MAX, true),
        (T1, 0, true),
        (T1 + 8, 0, true),
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
    assert_eq!(packed.batches.get(), 4);
    for (offset, width) in [
        (crate::cunit_supply_inputs::ASKED, 8),
        (crate::cunit_supply_inputs::RECEIVED, 16),
        (crate::cunit_supply_inputs::ORIGINS, 16),
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
            "supply-footer span {offset:#x} is acquired once"
        );
    }
    assert_eq!(
        actual[0].supply.map(|supply| supply.origins_len),
        Some(Ok(0))
    );
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
                && address != carmy::EXERCISE_MAXIMUM_LEVEL_RVA.0
        }),
        "group authentication excludes private training and exercise dependencies"
    );
    assert_eq!(rows, [super::StatusRow::default(); 1]);
}

fn presentation_fixture() -> PresentationMemory {
    use crate::{
        cgame_state,
        csub_unit_definition as stats,
        cunit,
        cprovince::definition as definition,
    };
    let mut bytes = fixture();
    bytes.resize(0x20000, 0);
    for army in [A1, A2, A3] {
        p64(
            &mut bytes,
            army + carmy::ACCLIMATIZATION,
            crate::acclimatization_data::VTABLE_RVA.0 as u64,
        );
    }
    p64(&mut bytes, A1 + cunit::STAT_BLOCK, 0x11000);
    p64(&mut bytes, 0x11000, stats::VTABLE_RVA.0 as u64);
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
                carmy::COUNTER_ATTRITION_THRESHOLD_RVA.0,
                500_i64.to_le_bytes().to_vec(),
            ),
            (
                carmy::EXERCISE_MAXIMUM_LEVEL_RVA.0,
                2_i32.to_le_bytes().to_vec(),
            ),
            (
                carmy::EXERCISE_MINIMUM_STRENGTH_RVA.0,
                50_000_i64.to_le_bytes().to_vec(),
            ),
            (carmy::EXPERIENCE_THRESHOLDS_RVA.0, thresholds_header),
        ]
        .into(),
    }
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
        p64(b, terrain, 0x02a3_b470);
        p32(b, terrain + 8, token);
        p32(b, terrain + 0x88, index);
        p64(b, terrain + 0x168, 0x0280_75d8);
        p64(b, terrain + 0x188, (-2_345_i64) as u64);
        p64(b, entry, 0x0280_75d8);
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
        p64(b, province, cprovince::VTABLE_RVA.0 as u64);
        p32(b, province + cprovince::ID, id);
        p64(b, province + cprovince::DEFINITION, definition as u64);
        p32(b, definition + crate::cprovince::definition::ID, id);
        p64(b, definition + 0xa8, terrain as u64);
        b[definition + crate::cprovince::definition::TERRAIN_FLAGS] = 1;
    }
    array(b, A1 + crate::cunit::ORDINARY_ROUTE_DATA, 0x1f000, 2);
    p32(b, 0x1f000, 2);
    p32(b, 0x1f004, 3);
    p64(b, GS + crate::cgame_state::WEATHER_MANAGER, 0x19000);
    p64(b, 0x19000, crate::cweather_manager::VTABLE_RVA.0 as u64);
    p64(
        b,
        T1 + creferenced_division_template::PRIORITY_UNIT,
        0x1a000,
    );
    p64(b, 0x1a000, crate::csub_unit_definition::VTABLE_RVA.0 as u64);
    p64(b, C1 + ccountry::DIPLOMACY_STATUS, 0x1b000);
    array(b, C1 + ccountry::MODIFIER_ACCUMULATOR, 0x1c000, 1);
    p32(b, 0x1c000, 0x2b);
    p64(b, 0x1c008, 10_001);
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA.0,
        0x1d000_u64.to_le_bytes().to_vec(),
    );
    memory.globals.insert(
        crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA.0,
        7_i64.to_le_bytes().to_vec(),
    );
    for (rva, value) in [
        (0x033e_0898, 50_000_i64),
        (0x033e_3e60, 100_000),
        (0x033e_4690, 25_000),
        (0x033e_10c8, 100_003),
        (0x033e_3c20, 25_003),
        (0x033e_2cc0, 33_333),
    ] {
        memory.globals.insert(rva, value.to_le_bytes().to_vec());
    }
    memory
}

#[test]
fn state_movement_modifier_keeps_raw_values_for_aliased_owners_and_fails_each_read() {
    let mut source = normal_route_fixture();
    let state = 0x28000;
    p64(&mut source.bytes, state, crate::cstate::VTABLE_RVA.0 as u64);
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
        assert!(!direct.is_empty());
        for &(fault, _) in &direct {
            observed.fault.set(Some(fault));
            assert!(crate::cstate::movement_modifier(&observed, 0, GS, state, owner, key).is_err());
        }
        observed.fault.set(None);
    }
}

#[test]
fn the_static_map_memo_reuses_provinces_until_the_session_changes() {
    let mut memory = normal_route_fixture();
    let mut statics = cprovince::StaticGeometry::default();
    assert!(statics.begin(&memory, 0, GS));
    let cold: Vec<_> = (1..=3)
        .map(|id| statics.province(&memory, 0, id).unwrap())
        .collect();
    assert_eq!(
        cold.iter()
            .map(|province| (province.pointer, province.definition, province.terrain))
            .collect::<Vec<_>>(),
        [
            (P1, 0x12000, 0x17000),
            (0x18200, 0x18600, 0x17000),
            (0x18400, 0x18700, 0x17400)
        ]
    );
    assert_eq!(statics.province(&memory, 0, 1), Ok(cold[0]));
    assert_eq!(
        statics.counters(),
        cprovince::StaticCounters {
            hits: 1,
            misses: 3,
            cleared: 1,
        }
    );
    assert!(statics.begin(&memory, 0, GS));
    let warm = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: None,
    };
    for id in 1..=3 {
        assert_eq!(statics.province(&warm, 0, id), Ok(cold[id as usize - 1]));
    }
    assert!(warm.reads.borrow().is_empty());
    assert_eq!(
        statics.counters(),
        cprovince::StaticCounters {
            hits: 3,
            misses: 0,
            cleared: 0,
        }
    );
    memory.globals.insert(
        crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA.0,
        8_i64.to_le_bytes().to_vec(),
    );
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.counters().cleared, 1);
    assert_eq!(statics.province(&memory, 0, 1), Ok(cold[0]));
    assert_eq!(statics.counters().misses, 1);
}

#[test]
fn embark_define_refreshes_without_a_session_or_province_census_change() {
    let mut memory = normal_route_fixture();
    let define = crate::cunit::EMBARK_MOVEMENT_DISTANCE_RVA.0;
    let mut statics = cprovince::StaticGeometry::default();
    memory
        .globals
        .insert(define, 12_345_i64.to_le_bytes().to_vec());
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.embark(&memory, 0), Ok(12_345));
    let key = memory.globals[&crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA.0].clone();
    memory
        .globals
        .insert(define, 67_890_i64.to_le_bytes().to_vec());
    assert_eq!(statics.embark(&memory, 0), Ok(12_345));
    assert!(statics.begin(&memory, 0, GS));
    assert_eq!(statics.counters().cleared, 0);
    assert_eq!(statics.embark(&memory, 0), Ok(67_890));
    assert_eq!(
        memory.globals[&crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA.0],
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
    let header = 0x12000 + crate::cprovince::definition::NEIGHBOURS;
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

fn presentation_source() -> [crate::cequipment_variant::pool::DivisionSource; 1] {
    [crate::cequipment_variant::pool::DivisionSource {
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
        assert_eq!(cache.read::<u64>(&memory, P1).unwrap(), crate::read(&source, P1));
    }
    assert_eq!(
        memory.reads.borrow().iter().filter(|&&a| a == P1).count(),
        1
    );
    assert_eq!(cache.read::<u32>(&memory, P1).unwrap(), crate::read(&source, P1));
    assert_eq!(cache.read::<u32>(&memory, P1 + cprovince::ID).unwrap(), Some(1));
    assert_eq!(cache.read::<u64>(&memory, 0xdead).unwrap(), None);
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
    assert_eq!(cache.read::<u64>(&memory, 0xdead).unwrap(), None);
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
    assert_eq!(empty.read::<u64>(&memory, P1).unwrap(), crate::read(&source, P1));
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
        0x12000 + crate::cprovince::definition::NEIGHBOURS,
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
    sources: &[crate::cequipment_variant::pool::DivisionSource],
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
    let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
        crate::cprovince::GeometryCapacities::default(),
    );
    geometry.begin(memory, 0, GS).unwrap();
    let mut packet = super::MovementPacket::new(sources.len(), paths.len());
    let allocated = packet.allocated_bytes();
    super::capture_movement_inputs(
        memory,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: sources.len() },
        &mut geometry,
        &mut packet,
    );
    let result = super::project_movement(&packet, output, paths);
    assert_eq!(packet.allocated_bytes(), allocated);
    result
}

fn movement_combat_fixture() -> (
    PresentationMemory,
    [crate::cequipment_variant::pool::DivisionSource; 3],
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
        crate::cequipment_variant::pool::DivisionSource {
            pointer,
            division,
            ..presentation_source()[0]
        }
    });
    p64(&mut source.bytes, 0x16000, combat::VTABLE_RVA.0 as u64);
    p64(&mut source.bytes, 0x16000 + combat::ATTACKER, 0x17000);
    p64(&mut source.bytes, 0x16000 + combat::PROVINCE, P1 as u64);
    p64(&mut source.bytes, 0x17000, side::VTABLE_RVA.0 as u64);
    p64(&mut source.bytes, 0x17000 + side::COMBAT, 0x16000);
    p64(&mut source.bytes, 0x17000 + side::COUNTRIES, 0x18000);
    p32(&mut source.bytes, 0x18000, 3);
    (source, sources)
}

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
fn optional_arrow_styles_preserve_all_movement_rows_and_read_once_without_retry() {
    let (mut source, sources) = movement_combat_fixture();
    for (index, pointer) in [A1, A2, A3].into_iter().enumerate() {
        source.bytes[pointer + crate::cunit::WITHDRAWAL] = 0xfe - index as u8;
        p32(
            &mut source.bytes,
            pointer + crate::cunit::MOVEMENT_PRIORITY,
            (i32::MIN + index as i32) as u32,
        );
        p32(
            &mut source.bytes,
            pointer + crate::cunit::QUEUED_ACTION_COUNT,
            1 + index as u32,
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
        run(&healthy, &mut baseline, false).outcome,
        Outcome::Rows { len: 3 }
    );
    let tail = [A1, A2, A3]
        .into_iter()
        .flat_map(|pointer| {
            [
                (pointer + crate::cunit::WITHDRAWAL, 1),
                (pointer + crate::cunit::MOVEMENT_PRIORITY, 4),
                (pointer + carmy::FUEL_CANDIDATE, 8),
                (pointer + crate::cunit::QUEUED_ACTION_COUNT, 4),
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
        if owned {
            for &(address, len) in &tail {
                assert_eq!(
                    memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&read| read == (address, len))
                        .count(),
                    1
                );
            }
            assert!(!memory.reads.borrow().ends_with(&tail));
        } else {
            assert!(memory.reads.borrow().ends_with(&tail));
        }
        for fault in [
            A1 + crate::cunit::WITHDRAWAL,
            A1 + crate::cunit::MOVEMENT_PRIORITY,
            A1 + crate::cunit::QUEUED_ACTION_COUNT,
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
            assert_eq!(
                rows[0].queued_actions,
                (fault != A1 + crate::cunit::QUEUED_ACTION_COUNT).then_some(1)
            );
            assert_eq!(rows[1..], baseline[1..]);
            for (mut actual, mut expected) in rows.into_iter().zip(baseline) {
                actual.withdrawal = None;
                actual.movement_priority = None;
                actual.queued_actions = None;
                expected.withdrawal = None;
                expected.movement_priority = None;
                expected.queued_actions = None;
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn owned_movement_takes_both_arrow_styles_from_the_seed_without_reading_them() {
    let (mut source, sources) = movement_combat_fixture();
    for (index, pointer) in [A1, A2, A3].into_iter().enumerate() {
        source.bytes[pointer + crate::cunit::WITHDRAWAL] = 0x10 + index as u8;
        p32(
            &mut source.bytes,
            pointer + crate::cunit::MOVEMENT_PRIORITY,
            7 + index as u32,
        );
    }
    let mut seeds: Vec<_> = sources
        .iter()
        .map(|source| super::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..super::DivisionSeed::default()
        })
        .collect();
    super::capture_division_seeds(&source, &mut seeds);
    assert_eq!(
        seeds
            .iter()
            .map(|seed| (seed.withdrawal, seed.movement_priority))
            .collect::<Vec<_>>(),
        [
            (Some(0x10), Some(7)),
            (Some(0x11), Some(8)),
            (Some(0x12), Some(9))
        ]
    );
    seeds[1].withdrawal = None;
    seeds[2].movement_priority = Some(-3);
    let memory = ScalarBatchMemory::new(&source);
    let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
        crate::cprovince::GeometryCapacities::default(),
    );
    geometry.begin(&memory, 0, GS).unwrap();
    let mut packet = super::MovementPacket::new(sources.len(), 0);
    super::capture_movement_inputs(
        &memory,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: sources.len() },
        &mut geometry,
        &mut packet,
    );
    let mut rows = [super::MovementRow::default(); 3];
    assert_eq!(
        super::project_movement(&packet, &mut rows, &mut []).outcome,
        Outcome::Rows { len: 3 }
    );
    assert!(!memory.reads.borrow().iter().any(|&(address, _)| {
        [A1, A2, A3].into_iter().any(|pointer| {
            address == pointer + crate::cunit::WITHDRAWAL
                || address == pointer + crate::cunit::MOVEMENT_PRIORITY
        })
    }));
    assert_eq!(
        rows.map(|row| (row.withdrawal, row.movement_priority)),
        [
            (Some(0x10), Some(7)),
            (None, Some(8)),
            (Some(0x12), Some(-3))
        ]
    );
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
                    crate::cland_border_war_combat::VTABLE_RVA.0 as u64,
                );
                p64(
                    &mut source.bytes,
                    0x17000,
                    crate::cland_border_war_combatant::VTABLE_RVA.0 as u64,
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
        for (memory, gathered) in [(&direct, 0), (&owned, 1)] {
            for pointer in [A1, A2, A3] {
                for field in [crate::cunit::WITHDRAWAL, crate::cunit::MOVEMENT_PRIORITY] {
                    assert_eq!(
                        memory
                            .reads
                            .borrow()
                            .iter()
                            .filter(|&&(address, _)| address == pointer + field)
                            .count(),
                        gathered
                    );
                }
            }
        }
    }
}

#[test]
fn owned_movement_projection_matches_direct_stack_branches() {
    for class in [0, 1, 13] {
        let mut memory = shared_geometry_fixture();
        p32(&mut memory.bytes, A1 + crate::cunit::CLASS, class);
        let sources = [presentation_source()[0]; 3];
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
            &mut expected,
            &mut expected_paths,
        );
        let owned = owned_movement_fixture_run(&memory, &sources, &mut actual, &mut actual_paths);
        assert_eq!(
            (owned, actual, actual_paths),
            (direct, expected, expected_paths)
        );
    }
}

#[test]
fn owned_movement_admits_every_route_and_ignores_private_status_fault() {
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
        crate::cequipment_variant::pool::DivisionSource {
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
    let result = owned_movement_fixture_run(&memory, &sources, &mut rows, &mut [0; 3]);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!((result.paths_len, result.required_paths), (3, 3));
    assert_eq!(rows[1].division, handle(10, 9000));
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

#[test]
fn only_moving_divisions_read_their_authenticated_stat_block_base_speed() {
    use crate::csub_unit_definition as stats;
    const STATIONARY: usize = 0x19000;
    const STATS: usize = 0x11000;
    const FOREIGN_STATS: usize = 0x1b000;
    const STATIONARY_STATS: usize = 0x1c000;
    const SPEED: i64 = -412_345;
    let mut source = shared_geometry_fixture();
    p64(
        &mut source.bytes,
        STATS + stats::MAXIMUM_SPEED,
        SPEED as u64,
    );
    let army = source.bytes[A1..A1 + 0x700].to_vec();
    for (at, id, stat) in [
        (A2, 10, FOREIGN_STATS),
        (A3, 11, 0),
        (STATIONARY, 12, STATIONARY_STATS),
    ] {
        source.bytes[at..at + 0x700].copy_from_slice(&army);
        p64(&mut source.bytes, at + cunit::HANDLE, handle(id, 9000));
        p64(&mut source.bytes, at + cunit::MAP_ENTITY, 0);
        p64(&mut source.bytes, at + cunit::STAT_BLOCK, stat as u64);
    }
    p64(
        &mut source.bytes,
        FOREIGN_STATS,
        stats::NULL_VTABLE_RVA.0 as u64,
    );
    p64(&mut source.bytes, FOREIGN_STATS + stats::MAXIMUM_SPEED, 7);
    p64(
        &mut source.bytes,
        STATIONARY_STATS,
        stats::VTABLE_RVA.0 as u64,
    );
    p64(
        &mut source.bytes,
        STATIONARY_STATS + stats::MAXIMUM_SPEED,
        9,
    );
    array(
        &mut source.bytes,
        STATIONARY + cunit::ORDINARY_ROUTE_DATA,
        0,
        0,
    );
    let sources = [(A1, 9), (A2, 10), (A3, 11), (STATIONARY, 12)].map(|(pointer, id)| {
        crate::cequipment_variant::pool::DivisionSource {
            pointer,
            division: handle(id, 9000),
            ..presentation_source()[0]
        }
    });
    for fault in [None, Some(A3 + cunit::STAT_BLOCK)] {
        let memory = ObservedMovementMemory {
            source: &source,
            reads: Default::default(),
            fault,
        };
        let mut seeds = sources.map(|source| super::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..super::DivisionSeed::default()
        });
        super::capture_division_seeds(&memory, &mut seeds);
        assert_eq!(seeds[2].stat_block, fault.is_none().then_some(0));
        let mut geometry =
            cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities::default());
        geometry.begin(&memory, 0, GS).unwrap();
        let mut packet = super::MovementPacket::new(sources.len(), 16);
        super::capture_movement_inputs(
            &memory,
            0,
            GS,
            &seeds,
            Outcome::Rows { len: sources.len() },
            &mut geometry,
            &mut packet,
        );
        let mut rows = [super::MovementRow::default(); 4];
        let mut paths = [0; 16];
        let owned = super::project_movement(&packet, &mut rows, &mut paths);
        assert_eq!(owned.outcome, Outcome::Rows { len: 4 });
        assert_eq!(
            rows.map(|row| (row.path_total, row.base_speed)),
            [(2, Some(SPEED)), (2, None), (2, None), (0, None)]
        );
        assert_eq!(packet.base_speed_refusals(), 2);
        let reads = memory.reads.borrow();
        for at in [STATS, STATS + stats::MAXIMUM_SPEED] {
            assert!(reads.contains(&at));
        }
        assert!(
            reads.iter().all(
                |&at| !(STATIONARY_STATS..=STATIONARY_STATS + stats::MAXIMUM_SPEED).contains(&at)
            ),
            "a stationary division never reads its stat block"
        );
        drop(reads);
        let mut expected = [super::MovementRow::default(); 4];
        let mut expected_paths = [0; 16];
        let direct = super::walk_movement(
            &memory,
            0,
            GS,
            &sources,
            Outcome::Rows { len: sources.len() },
            &mut expected,
            &mut expected_paths,
        );
        assert_eq!((direct, expected, expected_paths), (owned, rows, paths));
    }
}

#[test]
fn presentation_demand_sizes_names_and_origins_without_selected_transport_members() {
    use crate::{carmy_combat_inputs, cname_group_member, cunit_supply_inputs, msvc_string};
    const MEMBER: usize = 0x1d000;
    const ORIGINS: usize = 0x1e000;
    let mut memory = presentation_fixture();
    p64(
        &mut memory.bytes,
        A1 + carmy_combat_inputs::NAME_GROUP_MEMBER,
        MEMBER as u64,
    );
    p64(
        &mut memory.bytes,
        MEMBER,
        cname_group_member::VTABLE_RVA.0 as u64,
    );
    let name = MEMBER + cname_group_member::CACHED_NAME;
    memory.bytes[name..name + 10].copy_from_slice(b"exact name");
    p64(&mut memory.bytes, name + 16, 10);
    p64(&mut memory.bytes, name + 24, 15);
    array(
        &mut memory.bytes,
        A1 + cunit_supply_inputs::ORIGINS,
        ORIGINS,
        2,
    );
    array(
        &mut memory.bytes,
        A3 + cunit_supply_inputs::ORIGINS,
        ORIGINS,
        3,
    );
    let demand = super::presentation_demand(&memory, 0, GS).unwrap();
    assert_eq!(
        (demand.rows, demand.cached_name_text, demand.supply_origins),
        (3, 10, 5)
    );
    assert_eq!(demand.failure, None);
    struct Recording<'a> {
        source: &'a PresentationMemory,
        reads: std::cell::RefCell<Vec<usize>>,
    }
    impl crate::Memory for Recording<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            self.source.copy(address, output)
        }
    }
    let recording = Recording {
        source: &memory,
        reads: Default::default(),
    };
    super::presentation_demand(&recording, 0, GS).unwrap();
    for unit in [A1, A2, A3] {
        assert!(
            recording
                .reads
                .borrow()
                .contains(&(unit + cunit_supply_inputs::ORIGINS))
        );
    }

    struct Changing<'a> {
        source: &'a PresentationMemory,
        header: usize,
        len: usize,
        stable: usize,
        byte: usize,
        reads: std::cell::Cell<usize>,
    }
    impl crate::Memory for Changing<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !self.source.copy(address, output) {
                return false;
            }
            if address == self.header && output.len() == self.len {
                self.reads.set(self.reads.get() + 1);
                if self.reads.get() > self.stable {
                    output[self.byte] ^= 1;
                }
            }
            true
        }
    }
    for (header, len, stable, byte, reason) in [
        (
            name,
            msvc_string::STRIDE,
            1,
            16,
            "division presentation demand string header changed",
        ),
        (
            A1 + cunit_supply_inputs::ORIGINS,
            16,
            2,
            12,
            "division presentation demand selecting header changed",
        ),
    ] {
        let changing = Changing {
            source: &memory,
            header,
            len,
            stable,
            byte,
            reads: std::cell::Cell::new(0),
        };
        assert_eq!(super::presentation_demand(&changing, 0, GS), Err(reason));
        assert!(changing.reads.get() > stable);
    }
}

mod status_batches;

mod organisation_batches;

mod read_ahead;

#[path = "tests/border_index/tests.rs"]
mod border_index;

mod movement_pairs;

#[test]
fn shared_movement_geometry_matches_direct_caps_and_reduces_repeated_guard_reads() {
    let mut source = shared_geometry_fixture();
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
    use crate::{
        cunit,
        cprovince::definition as definition,
        province_neighbour as neighbor,
    };
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
fn division_status_keeps_the_model_number_and_camouflage_and_fails_only_that_field() {
    use crate::{acclimatization_category, acclimatization_data};
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
        3,
    );
    p64(
        &mut memory.bytes,
        object + acclimatization_data::MAXIMUM,
        15_000_000,
    );
    memory.globals.insert(
        acclimatization_data::CAMO_SHOW_AT_RVA.0,
        50_000_i64.to_le_bytes().to_vec(),
    );
    let entry = |memory: &mut PresentationMemory, index: usize, category: usize, weight: i64| {
        p64(&mut memory.bytes, 0x16000 + index * 16, category as u64);
        p64(&mut memory.bytes, 0x16008 + index * 16, weight as u64);
    };
    for (category, token) in [(0x16100_usize, 7_u32), (0x16200, 8)] {
        p32(
            &mut memory.bytes,
            category + acclimatization_category::TOKEN,
            token,
        );
    }
    entry(&mut memory, 0, 0x16100, 9_000_000);
    entry(&mut memory, 1, 0x16200, 8_000_000);
    entry(&mut memory, 2, 0x16200, -1);
    memory.globals.insert(
        acclimatization_category::REGISTRY_RVA.0,
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
    let shown = |category| {
        Some(super::Camouflage {
            category: Some(category),
        })
    };
    let none = Some(super::Camouflage { category: None });
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        capture(&memory, &mut packet, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        (rows[0].model_number, rows[0].camouflage),
        (0x1ff, shown(7))
    );
    let camouflage = |memory: &PresentationMemory, packet: &mut super::StatusPacket| {
        let mut rows = [super::StatusRow::default(); 1];
        assert_eq!(capture(memory, packet, &mut rows), Outcome::Rows { len: 1 });
        assert_eq!(rows[0].model_number, 0x1ff);
        rows[0].camouflage
    };
    entry(&mut memory, 1, 0x16200, 9_000_000);
    assert_eq!(camouflage(&memory, &mut packet), shown(7));
    entry(&mut memory, 1, 0x16200, 9_000_001);
    assert_eq!(camouflage(&memory, &mut packet), shown(8));
    entry(&mut memory, 0, 0x16100, 7_499_999);
    entry(&mut memory, 1, 0x16200, 7_000_000);
    let mut unread = super::StatusPacket::with_capacities(super::StatusCapacities {
        modifiers: crate::modifier_accumulator::SnapshotCapacity {
            nodes: 1,
            entries: 1,
        },
        ..super::StatusCapacities::new(1)
    })
    .unwrap();
    assert_eq!(camouflage(&memory, &mut unread), none);
    entry(&mut memory, 0, 0x16100, 7_500_000);
    assert_eq!(camouflage(&memory, &mut packet), shown(7));
    for weight in [0, -1] {
        entry(&mut memory, 0, 0x16100, weight);
        entry(&mut memory, 1, 0x16200, weight);
        assert_eq!(camouflage(&memory, &mut unread), none);
    }
    entry(&mut memory, 0, 0x16100, 9_000_000);
    entry(&mut memory, 1, 0x16300, 1);
    assert_eq!(camouflage(&memory, &mut packet), shown(7));
    entry(&mut memory, 0, 0, 9_000_000);
    assert_eq!(camouflage(&memory, &mut unread), none);
    entry(&mut memory, 0, 0x16300, 9_000_000);
    assert_eq!(camouflage(&memory, &mut packet), None);
    entry(&mut memory, 0, 0x16100, 9_000_000);
    let define = memory
        .globals
        .remove(&acclimatization_data::CAMO_SHOW_AT_RVA.0)
        .unwrap();
    assert_eq!(camouflage(&memory, &mut packet), None);
    entry(&mut memory, 0, 0x16100, 0);
    entry(&mut memory, 1, 0x16200, 0);
    assert_eq!(camouflage(&memory, &mut packet), none);
    memory
        .globals
        .insert(acclimatization_data::CAMO_SHOW_AT_RVA.0, define);
    entry(&mut memory, 0, 0x16100, 9_000_000);
    assert_eq!(camouflage(&memory, &mut packet), shown(7));
    p64(&mut memory.bytes, object, 0);
    assert_eq!(camouflage(&memory, &mut packet), None);
}

#[test]
fn division_status_camouflage_equals_the_full_weight_decision() {
    use crate::{acclimatization_category, acclimatization_data};
    let mut memory = presentation_fixture();
    memory.bytes.resize(0x40000, 0);
    let object = A1 + carmy::ACCLIMATIZATION;
    memory.globals.insert(
        acclimatization_category::REGISTRY_RVA.0,
        0x30000_u64.to_le_bytes().to_vec(),
    );
    array(&mut memory.bytes, 0x30000, 0x30100, 4);
    for index in 0..4 {
        let category = 0x31000 + index * 0x40;
        p64(&mut memory.bytes, 0x30100 + index * 8, category as u64);
        p32(
            &mut memory.bytes,
            category + acclimatization_category::TOKEN,
            index as u32 + 1,
        );
    }
    let weights = [
        0_i64,
        1,
        -1,
        49_999,
        50_000,
        50_001,
        100_000,
        i64::MAX,
        i64::MIN,
    ];
    let mut seed = 0x9e37_79b9_u64;
    let mut next = |bound: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound as u64) as usize
    };
    for _ in 0..2_000 {
        let count = next(6);
        let maximum = [0_i64, 100_000, 10_000_000, -10_000_000, i64::MAX][next(5)];
        let show_at = [0_i64, 50_000, 100_000, 75_000, -50_000][next(5)];
        memory.globals.insert(
            acclimatization_data::CAMO_SHOW_AT_RVA.0,
            show_at.to_le_bytes().to_vec(),
        );
        array(
            &mut memory.bytes,
            object + acclimatization_data::DATA,
            0x32000,
            count as u32,
        );
        p64(
            &mut memory.bytes,
            object + acclimatization_data::MAXIMUM,
            maximum as u64,
        );
        let mut entries = Vec::new();
        for index in 0..count {
            let category = next(4);
            let weight = weights[next(weights.len())];
            p64(
                &mut memory.bytes,
                0x32000 + index * 16,
                (0x31000 + category * 0x40) as u64,
            );
            p64(&mut memory.bytes, 0x32008 + index * 16, weight as u64);
            entries.push((category as u32 + 1, weight));
        }
        let highest = entries.iter().map(|entry| entry.1).max().filter(|w| *w > 0);
        let expected = highest.and_then(|highest| {
            let first = entries.iter().find(|entry| entry.1 == highest)?;
            (i128::from(highest) >= i128::from(maximum.wrapping_mul(show_at)) / 100_000)
                .then_some(first.0)
        });
        let mut packet = super::StatusPacket::new(1).unwrap();
        let mut rows = [super::StatusRow::default(); 1];
        assert_eq!(
            super::status_test_capture(
                &memory,
                0,
                GS,
                &presentation_source(),
                Outcome::Rows { len: 1 },
                &mut rows,
                &mut packet,
            ),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows[0].camouflage,
            Some(super::Camouflage { category: expected }),
            "{entries:?} maximum {maximum} show_at {show_at}"
        );
    }
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
    p64(b, LEADER, carmy_leader::VTABLE_RVA.0 as u64);
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
    p64(b, OTHER, corders_group::VTABLE_RVA.0 as u64);
    p32(
        b,
        OTHER + corders_group::HANDLE,
        corders_group::REFERENCE_TYPE,
    );
    p64(b, OTHER + corders_group::LEADER, 0x50000);
    p64(b, OTHER + 0x1000, corders_group::VTABLE_RVA.0 as u64);
    p32(
        b,
        OTHER + 0x1000 + corders_group::HANDLE,
        corders_group::REFERENCE_TYPE,
    );
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA.0,
        0x22000_u64.to_le_bytes().to_vec(),
    );
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
        &[],
        &mut batched,
        &crate::modifier_definition::DefinitionMemo::default(),
        &mut |_| None,
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
    struct Counted<'a>(&'a PresentationMemory, std::cell::RefCell<Vec<usize>>);
    impl crate::Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.1.borrow_mut().push(address);
            crate::Memory::copy(self.0, address, destination)
        }
    }
    let known = groups.map(|group| (group == G1).then_some(Ok(LEADER)));
    let counted = Counted(&memory, Default::default());
    let mut shared = [Ok(None); 7];
    carmy_leader::retreat_bans_many(
        &counted,
        0,
        &groups,
        &known,
        &mut shared,
        &crate::modifier_definition::DefinitionMemo::default(),
        &mut |_| None,
    );
    assert_eq!(shared, batched);
    let reads = counted.1.borrow();
    assert!(!reads.contains(&G1));
    assert!(!reads.contains(&(G1 + corders_group::LEADER)));
    assert!(reads.contains(&LEADER));
}

#[test]
fn census_selection_keeps_each_consumers_handle_contract_under_upper_half_faults() {
    use crate::{carmy_leader, corders_group_member as membership, cunit_leader};
    const LEADER: usize = 0x20000;
    let mut memory = presentation_fixture();
    memory.bytes.resize(0x40000, 0);
    let b = &mut memory.bytes;
    p64(b, G1 + corders_group::LEADER, LEADER as u64);
    p64(b, LEADER, carmy_leader::VTABLE_RVA.0 as u64);
    p64(
        b,
        LEADER + cunit_leader::TYPE,
        handle(7, cunit_leader::REFERENCE_TYPE),
    );
    for (block, data) in [
        (carmy_leader::MOVEMENT_MODIFIER, 0x21000),
        (carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER, 0x21100),
    ] {
        array(
            b,
            LEADER + block + carmy_leader::MODIFIER_ACCUMULATOR,
            data,
            1,
        );
        p32(b, data, carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING);
        p64(b, data + 8, 1);
    }
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA.0,
        0x22000_u64.to_le_bytes().to_vec(),
    );
    struct Fault<'a>(&'a PresentationMemory, usize);
    impl crate::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            !(address <= self.1 && address.saturating_add(output.len()) > self.1)
                && crate::Memory::copy(self.0, address, output)
        }
    }
    let seed = carmy::DivisionSeed {
        pointer: A1,
        orders_group: Some(G1),
        class: Some(1),
        ..Default::default()
    };
    let select = |memory: &Fault<'_>| {
        let mut seeds = [seed];
        carmy_leader::capture_selections(memory, 0, &mut seeds, &mut carmy_leader::Fresh).unwrap();
        seeds[0]
    };
    let bans = |memory: &Fault<'_>, known: Option<Result<usize, &'static str>>| {
        let mut output = [Ok(None)];
        carmy_leader::retreat_bans_many(
            memory,
            0,
            &[G1],
            &[known],
            &mut output,
            &crate::modifier_definition::DefinitionMemo::default(),
            &mut |_| None,
        );
        output[0]
    };
    let member = |memory: &Fault<'_>, seed: &carmy::DivisionSeed| {
        let mut packet = membership::MembershipPacket::new(1, 4, 4, 4);
        if super::status_parts::census_authenticated(seed) {
            packet.admit_group(G1).unwrap();
        }
        let query = packet
            .push_query(membership::MembershipQuery {
                member: A1 + cunit::SCHEDULED_MEMBER,
                own_group: G1,
            })
            .unwrap();
        packet.acquire(memory, 0);
        packet.outcome(query)
    };
    let leader_upper = LEADER + cunit_leader::TYPE + 4;
    let group_upper = G1 + corders_group::HANDLE + 4;
    for fault in [usize::MAX, leader_upper, group_upper] {
        let memory = Fault(&memory, fault);
        let selected = select(&memory);
        let census = selected.selected.unwrap();
        assert_eq!(census.leader, Some(Ok(LEADER)), "{fault:#x}");
        assert_eq!(
            census.block,
            carmy_leader::movement_block_memoized(
                &memory,
                0,
                &seed,
                &mut carmy_leader::LeaderMemo::vacant()
            ),
            "{fault:#x}"
        );
        assert!(matches!(census.block, Ok(Some(_))), "{fault:#x}");
        assert_eq!(
            super::counter_dig_in_selection_memoized(
                &memory,
                0,
                &selected,
                &mut super::DigInMemo::vacant()
            ),
            super::counter_dig_in_selection_memoized(
                &memory,
                0,
                &seed,
                &mut super::DigInMemo::vacant()
            ),
            "{fault:#x}"
        );
        let banned = bans(&memory, census.leader);
        assert_eq!(banned, bans(&memory, None), "{fault:#x}");
        assert_eq!(
            banned,
            carmy_leader::retreat_bans(&memory, 0, G1),
            "{fault:#x}"
        );
        assert!(matches!(banned, Ok(Some(_))), "{fault:#x}");
        assert_eq!(
            census.handle,
            crate::read::<u64>(&memory, LEADER + cunit_leader::TYPE),
            "{fault:#x}"
        );
        assert_eq!(census.handle.is_none(), fault == leader_upper);
        assert_eq!(census.group_identity, fault != group_upper);
        assert_eq!(
            member(&memory, &selected),
            member(&memory, &seed),
            "{fault:#x}"
        );
    }
    assert_eq!(
        member(&Fault(&memory, group_upper), &seed),
        Err("orders membership pointer unreadable")
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
        let outcome = packet.retreat_bans(
            memory,
            0,
            seeds,
            Outcome::Rows { len: 1 },
            owner,
            &mut rows,
            &mut |_| None,
        );
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
    p64(&mut memory.bytes, A1 + carmy::AIR_SUPPLY, (-1_i64) as u64);
    assert_eq!(run(&memory), (rows, base_flags));
    p64(&mut memory.bytes, A1 + carmy::AIR_SUPPLY, 1);
    assert_eq!(
        run(&memory),
        (rows, base_flags | super::StatusRow::MOVE_NONCORE_AIR)
    );
    p64(&mut memory.bytes, A1 + carmy::AIR_SUPPLY, 0);
    assert_eq!(bans(&memory, &|_| Ok(Some(3))).0, Outcome::Rows { len: 0 });
    let b = &mut memory.bytes;
    p64(b, G1 + corders_group::LEADER, LEADER as u64);
    p64(b, LEADER, carmy_leader::VTABLE_RVA.0 as u64);
    p32(b, LEADER + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
    for (block, data, movement) in [
        (
            carmy_leader::MOVEMENT_MODIFIER,
            0x21000,
            (crate::modifier_keys::ARMY_SPEED_FACTOR, -10_000_i64),
        ),
        (
            carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER,
            0x21100,
            (crate::modifier_keys::RIVER_CROSSING_FACTOR, 5_000),
        ),
    ] {
        array(b, LEADER + block + 0x10, data, 3);
        p32(b, data, movement.0);
        p64(b, data + 8, movement.1 as u64);
        p32(b, data + 16, carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING);
        p64(b, data + 24, 1);
        p32(b, data + 32, carmy_leader::CANNOT_RETREAT_WHILE_ATTACKING);
        p64(b, data + 40, 0);
    }
    memory.globals.insert(
        crate::modifier_definition::TABLE_RVA.0,
        0x22000_u64.to_le_bytes().to_vec(),
    );
    assert_eq!(run(&memory), (rows, base_flags));
    array(
        &mut memory.bytes,
        A1 + cunit::COMBAT_REFERENCE_DATA,
        0x21800,
        1,
    );
    assert_eq!(run(&memory), (rows, base_flags));
    let (outcome, row) = bans(&memory, &|_| Ok(Some(3)));
    assert_eq!(outcome, rows);
    assert_eq!(
        (row.owner, row.bans),
        (3, carmy_leader::RETREAT_BANNED_DEFENDING)
    );
    let seeds = seeds_of(&memory);
    let mut packet = super::StatusPacket::new(1).unwrap();
    packet.retreat_bans(
        &memory,
        0,
        &seeds,
        Outcome::Rows { len: 1 },
        &|_| Ok(Some(3)),
        &mut [super::RetreatBanRow::default(); 1],
        &mut |_| None,
    );
    let division = seeds[0].division;
    assert_eq!(
        packet.commander_movement(division, super::COMMANDER_BLOCK_ORDINARY),
        Some([Some(-10_000), None, None, None, None, None])
    );
    assert_eq!(
        packet.commander_movement(division, super::COMMANDER_BLOCK_SEPARATE_HQ),
        Some([None, None, None, None, Some(5_000), None])
    );
    assert_eq!(
        packet.commander_movement(division, super::COMMANDER_BLOCK_NONE),
        Some([None; 6])
    );
    assert_eq!(
        packet.commander_movement(division + 1, super::COMMANDER_BLOCK_ORDINARY),
        None
    );
    assert_eq!(bans(&memory, &|_| Ok(None)).0, Outcome::Rows { len: 0 });
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
    let mut seeds = seeds_of(&memory);
    seeds[0].roster_owner = None;
    assert_eq!(
        read(&memory, &seeds, &|_| Ok(Some(3))).0,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    p64(&mut memory.bytes, LEADER, 0);
    assert!(matches!(
        bans(&memory, &|_| Ok(Some(3))).0,
        Outcome::Unavailable { .. }
    ));
    let seeds = seeds_of(&memory);
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::RetreatBanRow::default(); 1];
    let mut asked = Vec::new();
    let outcome = packet.retreat_bans(
        &memory,
        0,
        &seeds,
        Outcome::Rows { len: 1 },
        &|_| Ok(Some(3)),
        &mut rows,
        &mut |leader| {
            asked.push(leader);
            Some(carmy_leader::RetreatBans {
                leader,
                ordinary: carmy_leader::RETREAT_BANNED_ATTACKING,
                alternate: carmy_leader::RETREAT_BANNED_ATTACKING,
                movement: [[None; 6]; 2],
            })
        },
    );
    assert_eq!(asked, [LEADER]);
    assert_eq!(
        (outcome, rows[0].bans),
        (
            Outcome::Rows { len: 1 },
            carmy_leader::RETREAT_BANNED_ATTACKING
        )
    );
    assert_eq!(
        packet
            .commander_bans()
            .map(|bans| bans.map(|bans| bans.map(|bans| bans.leader)))
            .collect::<Vec<_>>(),
        [Ok(Some(LEADER))]
    );
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
    p64(&mut memory.bytes, A1 + carmy::STRENGTH, 100_000);
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

#[derive(Default)]
struct MembershipTestSignals {
    stamps: std::cell::RefCell<std::collections::BTreeMap<usize, u64>>,
    calls: std::cell::Cell<usize>,
}

impl MembershipTestSignals {
    fn bump(&self, group: usize) {
        *self.stamps.borrow_mut().entry(group).or_insert(0) += 1;
    }
}

impl crate::corders_group_member::MembershipSignals for MembershipTestSignals {
    fn available(&self) -> bool {
        true
    }

    fn group(&self, receiver: usize, _slot: usize) -> Option<((u64, u64), usize)> {
        self.calls.set(self.calls.get() + 1);
        (receiver != 0).then(|| {
            (
                (1, self.stamps.borrow().get(&receiver).copied().unwrap_or(0)),
                receiver,
            )
        })
    }
}

fn membership_signal_fixture() -> (PresentationMemory, [super::DivisionSeed; 2]) {
    use crate::corder_instance as order;
    let mut memory = presentation_fixture();
    array(
        &mut memory.bytes,
        G1 + corders_group::PRIMARY_ORDER_DATA,
        0x17000,
        1,
    );
    p64(&mut memory.bytes, 0x17000, 0x18000);
    p64(&mut memory.bytes, 0x18000, order::VTABLE_RVA.0 as u64);
    p64(&mut memory.bytes, 0x18000 + order::OWNER, G1 as u64);
    array(
        &mut memory.bytes,
        0x18000 + order::ASSIGNED_MEMBER_DATA,
        0x19000,
        2,
    );
    let seeds = [A1, A2].map(|pointer| super::DivisionSeed {
        pointer,
        division: pointer as u64,
        orders_group: Some(G1),
        ..Default::default()
    });
    for (index, seed) in seeds.iter().enumerate() {
        p64(
            &mut memory.bytes,
            0x19000 + index * 8,
            (seed.pointer + cunit::SCHEDULED_MEMBER) as u64,
        );
    }
    (memory, seeds)
}

fn membership_test_packet(seeds: &[super::DivisionSeed]) -> super::StatusPacket {
    let mut packet = super::StatusPacket::new(seeds.len()).unwrap();
    for seed in seeds {
        packet.units.push(super::StatusUnitInputs {
            division: seed.division,
            group: seed.orders_group.unwrap(),
            ..Default::default()
        });
    }
    packet.outcome = Outcome::Rows { len: seeds.len() };
    packet
}

fn signaled_membership(
    memory: &impl crate::Memory,
    seeds: &[super::DivisionSeed],
    cache: &mut super::status_membership::Cache,
    signals: &impl crate::corders_group_member::MembershipSignals,
) -> super::StatusPacket {
    let mut packet = membership_test_packet(seeds);
    cache
        .acquire(memory, 0, seeds, &mut packet, signals)
        .unwrap();
    packet
}

#[test]
fn status_membership_carries_without_native_reads_and_reacquires_own_writes() {
    let (mut source, seeds) = membership_signal_fixture();
    let signals = MembershipTestSignals::default();
    let mut cache = super::status_membership::Cache::default();
    for _ in 0..2 {
        signaled_membership(&source, &seeds, &mut cache, &signals);
    }
    let observed = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let packet = signaled_membership(&observed, &seeds, &mut cache, &signals);
    for unit in &packet.units {
        assert_eq!(unit.has_order, Some(Ok(true)));
        assert_eq!(unit.membership, usize::MAX);
    }
    #[cfg(not(feature = "order_membership_oracle"))]
    assert!(observed.reads.borrow().is_empty());
    array(
        &mut source.bytes,
        0x18000 + crate::corder_instance::ASSIGNED_MEMBER_DATA,
        0x19000,
        0,
    );
    signals.bump(G1);
    let packet = signaled_membership(&source, &seeds, &mut cache, &signals);
    assert!(
        packet
            .units
            .iter()
            .all(|unit| unit.has_order == Some(Ok(false)) && unit.membership != usize::MAX)
    );
}

#[test]
fn status_membership_tracks_parent_writes_and_does_not_certify_a_new_parent_immediately() {
    let (mut source, seeds) = membership_signal_fixture();
    let signals = MembershipTestSignals::default();
    let mut cache = super::status_membership::Cache::default();
    p64(&mut source.bytes, G1 + corders_group::PARENT, 0x16000);
    for _ in 0..2 {
        signaled_membership(&source, &seeds, &mut cache, &signals);
    }
    signals.bump(0x16000);
    let packet = signaled_membership(&source, &seeds, &mut cache, &signals);
    assert_ne!(packet.units[0].membership, usize::MAX);
    assert_eq!(
        signaled_membership(&source, &seeds, &mut cache, &signals).units[0].membership,
        usize::MAX
    );
    p64(&mut source.bytes, G1 + corders_group::PARENT, 0x1a000);
    signals.bump(G1);
    for _ in 0..2 {
        assert_ne!(
            signaled_membership(&source, &seeds, &mut cache, &signals).units[0].membership,
            usize::MAX
        );
    }
    assert_eq!(
        signaled_membership(&source, &seeds, &mut cache, &signals).units[0].membership,
        usize::MAX
    );
}

#[test]
fn status_membership_errors_are_retried_and_never_certified() {
    let (source, seeds) = membership_signal_fixture();
    let signals = MembershipTestSignals::default();
    let mut cache = super::status_membership::Cache::default();
    for _ in 0..2 {
        signaled_membership(&source, &seeds, &mut cache, &signals);
    }
    signals.bump(G1);
    let fault = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: Some(0x19000),
    };
    for _ in 0..2 {
        let packet = signaled_membership(&fault, &seeds, &mut cache, &signals);
        assert!(packet.units[0].has_order.unwrap().is_err());
        assert_ne!(packet.units[0].membership, usize::MAX);
    }
    let packet = signaled_membership(&source, &seeds, &mut cache, &signals);
    assert_eq!(packet.units[0].has_order, Some(Ok(true)));
    assert_ne!(packet.units[0].membership, usize::MAX);
}

#[test]
fn status_membership_keys_and_mixed_batches_preserve_peer_admission() {
    let (source, mut seeds) = membership_signal_fixture();
    for seed in &mut seeds {
        seed.selected = Some(crate::carmy_leader::Selected {
            leader: None,
            handle: None,
            group_identity: true,
            block: Ok(None),
            training: None,
        });
    }
    let signals = MembershipTestSignals::default();
    let mut cache = super::status_membership::Cache::default();
    for _ in 0..2 {
        signaled_membership(&source, &seeds, &mut cache, &signals);
    }
    seeds[1].selected.as_mut().unwrap().group_identity = false;
    let observed = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let mixed = signaled_membership(&observed, &seeds, &mut cache, &signals);
    let full = signaled_membership(&source, &seeds, &mut Default::default(), &());
    assert_eq!(mixed.units[0].membership, usize::MAX);
    assert_ne!(mixed.units[1].membership, usize::MAX);
    assert!(observed.reads.borrow().contains(&G1));
    assert_eq!(
        mixed
            .units
            .iter()
            .map(|unit| unit.has_order)
            .collect::<Vec<_>>(),
        full.units
            .iter()
            .map(|unit| unit.has_order)
            .collect::<Vec<_>>()
    );
    for change in 0..2 {
        if change == 0 {
            seeds[0].division += 1;
        } else {
            seeds[0].pointer = A2;
        }
        let packet = signaled_membership(&source, &seeds, &mut cache, &signals);
        assert_ne!(packet.units[0].membership, usize::MAX);
        assert_eq!(packet.units[0].has_order, Some(Ok(true)));
    }
}

#[test]
fn unavailable_status_hours_neither_admit_nor_revoke_membership() {
    let (source, seeds) = membership_signal_fixture();
    let signals = MembershipTestSignals::default();
    let mut prepared = membership_test_packet(&seeds[..1]);
    let mut part = super::StatusPart::vacant();
    part.range = 0..1;
    prepared.outcome = Outcome::Unavailable {
        reason: "source gap",
    };
    part.capture_signaled(&source, 0, GS, &seeds, &prepared, &signals);
    assert_eq!(signals.calls.get(), 0);
    prepared.outcome = Outcome::Rows { len: 1 };
    for _ in 0..2 {
        part.capture_signaled(&source, 0, GS, &seeds, &prepared, &signals);
    }
    prepared.outcome = Outcome::Unavailable {
        reason: "source gap",
    };
    let before = signals.calls.get();
    part.capture_signaled(&source, 0, GS, &seeds, &prepared, &signals);
    assert_eq!(signals.calls.get(), before);
    prepared.outcome = Outcome::Rows { len: 1 };
    let observed = ObservedMovementMemory {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    part.capture_signaled(&observed, 0, GS, &seeds, &prepared, &signals);
    #[cfg(not(feature = "order_membership_oracle"))]
    assert!(!observed.reads.borrow().contains(&0x19000));
    assert!(signals.calls.get() > before);
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
    p64(&mut memory.bytes, 0x18000, order::VTABLE_RVA.0 as u64);
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
        packet.capture(&memory, 0, GS, &[], unavailable),
        unavailable
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
    p64(&mut memory.bytes, 0x18000, order::VTABLE_RVA.0 as u64);
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
        packet.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
fn status_packet_preserves_raw_maxima_through_owned_projection() {
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
    }];
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
        first.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }),
        Outcome::Rows { len: 1 }
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
        second.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
    p64(&mut memory.bytes, A1 + carmy::MAXIMUM_SUPPLY_GRACE, 0);
    p64(&mut memory.bytes, A1 + carmy::REMAINING_SUPPLY_GRACE, 0);
    let mut packet = super::StatusPacket::new(1).unwrap();
    let mut rows = [super::StatusRow::default(); 1];
    assert_eq!(
        packet.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
            packet.capture(&memory, 0, GS, &failed, Outcome::Rows { len: 1 }),
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
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
            packet.capture(&observed, 0, GS, &seeds, Outcome::Rows { len: 1 }),
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
    p64(&mut memory.bytes, A1 + carmy::EXPERIENCE, 20_000);
    assert_eq!(super::exercise_level(&memory, 0, A1), Ok(2));
    assert_eq!(super::exercising(&memory, 0, GS, A1, G1, 200_000), Ok(true));
    memory.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 1;
    assert_eq!(
        super::exercising(&memory, 0, GS, A1, G1, 200_000),
        Ok(false)
    );
    memory.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 0;
    memory.bytes[0x15011] = 1;
    assert_eq!(
        super::exercising(&memory, 0, GS, A1, G1, 200_000),
        Ok(false)
    );
    memory.bytes[0x15001] = 1;
    assert_eq!(super::exercising(&memory, 0, GS, A1, G1, 200_000), Ok(true));
    p32(&mut memory.bytes, 0x16004, i32::MAX as u32);
    p32(&mut memory.bytes, 0x1600c, 1);
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
    memory.globals.insert(
        metadata::TABLE_RVA.0,
        0x0100_0000_u64.to_le_bytes().to_vec(),
    );
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
    p64(&mut memory.bytes, 0x19000, chuman::VTABLE_RVA.0 as u64);
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
    assert_eq!(DIVISION_SEED_STRIDE, 383);
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
            (cunit::MOVEMENT_PRIORITY, 336, 4),
            (cunit::WITHDRAWAL, 340, 1),
            (FAKE_INTEL_TEMPLATE, 341, 8),
            (SEPARATE_COUNTER, 349, 1),
            (NONCORE_SUPPLY_BYPASS, 350, 1),
            (OUT_OF_SUPPLY_DAYS, 351, 4),
            (FUEL_CANDIDATE, 355, 8),
            (AIR_SUPPLY, 363, 8),
            (cunit::THEATRE, 371, 8),
            (cunit::QUEUED_ACTION_COUNT, 379, 4),
        ]
    );
}

#[test]
fn division_seeds_gather_supply_days_bypass_and_fuel() {
    use super::*;
    const POINTER: usize = 0x100;
    let mut bytes = vec![0_u8; 0x800];
    bytes[POINTER + NONCORE_SUPPLY_BYPASS] = 1;
    bytes[POINTER + AIR_SUPPLY..POINTER + AIR_SUPPLY + 8]
        .copy_from_slice(&7_654_321_i64.to_le_bytes());
    bytes[POINTER + OUT_OF_SUPPLY_DAYS..POINTER + OUT_OF_SUPPLY_DAYS + 4]
        .copy_from_slice(&(-3_i32).to_le_bytes());
    bytes[POINTER + FUEL_CANDIDATE..POINTER + FUEL_CANDIDATE + 8]
        .copy_from_slice(&1_234_567_i64.to_le_bytes());
    bytes[POINTER + cunit::THEATRE..POINTER + cunit::THEATRE + 8]
        .copy_from_slice(&0x7000_1230_u64.to_le_bytes());
    bytes[POINTER + cunit::QUEUED_ACTION_COUNT..POINTER + cunit::QUEUED_ACTION_COUNT + 4]
        .copy_from_slice(&2_i32.to_le_bytes());
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut seeds = [DivisionSeed {
        pointer: POINTER,
        division: 1,
        ..Default::default()
    }];
    capture_division_seeds(&memory, &mut seeds);
    assert_eq!(
        (
            seeds[0].noncore_supply_bypass,
            seeds[0].air_supply,
            seeds[0].out_of_supply_days,
            seeds[0].fuel
        ),
        (Some(1), Some(7_654_321), Some(-3), Some(1_234_567))
    );
    assert_eq!(
        (seeds[0].theatre, seeds[0].queued_actions),
        (Some(0x7000_1230), Some(2))
    );
    let short = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes[..POINTER + FUEL_CANDIDATE + 4],
    };
    let mut seeds = [DivisionSeed {
        pointer: POINTER,
        division: 1,
        ..Default::default()
    }];
    capture_division_seeds(&short, &mut seeds);
    assert_eq!(seeds[0].fuel, None);
    assert_eq!(seeds[0].out_of_supply_days, Some(-3));
}

#[test]
fn seeded_icon_inputs_read_no_division_field() {
    use super::*;
    let base = 0;
    const POINTER: usize = 0x100;
    let pointer = POINTER;
    let division: u64 = 51 | (9 << 32);
    let mut bytes = vec![0_u8; 0x800];
    bytes[pointer..pointer + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pointer + SECONDARY_BASE..pointer + SECONDARY_BASE + 8]
        .copy_from_slice(&(SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pointer + cunit::HANDLE..pointer + cunit::HANDLE + 8]
        .copy_from_slice(&division.to_le_bytes());
    bytes[pointer + cunit::ROSTER_OWNER..pointer + cunit::ROSTER_OWNER + 4]
        .copy_from_slice(&25_u32.to_le_bytes());
    bytes[pointer + SEPARATE_COUNTER] = 7;
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut seeds = [DivisionSeed {
        pointer,
        division,
        ..Default::default()
    }];
    capture_division_seeds(&memory, &mut seeds);
    assert_eq!(
        (seeds[0].separate_counter, seeds[0].alternate_template),
        (Some(7), Some(0))
    );
    let source = crate::cequipment_variant::pool::DivisionSource {
        pointer,
        division,
        country: 25,
        template: 0,
    };
    let mut identity = vec![0_u8; 0x800];
    identity[..0x100 + cunit::ROSTER_OWNER + 4]
        .copy_from_slice(&bytes[..0x100 + cunit::ROSTER_OWNER + 4]);
    struct Unread<'a>(crate::memory::SliceMemory<'a>);
    impl Memory for Unread<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let fields = [
                (POINTER + SEPARATE_COUNTER, 1),
                (POINTER + FAKE_INTEL_TEMPLATE, 8),
            ];
            if fields
                .iter()
                .any(|&(at, len)| address < at + len && at < address + output.len())
            {
                return false;
            }
            self.0.copy(address, output)
        }
    }
    let unread = Unread(crate::memory::SliceMemory {
        base: 0,
        bytes: &identity,
    });
    assert_eq!(
        read_icon_inputs(&unread, base, source, Some(&seeds[0])),
        Ok(IconInputsRow {
            division,
            separate_counter: 7,
            alternate_template: 0,
        })
    );
    for (failed, reason) in [
        (
            DivisionSeed {
                separate_counter: None,
                ..seeds[0]
            },
            "army separate-counter byte unreadable",
        ),
        (
            DivisionSeed {
                alternate_template: None,
                ..seeds[0]
            },
            "army alternate template pointer unreadable",
        ),
    ] {
        let counted = crate::read_census::ReadCensus::new(&memory);
        assert_eq!(
            read_icon_inputs(&counted, base, source, Some(&failed)),
            Err(reason)
        );
        assert_eq!(counted.calls(), 0);
    }
}

#[test]
fn status_packet_capacity_failure_is_out_of_memory_and_vacancy_allocates_nothing() {
    assert_eq!(
        super::StatusPacket::new(usize::MAX).err().unwrap().kind(),
        std::io::ErrorKind::OutOfMemory,
    );
    assert_eq!(super::StatusPacket::vacant().allocated_bytes(), 0);
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
    p64(&mut memory.bytes, STATE, cstate::VTABLE_RVA.0 as u64);
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
        modifier_definition::TABLE_RVA.0,
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

#[test]
fn a_pass_reads_each_acclimatization_category_factor_once() {
    use super::*;
    use crate::acclimatization_data as data;
    struct Reads(Vec<u8>, std::cell::RefCell<Vec<usize>>);
    impl Memory for Reads {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.1.borrow_mut().push(address);
            crate::memory::SliceMemory {
                base: CATEGORY,
                bytes: &self.0,
            }
            .copy(address, output)
        }
    }
    const CATEGORY: usize = 0x1000;
    let mut bytes = vec![0_u8; 0x40];
    bytes[data::CATEGORY_FACTOR..data::CATEGORY_FACTOR + 8]
        .copy_from_slice(&50_000_i64.to_le_bytes());
    let memory = Reads(bytes, Default::default());
    let division = |active: usize| {
        let mut object = [0_u8; 0x40];
        object[data::ACTIVE..data::ACTIVE + 8].copy_from_slice(&(active as u64).to_le_bytes());
        object[data::MAXIMUM..data::MAXIMUM + 8].copy_from_slice(&100_i64.to_le_bytes());
        object
    };
    let mut factors = CategoryFactors::default();
    for _ in 0..3 {
        assert_eq!(
            weather_factor(
                &memory,
                &division(CATEGORY),
                Some((CATEGORY as u64, 50)),
                &mut factors
            ),
            Ok(75_000)
        );
    }
    assert_eq!(*memory.1.borrow(), [CATEGORY + data::CATEGORY_FACTOR]);
    let unreadable = 0x9000;
    for _ in 0..2 {
        assert!(
            weather_factor(
                &memory,
                &division(unreadable),
                Some((unreadable as u64, 50)),
                &mut factors
            )
            .is_err()
        );
    }
    assert_eq!(memory.1.borrow().len(), 3);
    assert_eq!(factors.len, 1);
}

#[test]
fn movement_speed_keys_read_the_priority_sub_unit_then_its_categories_in_order() {
    use crate::csub_unit_definition as unit;
    const UNIT: usize = 0x1a000;
    const CATEGORIES: usize = 0x2c000;
    let mut memory = normal_route_fixture();
    let b = &mut memory.bytes;
    p32(b, UNIT + unit::SPEED_MODIFIER_KEY, 0x31);
    array(b, UNIT + unit::CATEGORIES, CATEGORIES, 4);
    for (slot, category, key) in [
        (0, 0x2c100, 0x40_u32),
        (1, 0x2c200, 0),
        (2, 0x2c100, 0x40),
        (3, 0x2c300, 0x41),
    ] {
        p64(b, CATEGORIES + slot * 8, category as u64);
        p32(b, category + unit::CATEGORY_MODIFIER_KEY, key);
    }
    let mut keys = vec![7];
    super::movement_speed_keys(&memory, 0, T1, &mut keys).unwrap();
    assert_eq!(keys, [7, 0x31, 0x40, 0x40, 0x41]);
    p32(&mut memory.bytes, UNIT + unit::SPEED_MODIFIER_KEY, 0);
    keys.clear();
    super::movement_speed_keys(&memory, 0, T1, &mut keys).unwrap();
    assert_eq!(keys, [0x40, 0x40, 0x41]);
    p64(&mut memory.bytes, CATEGORIES + 3 * 8, 0x7fff_0000);
    let mut keys = vec![7];
    assert!(super::movement_speed_keys(&memory, 0, T1, &mut keys).is_err());
    assert_eq!(keys, [7]);
    assert!(super::movement_speed_key_demands(&memory, 0, T1, &mut keys).is_err());
    assert_eq!(keys, [7, 0x40, 0x40]);
    p32(&mut memory.bytes, UNIT + unit::SPEED_MODIFIER_KEY, 0x31);
    p32(
        &mut memory.bytes,
        UNIT + unit::CATEGORIES + crate::cpdx_array::COUNT,
        5,
    );
    let mut keys = vec![7];
    assert!(super::movement_speed_key_demands(&memory, 0, T1, &mut keys).is_err());
    assert_eq!(keys, [7, 0x31]);
    let mut keys = vec![7];
    assert!(super::movement_speed_keys(&memory, 0, T1, &mut keys).is_err());
    assert_eq!(keys, [7]);
}

#[test]
fn exact_organisation_capture_keeps_hostile_later_land_edges_and_independent_refusals() {
    use super::*;
    use crate::{cdiplomacy_status as diplomacy, crelation::status as relation};
    const LAST: usize = 0x2e000;
    const DEFINITION: usize = 0x2e200;
    const DIPLOMACY: usize = 0x2d000;
    const RELATIONS: usize = 0x2d200;
    const RELATION: usize = 0x2d300;
    const WAR: usize = 0x2d500;
    let mut source = normal_route_fixture();
    let b = &mut source.bytes;
    array(b, GS + crate::cgame_state::PROVINCES, PROVINCES, 5);
    p64(b, PROVINCES + 4 * 8, LAST as u64);
    p64(b, LAST, cprovince::VTABLE_RVA.0 as u64);
    p32(b, LAST + cprovince::ID, 4);
    p64(b, LAST + cprovince::DEFINITION, DEFINITION as u64);
    p32(b, DEFINITION + crate::cprovince::definition::ID, 4);
    b[DEFINITION + crate::cprovince::definition::TERRAIN_FLAGS] = 1;
    b[0x18600 + crate::cprovince::definition::TERRAIN_FLAGS] = 0;
    p32(b, LAST + cprovince::CONTROLLER, 2);
    p64(b, C2 + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
    p64(b, DIPLOMACY, diplomacy::VTABLE_RVA.0 as u64);
    p64(b, DIPLOMACY + diplomacy::COUNTRY, C2 as u64);
    array(b, DIPLOMACY + diplomacy::RELATIONS, RELATIONS, 4);
    p64(b, RELATIONS + 8, RELATION as u64);
    p64(b, RELATION, relation::VTABLE_RVA.0 as u64);
    p64(b, RELATION + relation::DIPLOMACY_STATUS, DIPLOMACY as u64);
    p32(b, RELATION + relation::TARGET_COUNTRY, 3);
    p64(b, RELATION + relation::WAR_RELATION, WAR as u64);
    p64(b, WAR, crate::cwar_relation::VTABLE_RVA.0 as u64);
    p32(
        b,
        WAR + crate::crelation::TYPE_ID,
        crate::cwar_relation::TYPE,
    );
    array(b, A1 + cunit::ORDINARY_ROUTE_DATA, 0x1f000, 3);
    p32(b, 0x1f000, 2);
    p32(b, 0x1f004, 3);
    p32(b, 0x1f008, 4);
    let mut header = [0; 16];
    header
        .copy_from_slice(&b[A1 + cunit::ORDINARY_ROUTE_DATA..A1 + cunit::ORDINARY_ROUTE_DATA + 16]);
    let seed = DivisionSeed {
        pointer: A1,
        ordinary_route: Some(header),
        location: Some(P1),
        naval_transfer_membership: Some(0),
        strategic_redeployment: Some(0),
        organisation: Some(0),
        ..Default::default()
    };
    let mut packet = StatusPacket::vacant();
    packet.units.push(StatusUnitInputs::default());
    packet
        .maxima
        .push(crate::csub_unit_definition::DivisionMaxima {
            organisation: 1_000_000,
            ..Default::default()
        });
    assert_eq!(
        packet.capture_movement_organisation(&source, 0, GS, &[seed]),
        0
    );
    assert_eq!(
        packet.units[0].movement_organisation,
        Some(Some([0, 1_000_000]))
    );
    source.bytes[0x12000 + crate::cprovince::definition::TERRAIN_FLAGS] =
        crate::cprovince::definition::LAKE_FLAG;
    source.bytes[0x18600 + crate::cprovince::definition::TERRAIN_FLAGS] =
        crate::cprovince::definition::LAND_FLAG;
    assert_eq!(
        packet.capture_movement_organisation(&source, 0, GS, &[seed]),
        0
    );
    assert_eq!(
        packet.units[0].movement_organisation,
        Some(Some([0, 1_000_000]))
    );
    source.bytes[0x12000 + crate::cprovince::definition::TERRAIN_FLAGS] =
        crate::cprovince::definition::LAND_FLAG;
    p32(&mut source.bytes, 0x1f000, 99);
    assert_eq!(
        packet.capture_movement_organisation(&source, 0, GS, &[seed]),
        1
    );
    assert_eq!(
        packet.units[0].movement_organisation,
        Some(Some([0, 1_000_000]))
    );
    let transfer = DivisionSeed {
        naval_transfer_membership: Some(1),
        organisation: None,
        location: None,
        ..seed
    };
    assert_eq!(
        packet.capture_movement_organisation(&source, 0, GS, &[transfer]),
        0
    );
    assert_eq!(packet.units[0].movement_organisation, Some(None));
    source.bytes[0x18600 + crate::cprovince::definition::TERRAIN_FLAGS] = 0;
    source.bytes[0x12000 + crate::cprovince::definition::TERRAIN_FLAGS] = 0;
    p32(&mut source.bytes, 0x1f000, 2);
    let mut sea_header = header;
    sea_header[12..16].copy_from_slice(&1_u32.to_le_bytes());
    let sea = DivisionSeed {
        ordinary_route: Some(sea_header),
        naval_transfer_membership: None,
        organisation: None,
        ..seed
    };
    assert_eq!(
        packet.capture_movement_organisation(&source, 0, GS, &[sea]),
        0
    );
    assert_eq!(packet.units[0].movement_organisation, Some(None));
}

fn model_inputs_test_capture(
    memory: &impl crate::Memory,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
) -> (super::ModelInputsReport, Vec<super::ModelInputsRow>) {
    let mut seeds: Vec<_> = sources
        .iter()
        .map(|source| super::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..super::DivisionSeed::default()
        })
        .collect();
    super::capture_division_seeds(memory, &mut seeds);
    let mut packet = super::StatusPacket::new(0).unwrap();
    let mut rows = vec![super::ModelInputsRow::default(); seeds.len()];
    let report = packet.capture_model_inputs(
        memory,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: seeds.len() },
        &mut rows,
    );
    (report, rows)
}

fn status_model_columns(memory: &PresentationMemory) -> Option<super::ModelInputsRow> {
    let mut rows = [super::StatusRow::default(); 1];
    let outcome = super::walk_status(
        memory,
        0,
        GS,
        &presentation_source(),
        Outcome::Rows { len: 1 },
        &mut rows,
    );
    (outcome == Outcome::Rows { len: 1 }).then_some(super::ModelInputsRow {
        division: rows[0].division,
        model_number: Some(rows[0].model_number),
        camouflage: rows[0].camouflage,
        exercising: Some(rows[0].exercising),
    })
}

#[test]
fn model_inputs_equal_division_status_and_fail_only_their_own_field() {
    use crate::{acclimatization_category, acclimatization_data};
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
        15_000_000,
    );
    memory.globals.insert(
        acclimatization_data::CAMO_SHOW_AT_RVA.0,
        50_000_i64.to_le_bytes().to_vec(),
    );
    for (index, category, token, weight) in [
        (0, 0x16100_usize, 7_u32, 9_000_000_u64),
        (1, 0x16200, 8, 8_000_000),
    ] {
        p64(&mut memory.bytes, 0x16000 + index * 16, category as u64);
        p64(&mut memory.bytes, 0x16008 + index * 16, weight);
        p32(
            &mut memory.bytes,
            category + acclimatization_category::TOKEN,
            token,
        );
    }
    memory.globals.insert(
        acclimatization_category::REGISTRY_RVA.0,
        0x16800_u64.to_le_bytes().to_vec(),
    );
    array(&mut memory.bytes, 0x16800, 0x16900, 2);
    p64(&mut memory.bytes, 0x16900, 0x16200);
    p64(&mut memory.bytes, 0x16908, 0x16100);
    let witness = |memory: &PresentationMemory| {
        let (report, rows) = model_inputs_test_capture(memory, &presentation_source());
        assert_eq!(report.outcome, Outcome::Rows { len: 1 });
        (rows[0], report.training)
    };
    let shown = Some(super::Camouflage { category: Some(7) });

    let idle = status_model_columns(&memory).unwrap();
    assert_eq!(
        (idle.model_number, idle.camouflage, idle.exercising),
        (Some(0x1ff), shown, Some(0))
    );
    let observed = ObservedMovementMemory {
        source: &memory,
        reads: Default::default(),
        fault: None,
    };
    let (report, rows) = model_inputs_test_capture(&observed, &presentation_source());
    assert_eq!((rows[0], report.training), (idle, 0));
    assert!(observed.reads.borrow().iter().all(|&address| {
        address != A1 + carmy::DEPLOYMENT_TYPE
            && address != A1 + carmy::EXPERIENCE
            && address != 0x11000
    }));

    memory.bytes[G1 + corders_group::TRAINING] = 1;
    let training = status_model_columns(&memory).unwrap();
    assert_eq!(training.exercising, Some(1));
    assert_eq!(witness(&memory), (training, 1));
    p64(&mut memory.bytes, A1 + carmy::STRENGTH, 100_000);
    let weak = status_model_columns(&memory).unwrap();
    assert_eq!(weak.exercising, Some(0));
    assert_eq!(witness(&memory), (weak, 1));
    p64(&mut memory.bytes, A1 + carmy::STRENGTH, 150_000);

    array(
        &mut memory.bytes,
        object + acclimatization_data::DATA,
        0x16000,
        0,
    );
    let empty = status_model_columns(&memory).unwrap();
    assert_eq!(empty.camouflage, Some(super::Camouflage { category: None }));
    assert_eq!(witness(&memory).0, empty);
    array(
        &mut memory.bytes,
        object + acclimatization_data::DATA,
        0x7fff_0000,
        2,
    );
    let unreadable = status_model_columns(&memory).unwrap();
    assert_eq!(
        (unreadable.camouflage, unreadable.exercising),
        (None, Some(1))
    );
    assert_eq!(witness(&memory).0, unreadable);
    array(
        &mut memory.bytes,
        object + acclimatization_data::DATA,
        0x16000,
        2,
    );

    let define = memory
        .globals
        .remove(&carmy::EXERCISE_MINIMUM_STRENGTH_RVA.0)
        .unwrap();
    assert_eq!(status_model_columns(&memory), None);
    assert_eq!(
        witness(&memory).0,
        super::ModelInputsRow {
            exercising: None,
            ..training
        }
    );
    memory
        .globals
        .insert(carmy::EXERCISE_MINIMUM_STRENGTH_RVA.0, define);

    let sources = [
        presentation_source()[0],
        crate::cequipment_variant::pool::DivisionSource {
            pointer: A2,
            division: handle(9, 51),
            template: handle(0, 52),
            country: 3,
        },
    ];
    let (report, rows) = model_inputs_test_capture(&memory, &sources);
    assert_eq!(
        (report.outcome, report.training),
        (Outcome::Rows { len: 2 }, 2)
    );
    assert_eq!(rows[0], training);
    assert_eq!(
        rows[1],
        super::ModelInputsRow {
            division: handle(9, 51),
            model_number: Some(0),
            camouflage: Some(super::Camouflage { category: None }),
            exercising: None,
        }
    );

    p64(&mut memory.bytes, A1 + cunit::HANDLE, handle(9, 9001));
    assert_eq!(status_model_columns(&memory), None);
    assert_eq!(
        witness(&memory).0,
        super::ModelInputsRow {
            division: handle(9, 9000),
            ..super::ModelInputsRow::default()
        }
    );
}