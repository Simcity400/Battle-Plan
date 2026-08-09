//! The 2026-08-09 vocabulary amendment, one test per thing lane FX proved the
//! substrate could not say.
//!
//! Each test names the PRODUCTION CHANGE that makes it fail. A test whose
//! answer does not depend on the mechanism it is named after is decoration, and
//! this file exists because nine mechanisms were added at once.

use super::annot::StridedMask;
use super::executor::{
    observe, validate, Admissibility, CarrySource, CloseStage, ClosingProgram, CompareMask,
    CountExpr, CountTerm, DisagreeAction, ObservationConfig, OpMod, Probe, Recipe, RecipeOp,
    RecipeOutcome, RejectAction, RelWindow, Root, SampleSpec, SnapshotBuild, Tolerance,
};
use super::synthetic::{MutatingMemory, SyntheticMemory};
use super::view::ViewRefusal;
use super::*;

const HOUR: HourStamp = HourStamp(19390901);

fn config() -> ObservationConfig {
    ObservationConfig::new(HOUR)
}

fn mods(count: usize, edits: &[(usize, OpMod)]) -> Vec<OpMod> {
    let mut table = vec![OpMod::default(); count];
    for (index, modifier) in edits {
        table[*index] = modifier.clone();
    }
    table
}

fn refusal_of(build: &SnapshotBuild, id: u32) -> Option<RefusalRecord> {
    match build.report_for(RecipeId(id)).expect("reported").outcome {
        RecipeOutcome::Refused(record) => Some(record),
        RecipeOutcome::Captured => None,
    }
}

// ---------------------------------------------------------------------------
// M1. A count derived from an expression over copied header fields
// ---------------------------------------------------------------------------

/// d81's hash tables are `mask + max_probe + 2` slots long, because
/// `select_secondary` starts at the masked slot, advances without wrapping, and
/// reads the terminating marker one slot past its last admitted step. A recipe
/// that could only name ONE count field would copy `mask + 1` slots and stop
/// short of the probe tail — quietly, with a `Captured` outcome and a table
/// that does not contain the entry the reader is looking for.
///
/// MUTATION: delete `constant` from the sum in `evaluate_count` (or drop a
/// term) and the tail slot stops being covered; the assertion below fails
/// because the view refuses the bytes.
#[test]
fn a_derived_extent_covers_the_probe_tail_a_single_count_field_would_miss() {
    const HEADER: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    const STRIDE: u32 = 0x18;
    // mask 7 (so 8 masked slots) + max_probe 2 + 2 = 12 slots.
    const MASK: u32 = 7;
    const MAX_PROBE: u8 = 2;
    const DERIVED: u64 = (MASK + 1 + MAX_PROBE as u32 + 1) as u64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HEADER, 0x40);
    memory.map_zeroed(TABLE, 64 * STRIDE as usize);
    memory.write_u64(HEADER + 0x08, TABLE);
    memory.write_u32(HEADER + 0x34, MASK);
    memory.write(HEADER + 0x38, &[MAX_PROBE]);

    let ops = vec![
        RecipeOp::CopyHeader {
            size: 0x40,
            next: vec![1],
        },
        RecipeOp::DecodeExtent {
            // Ignored: the modifier supplies the count.
            count_offset: 0x34,
            count_width: 4,
            ptr_offset: 0x08,
            stride: STRIDE,
            max_count: 64,
            max_bytes: 1 << 20,
            trailer_elements: 0,
            guard: None,
            register: None,
            element: vec![2],
        },
        RecipeOp::Freeze,
    ];
    let table = mods(
        3,
        &[(
            1,
            OpMod {
                count: Some(CountExpr {
                    terms: vec![
                        CountTerm {
                            offset: 0x34,
                            width: 4,
                        },
                        CountTerm {
                            offset: 0x38,
                            width: 1,
                        },
                    ],
                    constant: 2,
                }),
                ..OpMod::default()
            },
        )],
    );
    let recipe = Recipe::new(RecipeId(1), ConsumerId(1), ops.clone(), vec![Root {
        op: 0,
        addr: HEADER,
    }])
    .with_mods(table);

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(refusal_of(&build, 1).is_none(), "the derived extent captures");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    let last = TABLE + (DERIVED - 1) * u64::from(STRIDE);
    assert!(
        view.bytes(last, STRIDE as usize).is_ok(),
        "the terminating slot at {last:#x} must be inside the copied extent",
    );
    assert!(
        view.bytes(TABLE + DERIVED * u64::from(STRIDE), 1).is_err(),
        "and nothing past the derived count",
    );

    // The control: the same recipe WITHOUT the expression reads the mask alone
    // and copies seven slots, so it stops five short of the tail.
    let plain = Recipe::new(RecipeId(1), ConsumerId(1), ops, vec![Root {
        op: 0,
        addr: HEADER,
    }]);
    let build = observe(&[plain], &memory, config()).expect("custody");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(
        view.bytes(last, STRIDE as usize).is_err(),
        "a single count field is exactly the shortfall this mechanism exists to close",
    );
}

/// A count expression is a LENGTH, so its arithmetic has to refuse rather than
/// wrap. Summed in `u64` a header of `u64::MAX` fields lands on a small,
/// admissible count and the recipe copies a table it never checked.
#[test]
fn a_count_expression_that_overflows_refuses_instead_of_wrapping() {
    const HEADER: u64 = 0x10_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HEADER, 0x40);
    memory.write_u64(HEADER + 0x08, 0x20_0000);
    memory.write_u64(HEADER + 0x10, u64::MAX);
    memory.write_u64(HEADER + 0x18, u64::MAX);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x40,
                next: vec![1],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0x10,
                count_width: 8,
                ptr_offset: 0x08,
                stride: 8,
                max_count: 1024,
                max_bytes: 1 << 20,
                trailer_elements: 0,
                guard: None,
                register: None,
                element: vec![2],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: HEADER,
        }],
    )
    .with_mods(mods(
        3,
        &[(
            1,
            OpMod {
                count: Some(CountExpr {
                    terms: vec![
                        CountTerm {
                            offset: 0x10,
                            width: 8,
                        },
                        CountTerm {
                            offset: 0x18,
                            width: 8,
                        },
                    ],
                    constant: 2,
                }),
                ..OpMod::default()
            },
        )],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("an overflowing count refuses");
    assert!(
        matches!(record.refusal, Refusal::ExtentCountExceeded { .. }),
        "got {:?}",
        record.refusal
    );
}

// ---------------------------------------------------------------------------
// M2. Carried context, and per-instance table bindings
// ---------------------------------------------------------------------------

/// A province id decoded at the topology has to select into a table two waves
/// later. Before this the key could only be read at an offset from the CURRENT
/// site, so the id had to be a field of whatever object the join stood on —
/// which it is not.
///
/// WHAT MAKES THIS NON-VACUOUS: the extent copies the whole array, so asserting
/// that the selected ROW is present would pass whatever the join did. The
/// assertion is on the object the selected row POINTS AT, which is reached only
/// by following the joined row, and the row-0 object is asserted absent.
///
/// MUTATION: make `resolve_carry` return `site.carry` unconditionally (ignore
/// the modifier) and the join keys on nothing — the recipe refuses
/// `CarryNotSet`. Point the carry at a different offset and it follows row 0's
/// pointer instead, which the second assertion catches.
#[test]
fn a_key_decoded_at_one_site_selects_a_row_two_waves_later() {
    const ROOT: u64 = 0x10_0000;
    const HOP: u64 = 0x11_0000;
    const HEADER: u64 = 0x12_0000;
    const TABLE: u64 = 0x20_0000;
    const ROW_OBJECT: u64 = 0x40_0000;
    const OBJECT_STRIDE: u64 = 0x1000;
    const STRIDE: u32 = 0x10;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.map_zeroed(HOP, 0x20);
    memory.map_zeroed(HEADER, 0x20);
    memory.map_zeroed(TABLE, 8 * STRIDE as usize);
    // The id lives at the ROOT and nowhere else.
    memory.write_u32(ROOT + 0x10, 3);
    memory.write_u64(ROOT, HOP);
    memory.write_u64(HOP, HEADER);
    memory.write_u64(HEADER + 0x08, TABLE);
    memory.write_u32(HEADER, 8);
    for slot in 0..8u64 {
        let object = ROW_OBJECT + slot * OBJECT_STRIDE;
        memory.map_zeroed(object, 0x10);
        memory.write_u64(object, 0xA000 + slot);
        memory.write_u64(TABLE + slot * u64::from(STRIDE), object);
    }

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            // 0 root object
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            // 1 carry the id, then hop
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            // 2 the intermediate object
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![3],
            },
            // 3 hop again
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 4,
            },
            // 4 the table header: register the array AND run the join over it
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![5, 6],
            },
            // 5 register the table. Its elements do NOT follow their pointers:
            // the only route to a row object is the join.
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 0x08,
                stride: STRIDE,
                max_count: 64,
                max_bytes: 1 << 20,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(1)),
                element: vec![9],
            },
            // 6 join by the CARRIED id
            RecipeOp::JoinBounded {
                table: TableId(1),
                key_offset: 0,
                key_width: 4,
                next: 7,
            },
            // 7 the selected row
            RecipeOp::CopyHeader {
                size: STRIDE,
                next: vec![8],
            },
            // 8 follow the selected row's pointer
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 10,
            },
            // 9 freeze
            RecipeOp::Freeze,
            // 10 the row's object
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![9],
            },
        ],
        vec![Root { op: 0, addr: ROOT }],
    )
    .with_mods(mods(
        11,
        &[
            (
                1,
                OpMod {
                    carry: Some(CarrySource::Field {
                        offset: 0x10,
                        width: 4,
                    }),
                    ..OpMod::default()
                },
            ),
            (
                6,
                OpMod {
                    key_from_carry: true,
                    ..OpMod::default()
                },
            ),
        ],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(refusal_of(&build, 1).is_none(), "{:?}", refusal_of(&build, 1));
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.u64_le(ROW_OBJECT + 3 * OBJECT_STRIDE)
            .expect("row 3's object was reached through the join"),
        0xA003,
    );
    assert!(
        view.bytes(ROW_OBJECT, 8).is_err(),
        "and only the selected row's object: a key of 0 would copy this one",
    );
}

/// One logical table decoded once per instance used to overwrite its own
/// binding, so the last registration won and every earlier join silently
/// indexed the wrong array.
///
/// WHAT MAKES THIS NON-VACUOUS: both arrays are copied whole by their extents,
/// so the assertion is again on the object the joined row POINTS AT. Owner A
/// asks for row 2 and owner B for row 3; under a single binding both joins land
/// in whichever table registered last, and exactly one of the two expected
/// objects is missing.
#[test]
fn two_instances_of_one_table_stay_independently_addressable() {
    const OWNER_A: u64 = 0x10_0000;
    const OWNER_B: u64 = 0x11_0000;
    const TABLE_A: u64 = 0x20_0000;
    const TABLE_B: u64 = 0x21_0000;
    const ROWS_A: u64 = 0x40_0000;
    const ROWS_B: u64 = 0x50_0000;
    const OBJECT_STRIDE: u64 = 0x1000;
    const STRIDE: u32 = 8;

    let mut memory = SyntheticMemory::new();
    for (owner, table, objects) in [(OWNER_A, TABLE_A, ROWS_A), (OWNER_B, TABLE_B, ROWS_B)] {
        memory.map_zeroed(owner, 0x20);
        memory.map_zeroed(table, 4 * STRIDE as usize);
        memory.write_u32(owner, 4);
        memory.write_u64(owner + 0x08, table);
        for slot in 0..4u64 {
            let object = objects + slot * OBJECT_STRIDE;
            memory.map_zeroed(object, 0x10);
            memory.write_u64(object, objects + slot);
            memory.write_u64(table + slot * u64::from(STRIDE), object);
        }
    }
    // Each owner carries its own instance id and its own key.
    memory.write_u32(OWNER_A + 0x10, 0);
    memory.write_u32(OWNER_B + 0x10, 1);
    // Owner A asks for row 2 of ITS table; owner B asks for row 3 of ITS.
    memory.write_u32(OWNER_A + 0x14, 2);
    memory.write_u32(OWNER_B + 0x14, 3);

    let ops = vec![
        RecipeOp::CopyHeader {
            size: 0x20,
            next: vec![1, 2],
        },
        RecipeOp::DecodeExtent {
            count_offset: 0,
            count_width: 4,
            ptr_offset: 0x08,
            stride: STRIDE,
            max_count: 16,
            max_bytes: 1 << 20,
            trailer_elements: 0,
            guard: None,
            register: Some(TableId(1)),
            element: vec![4],
        },
        RecipeOp::JoinBounded {
            table: TableId(1),
            key_offset: 0x14,
            key_width: 4,
            next: 3,
        },
        RecipeOp::CopyHeader {
            size: STRIDE,
            next: vec![5],
        },
        RecipeOp::Freeze,
        RecipeOp::DecodePointer {
            offset: 0,
            guard: None,
            next: 6,
        },
        RecipeOp::CopyHeader {
            size: 0x10,
            next: vec![4],
        },
    ];

    let instance = OpMod {
        instance_from_carry: true,
        carry: Some(CarrySource::Field {
            offset: 0x10,
            width: 4,
        }),
        ..OpMod::default()
    };
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops,
        vec![
            Root {
                op: 0,
                addr: OWNER_A,
            },
            Root {
                op: 0,
                addr: OWNER_B,
            },
        ],
    )
    .with_mods(mods(7, &[(1, instance.clone()), (2, instance)]));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(refusal_of(&build, 1).is_none(), "{:?}", refusal_of(&build, 1));
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.u64_le(ROWS_A + 2 * OBJECT_STRIDE)
            .expect("A's join reached A's row 2"),
        ROWS_A + 2,
    );
    assert_eq!(
        view.u64_le(ROWS_B + 3 * OBJECT_STRIDE)
            .expect("B's join reached B's row 3"),
        ROWS_B + 3,
    );
    assert!(
        view.bytes(ROWS_A + 3 * OBJECT_STRIDE, 8).is_err()
            && view.bytes(ROWS_B + 2 * OBJECT_STRIDE, 8).is_err(),
        "neither join may cross into the other instance's table",
    );
}

// ---------------------------------------------------------------------------
// M3. Pointer admissibility
// ---------------------------------------------------------------------------

/// Without a window the substrate follows a pointer the reader would have
/// rejected. If the address happens to be readable it copies a wrong object and
/// reports `Captured`; if it does not, a typed `InvalidPointer` becomes a
/// generic memory refusal that gaps the whole recipe.
///
/// MUTATION: make `Admissibility::admits` return `true` unconditionally and the
/// rejection count drops to zero while the branch is followed.
#[test]
fn an_inadmissible_pointer_is_named_and_not_followed() {
    const ROOT: u64 = 0x10_0000;
    const GOOD: u64 = 0x30_0000;
    const OUTSIDE: u64 = 0x90_0000;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.map_zeroed(GOOD, 0x20);
    memory.map_zeroed(OUTSIDE, 0x20);
    memory.write_u64(ROOT, GOOD);
    // Readable, correctly aligned, and outside the heap window.
    memory.write_u64(ROOT + 8, OUTSIDE);

    let window = Admissibility {
        low: 0x20_0000,
        high: 0x40_0000,
        align: 8,
        singletons: [0, 0],
        on_reject: RejectAction::StopBranch,
    };
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 3,
            },
            RecipeOp::DecodePointer {
                offset: 8,
                guard: None,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: ROOT }],
    )
    .with_mods(mods(
        5,
        &[
            (
                1,
                OpMod {
                    admit: Some(window),
                    stage: 7,
                    ..OpMod::default()
                },
            ),
            (
                2,
                OpMod {
                    admit: Some(window),
                    stage: 9,
                    ..OpMod::default()
                },
            ),
        ],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(
        refusal_of(&build, 1).is_none(),
        "a recorded rejection is a BRANCH outcome; the recipe keeps its hour",
    );
    let report = build.report_for(RecipeId(1)).expect("reported");
    assert_eq!(report.pointers_rejected, 1);
    let rejections = build.snapshot.pointer_rejections();
    assert_eq!(rejections.len(), 1);
    assert_eq!(rejections[0].addr, OUTSIDE);
    assert_eq!(
        rejections[0].witness.stage, 9,
        "the rejection names the stage that decoded it",
    );
    assert_eq!(rejections[0].witness.subject, ROOT);

    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(view.bytes(GOOD, 8).is_ok(), "the admitted branch still runs");
    assert!(
        view.bytes(OUTSIDE, 8).is_err(),
        "and the rejected pointer was not followed",
    );
}

/// The other half of the ruling: a recipe may declare the rejection fatal, and
/// then it is a named refusal rather than a crash or a silent copy.
#[test]
fn a_fatal_rejection_names_the_window_it_failed() {
    const ROOT: u64 = 0x10_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.write_u64(ROOT, 0x99_0001);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: ROOT }],
    )
    .with_mods(mods(
        4,
        &[(
            1,
            OpMod {
                admit: Some(Admissibility {
                    low: 0x20_0000,
                    high: 0x40_0000,
                    align: 8,
                    singletons: [0, 0],
                    on_reject: RejectAction::RefuseRecipe,
                }),
                ..OpMod::default()
            },
        )],
    ));
    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("refused");
    assert_eq!(
        record.refusal,
        Refusal::PointerNotAdmissible {
            recipe: RecipeId(1),
            addr: 0x99_0001,
            low: 0x20_0000,
            high: 0x40_0000,
        },
    );
    assert_eq!(record.witness.raw, 0x99_0001);
    assert_eq!(record.witness.read_addr, ROOT);
    assert_eq!(record.witness.read_width, 8);
}

// ---------------------------------------------------------------------------
// M4. The adjacent A/B pair
// ---------------------------------------------------------------------------

/// Memory that hands back a different byte on ONE nominated copy and the
/// original on every other.
///
/// This is the shape the opening/closing witness structurally cannot see: a
/// write that happened and was undone between the two waves. It is also
/// precisely what `read_u64_stable` catches in production, which is why the
/// substrate not being able to express it was a gap and not a preference.
#[derive(Debug)]
struct TearingMemory {
    inner: SyntheticMemory,
    tear_on: std::cell::Cell<u64>,
    seen: std::cell::Cell<u64>,
    at: u64,
}

impl SnapshotMemory for TearingMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        self.inner.copy_into(addr, out)?;
        let seen = self.seen.get() + 1;
        self.seen.set(seen);
        if seen == self.tear_on.get() {
            let span = Span::new(addr, out.len() as u64);
            if span.contains_addr(self.at) {
                out[(self.at - addr) as usize] ^= 0xFF;
            }
        }
        Ok(())
    }
}

/// MUTATION: set `SampleSpec::count` to 1 in `copy_attributes` (i.e. ignore the
/// modifier) and the tear goes unseen — the consumer publishes an image the
/// copier took mid-write.
#[test]
fn an_adjacent_pair_catches_a_tear_the_closing_witness_cannot() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x1122_3344_5566_7788);

    let build_recipe = |samples: u8| {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::WitnessClose { next: vec![2] },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(
            3,
            &[(
                0,
                OpMod {
                    samples: SampleSpec {
                        count: samples,
                        on_disagree: DisagreeAction::Gap,
                    },
                    ..OpMod::default()
                },
            )],
        ))
    };

    // Copy 1 is the opening image, copy 2 the adjacent sample, copy 3 the
    // close. Tear the SECOND one only: opening and closing agree, so the
    // closing witness certifies an object that was written under the copier.
    let memory = TearingMemory {
        inner,
        tear_on: std::cell::Cell::new(2),
        seen: std::cell::Cell::new(0),
        at: OBJECT + 3,
    };
    let closed = observe(&[build_recipe(2)], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        closed.witness.changed.is_empty(),
        "the opening and closing images are identical, by construction",
    );
    assert_eq!(closed.witness.torn.len(), 1, "the adjacent pair saw it");
    assert!(matches!(
        closed.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::SamplesDisagreed { .. })
    ));

    // The control: one sample, same memory, same tear. Nothing sees it.
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x1122_3344_5566_7788);
    let memory = TearingMemory {
        inner,
        tear_on: std::cell::Cell::new(2),
        seen: std::cell::Cell::new(0),
        at: OBJECT + 3,
    };
    let closed = observe(&[build_recipe(1)], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        closed.witness.torn.is_empty() && !closed.witness.changed.is_empty(),
        "with one sample the tear lands in the CLOSING copy and reads as a change, \
         which is a different and weaker statement",
    );
}

// ---------------------------------------------------------------------------
// M5. A refusal carries its witness
// ---------------------------------------------------------------------------

/// `(recipe, span)` cannot reproduce d81's first failure. This pins the fields
/// the executor is in a position to fill at a join, which is the site with the
/// most of them.
#[test]
fn a_refusal_carries_the_stage_subject_index_and_value_that_caused_it() {
    const HEADER: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HEADER, 0x20);
    memory.map_zeroed(TABLE, 4 * 8);
    memory.write_u32(HEADER, 4);
    memory.write_u64(HEADER + 8, TABLE);
    // The key is out of range of the four-element table.
    memory.write_u32(HEADER + 0x10, 9);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1, 2],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: 8,
                max_count: 16,
                max_bytes: 1 << 20,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(1)),
                element: vec![4],
            },
            RecipeOp::JoinBounded {
                table: TableId(1),
                key_offset: 0x10,
                key_width: 4,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: HEADER,
        }],
    )
    .with_mods(mods(
        5,
        &[(
            2,
            OpMod {
                stage: 42,
                ..OpMod::default()
            },
        )],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("refused");
    assert!(matches!(record.refusal, Refusal::JoinOutOfRange { key: 9, .. }));
    assert_eq!(record.witness.stage, 42, "the stage the reader calls this");
    assert_eq!(record.witness.op, 2);
    assert_eq!(record.witness.subject, HEADER);
    assert_eq!(record.witness.raw, 9, "the key that was out of range");
    assert_eq!(record.witness.related, TABLE, "the table it indexed");
    // And the gap the consumer is handed carries the same record, rather than
    // making it fetch one.
    assert_eq!(
        build.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::Refused(record)),
    );
}

// ---------------------------------------------------------------------------
// RULING A. Over-read gaps only what is consumed
// ---------------------------------------------------------------------------

/// A copy plan must take supersets. Before this ruling one unreadable bucket in
/// a table the reader never indexes cost the whole frame.
///
/// MUTATION: make `acquire` return `Ok(Err(hole))` regardless of tolerance and
/// the recipe refuses, so the two readable windows stop being readable.
#[test]
fn an_unreadable_span_in_a_speculative_copy_refuses_only_the_reads_that_touch_it() {
    const A: u64 = 0x10_0000;
    const B: u64 = 0x11_0000;
    const C: u64 = 0x12_0000;
    let build_memory = || {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(A, 0x100);
        memory.map_zeroed(B, 0x100);
        memory.map_zeroed(C, 0x100);
        memory.write_u64(A, 0xAA);
        memory.write_u64(C, 0xCC);
        memory.set_readable(B, false);
        memory
    };
    let ops = vec![
        RecipeOp::CopyRanges {
            windows: vec![
                RelWindow::new(0, 0x40),
                RelWindow::new(0x1_0000, 0x40),
                RelWindow::new(0x2_0000, 0x40),
            ],
            next: vec![1],
        },
        RecipeOp::Freeze,
    ];

    let memory = build_memory();
    let recipe = Recipe::new(RecipeId(1), ConsumerId(1), ops.clone(), vec![Root {
        op: 0,
        addr: A,
    }])
    .with_mods(mods(
        2,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(
        refusal_of(&build, 1).is_none(),
        "the frame publishes: {:?}",
        refusal_of(&build, 1)
    );
    assert_eq!(build.report_for(RecipeId(1)).unwrap().scoped_voids, 1);
    assert_eq!(build.snapshot.voids().len(), 1);
    assert_eq!(build.snapshot.voids()[0].span.start, B);

    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(view.u64_le(A).expect("A publishes"), 0xAA);
    assert_eq!(view.u64_le(C).expect("C publishes"), 0xCC);
    match view.u64_le(B) {
        Err(ViewRefusal::Unreadable { void, .. }) => assert_eq!(void.start, B),
        other => panic!("a read that touches the void must name it, got {other:?}"),
    }

    // The control: the same shape at the default tolerance is the old
    // behaviour, unchanged. Guarantee 1 is not weakened for anyone who did not
    // ask.
    let memory = build_memory();
    let strict = Recipe::new(RecipeId(1), ConsumerId(1), ops, vec![Root { op: 0, addr: A }]);
    let build = observe(&[strict], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("Required still refuses the recipe");
    assert!(matches!(record.refusal, Refusal::MemoryUnreadable { .. }));
}

// ---------------------------------------------------------------------------
// RULING B. Comparison masks, closure order, elision
// ---------------------------------------------------------------------------

/// The bytes a reader does not consume must not gap it. d81's rail header has
/// four dead bytes inside its window; d47's map header has a LIVE pointer field
/// sitting BETWEEN two compared words. Under a whole-interval comparison either
/// one manufactures a torn gap out of a write the reader never looks at.
///
/// MUTATION: ignore the consumption in `close_witnesses` and compare whole
/// intervals; the first half of this test fails immediately.
#[test]
fn a_write_between_the_compared_words_does_not_gap_the_reader() {
    const OBJECT: u64 = 0x10_0000;
    let build_memory = || {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(OBJECT, 0x20);
        memory.write_u64(OBJECT, 0x1111);
        memory.write_u64(OBJECT + 0x10, 0x2222);
        memory
    };
    // Consumed: [0,8) and [0x10,0x18). The live field at +8 is in neither.
    let masked = |edits: Vec<(u64, Vec<u8>)>| {
        let recipe = Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::WitnessClose { next: vec![2] },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(
            3,
            &[(
                0,
                OpMod {
                    compare: CompareMask {
                        windows: vec![RelWindow::new(0, 8), RelWindow::new(0x10, 8)],
                    },
                    ..OpMod::default()
                },
            )],
        ));
        let memory = MutatingMemory::new(build_memory(), 2, edits);
        observe(&[recipe], &memory, config())
            .expect("custody")
            .close_witnesses()
    };

    let closed = masked(vec![(OBJECT + 8, 0xEEu64.to_le_bytes().to_vec())]);
    assert!(
        closed.witness.changed.is_empty(),
        "a write to the live field between the compared words is not a torn read",
    );
    assert!(closed.snapshot.gap_reason(ConsumerId(1)).is_none());

    // The control: a write INSIDE a compared window still gaps, so the mask
    // narrows the comparison and does not disable it.
    let closed = masked(vec![(OBJECT, 0xEEu64.to_le_bytes().to_vec())]);
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(matches!(
        closed.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::WitnessChanged { .. })
    ));
}

/// A per-element mask over a decoded extent, stated once for the whole array
/// rather than once per element.
#[test]
fn an_extent_mask_is_per_element_and_costs_one_entry() {
    const HEADER: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    const STRIDE: u32 = 0x20;
    const COUNT: u32 = 64;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(HEADER, 0x20);
    inner.map_zeroed(TABLE, (COUNT * STRIDE) as usize);
    inner.write_u32(HEADER, COUNT);
    inner.write_u64(HEADER + 8, TABLE);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: STRIDE,
                max_count: 128,
                max_bytes: 1 << 20,
                trailer_elements: 0,
                guard: None,
                register: None,
                element: vec![2],
            },
            RecipeOp::WitnessClose { next: vec![3] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: HEADER,
        }],
    )
    .with_mods(mods(
        4,
        &[(
            1,
            OpMod {
                // Only the first four bytes of each 0x20-byte element.
                compare: CompareMask {
                    windows: vec![RelWindow::new(0, 4)],
                },
                ..OpMod::default()
            },
        )],
    ));

    // Write into the unconsumed tail of one element, between the waves.
    let memory = MutatingMemory::new(
        inner.clone(),
        3,
        vec![(TABLE + 5 * u64::from(STRIDE) + 8, vec![0xEE])],
    );
    let closed = observe(&[recipe.clone()], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(memory.mutation_applied(), "the fixture must actually mutate");
    assert!(
        closed.witness.changed.is_empty(),
        "the write is outside every element's consumed window",
    );

    // THE CONTROL, four bytes lower: inside the consumed window of the SAME
    // element. Without it, deleting the strided comparison entirely would still
    // pass this test — every write would look unconsumed.
    let memory = MutatingMemory::new(
        inner,
        3,
        vec![(TABLE + 5 * u64::from(STRIDE) + 1, vec![0xEE])],
    );
    let closed = observe(&[recipe], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(memory.mutation_applied());
    assert_eq!(
        closed.witness.changed.len(),
        1,
        "a write inside an element's consumed window must still gap",
    );
    // And the mask is one entry, not 64.
    let strided: usize = closed
        .snapshot
        .intervals()
        .iter()
        .filter_map(|record| closed.snapshot.annotations().consumption(record.id()))
        .map(|consumption| consumption.strided.len())
        .sum();
    assert_eq!(strided, 1, "a per-element mask is stated once for the array");
}

/// A declared closure order must actually happen, and it must differ from
/// interval-id order or the declaration means nothing.
#[test]
fn a_closing_program_runs_its_stages_in_the_declared_order() {
    const FIRST: u64 = 0x10_0000;
    const SECOND: u64 = 0x11_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(FIRST, 0x20);
    memory.map_zeroed(SECOND, 0x20);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            // op 0 copies FIRST and carries stage 2
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            // op 1 copies SECOND and carries stage 1
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            RecipeOp::WitnessClose { next: vec![3] },
            RecipeOp::Freeze,
        ],
        vec![
            Root {
                op: 0,
                addr: FIRST,
            },
            Root {
                op: 1,
                addr: SECOND,
            },
        ],
    )
    .with_mods(mods(
        4,
        &[
            (
                0,
                OpMod {
                    stage: 2,
                    ..OpMod::default()
                },
            ),
            (
                1,
                OpMod {
                    stage: 1,
                    ..OpMod::default()
                },
            ),
        ],
    ))
    .with_closing(ClosingProgram {
        stages: vec![
            CloseStage {
                stage: 1,
                elide_into: None,
            },
            CloseStage {
                stage: 2,
                elide_into: None,
            },
        ],
    });

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let order: Vec<u64> = build
        .closing_order
        .iter()
        .map(|id| {
            build
                .snapshot
                .intervals()
                .iter()
                .find(|record| record.id() == *id)
                .expect("record")
                .start()
        })
        .collect();
    assert_eq!(
        order,
        vec![SECOND, FIRST],
        "stage 1 closes before stage 2, which is not interval-id order",
    );
}

/// d81 elides its selector close in favour of the later source-position read
/// and keeps the selector's attribution. The elision has to be RECORDED, or the
/// failure of the covering close cannot be reported in the elided site's
/// clothes.
#[test]
fn an_elided_stage_is_recorded_against_the_stage_that_covers_it() {
    const SHARED: u64 = 0x10_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(SHARED, 0x20);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            RecipeOp::WitnessClose { next: vec![3] },
            RecipeOp::Freeze,
        ],
        // Both roots stand on the SAME address, so deduplication gives one
        // interval carrying both stages — which is exactly d81's coincident
        // selector and source slots.
        vec![
            Root {
                op: 0,
                addr: SHARED,
            },
            Root {
                op: 1,
                addr: SHARED,
            },
        ],
    )
    .with_mods(mods(
        4,
        &[
            (
                0,
                OpMod {
                    stage: 1,
                    ..OpMod::default()
                },
            ),
            (
                1,
                OpMod {
                    stage: 2,
                    ..OpMod::default()
                },
            ),
        ],
    ))
    .with_closing(ClosingProgram {
        stages: vec![
            CloseStage {
                stage: 1,
                elide_into: Some(2),
            },
            CloseStage {
                stage: 2,
                elide_into: None,
            },
        ],
    });

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(build.elisions.len(), 1);
    assert_eq!(build.elisions[0].elided, 1);
    assert_eq!(build.elisions[0].into, 2);
    assert_eq!(
        build.closing_order.len(),
        1,
        "one interval, one close: the elided stage does not take a second one",
    );
}

// ---------------------------------------------------------------------------
// RULING C. Value-conditional copy selection
// ---------------------------------------------------------------------------

/// The structural case: a discriminant in already-copied bytes says which of
/// two layouts the object has, and therefore which window to copy next.
///
/// MUTATION: make `evaluate_probe` return `Some(true)` unconditionally and the
/// inline arm is taken for the out-of-line object, so the copy lands on bytes
/// that are not the payload.
#[test]
fn a_conditional_copies_the_layout_its_probe_selects() {
    const INLINE: u64 = 0x10_0000;
    const OUTLINE: u64 = 0x11_0000;
    const PAYLOAD: u64 = 0x30_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(INLINE, 0x40);
    memory.map_zeroed(OUTLINE, 0x40);
    memory.map_zeroed(PAYLOAD, 0x40);
    // Discriminant at +0: 1 means the payload is inline, 0 means it is behind
    // the pointer at +0x10.
    memory.write_u32(INLINE, 1);
    memory.write_u64(INLINE + 0x20, 0xDEAD);
    memory.write_u32(OUTLINE, 0);
    memory.write_u64(OUTLINE + 0x10, PAYLOAD);
    memory.write_u64(PAYLOAD, 0xBEEF);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            // 0 the head, which is all the probe may look at
            RecipeOp::CopyHeader {
                size: 0x18,
                next: vec![1],
            },
            // 1 the conditional
            RecipeOp::SelectByValue {
                probe: Probe::FieldEquals {
                    offset: 0,
                    width: 4,
                    value: 1,
                },
                then: 2,
                otherwise: Some(3),
            },
            // 2 inline: take the tail of the same object
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x20, 8)],
                next: vec![5],
            },
            // 3 out of line: take the pointer field, then follow it
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x10, 8)],
                next: vec![4],
            },
            RecipeOp::DecodePointer {
                offset: 0x10,
                guard: None,
                next: 6,
            },
            RecipeOp::Freeze,
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![5],
            },
        ],
        vec![
            Root {
                op: 0,
                addr: INLINE,
            },
            Root {
                op: 0,
                addr: OUTLINE,
            },
        ],
    );

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(refusal_of(&build, 1).is_none(), "{:?}", refusal_of(&build, 1));
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.u64_le(INLINE + 0x20).expect("inline tail copied"),
        0xDEAD,
    );
    assert_eq!(view.u64_le(PAYLOAD).expect("payload followed"), 0xBEEF);
    assert!(
        view.bytes(OUTLINE + 0x20, 8).is_err(),
        "the inline arm must not have run for the out-of-line object",
    );
}

/// The other conditional d81 needs: try the wide window, and on a DISAGREEMENT
/// between its two back-to-back images fall back to the narrow reads. The probe
/// compares copied bytes and nothing else leaves the op.
#[test]
fn a_conditional_can_fall_back_when_the_wide_windows_samples_disagree() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x40);
    inner.write_u64(OBJECT, 0x1234);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![1],
            },
            RecipeOp::SelectByValue {
                probe: Probe::SamplesAgree { offset: 0, len: 16 },
                // Agreed: the wide window stands, nothing more is copied.
                then: 2,
                // Disagreed: take the narrow scalars separately.
                otherwise: Some(3),
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 1)],
                next: vec![4],
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x20, 8)],
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    )
    .with_mods(mods(
        5,
        &[(
            0,
            OpMod {
                samples: SampleSpec {
                    count: 2,
                    // The reader resolves the disagreement itself, so it must
                    // NOT be gapped for it.
                    on_disagree: DisagreeAction::Record,
                },
                ..OpMod::default()
            },
        )],
    ));

    let memory = TearingMemory {
        inner,
        tear_on: std::cell::Cell::new(2),
        seen: std::cell::Cell::new(0),
        at: OBJECT + 3,
    };
    let build = observe(&[recipe.clone()], &memory, config()).expect("custody");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(
        view.bytes(OBJECT + 0x20, 8).is_ok(),
        "a disagreement must select the fallback shape",
    );

    // And when the two images agree, the fallback is not taken.
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x40);
    inner.write_u64(OBJECT, 0x1234);
    let quiet = TearingMemory {
        inner,
        tear_on: std::cell::Cell::new(0),
        seen: std::cell::Cell::new(0),
        at: OBJECT + 3,
    };
    let build = observe(&[recipe], &quiet, config()).expect("custody");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(
        view.bytes(OBJECT + 0x20, 8).is_err(),
        "the fallback must not run on a quiet frame",
    );
}

/// A conditional that quietly does nothing is a short result reported as
/// success, which is the one outcome the whole limit discipline exists to
/// prevent.
#[test]
fn a_conditional_with_no_arm_refuses_rather_than_stopping_quietly() {
    const OBJECT: u64 = 0x10_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(OBJECT, 0x20);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![1],
            },
            RecipeOp::SelectByValue {
                probe: Probe::FieldEquals {
                    offset: 0,
                    width: 4,
                    value: 0xFFFF,
                },
                then: 2,
                otherwise: None,
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x10, 8)],
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("refused");
    assert!(matches!(
        record.refusal,
        Refusal::ConditionalUnresolved { op: 1, .. }
    ));
}

// ---------------------------------------------------------------------------
// The structural checks on the vocabulary itself
// ---------------------------------------------------------------------------

/// A partial modifier table would apply an op's qualifications to its
/// neighbour, which is indistinguishable from a table an edit has misaligned.
#[test]
fn a_modifier_table_is_empty_or_complete() {
    let ops = vec![
        RecipeOp::CopyHeader {
            size: 8,
            next: vec![1],
        },
        RecipeOp::Freeze,
    ];
    let recipe = Recipe::new(RecipeId(1), ConsumerId(1), ops, vec![Root { op: 0, addr: 8 }])
        .with_mods(vec![OpMod::default()]);
    assert!(matches!(
        validate(&recipe),
        Err(Refusal::Malformed {
            reason: MalformedReason::ModsLengthMismatch { ops: 2, mods: 1 },
            ..
        })
    ));
}

/// A qualification an op cannot honour would be silently ignored, and a recipe
/// author would believe a bound was in force that nothing applies.
#[test]
fn a_modifier_on_an_op_that_cannot_honour_it_is_malformed() {
    let cases: Vec<(OpMod, &str)> = vec![
        (
            OpMod {
                count: Some(CountExpr {
                    terms: vec![],
                    constant: 4,
                }),
                ..OpMod::default()
            },
            "a count expression on a copy op",
        ),
        (
            OpMod {
                key_from_carry: true,
                ..OpMod::default()
            },
            "a join key from carry on a copy op",
        ),
        (
            OpMod {
                carry: Some(CarrySource::Constant(1)),
                ..OpMod::default()
            },
            "a carry source on a copy op",
        ),
    ];
    for (modifier, what) in cases {
        let recipe = Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 8,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: 8 }],
        )
        .with_mods(vec![modifier, OpMod::default()]);
        assert!(
            matches!(
                validate(&recipe),
                Err(Refusal::Malformed {
                    reason: MalformedReason::ModOnWrongOp { op: 0 },
                    ..
                })
            ),
            "{what} must be malformed",
        );
    }
}

/// The bounds ruling C rests on. A probe past
/// [`MAX_PROBE_COMPARE_BYTES`] is not a bounded read of copied bytes, and a
/// sample count past [`MAX_ADJACENT_SAMPLES`] is custody nobody admitted.
#[test]
fn the_conditional_and_sample_bounds_are_enforced_before_execution() {
    let wide = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::SelectByValue {
                probe: Probe::SamplesAgree {
                    offset: 0,
                    len: MAX_PROBE_COMPARE_BYTES + 1,
                },
                then: 0,
                otherwise: None,
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: 8 }],
    );
    assert!(matches!(
        validate(&wide),
        Err(Refusal::Malformed {
            reason: MalformedReason::ProbeTooWide { op: 1, .. },
            ..
        })
    ));

    let greedy = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: 8 }],
    )
    .with_mods(vec![
        OpMod {
            samples: SampleSpec {
                count: MAX_ADJACENT_SAMPLES + 1,
                on_disagree: DisagreeAction::Gap,
            },
            ..OpMod::default()
        },
        OpMod::default(),
    ]);
    assert!(matches!(
        validate(&greedy),
        Err(Refusal::Malformed {
            reason: MalformedReason::SampleCountOutOfRange { op: 0, .. },
            ..
        })
    ));
}

/// A mask window that reaches outside the object its op copies would narrow the
/// closing comparison to bytes the interval does not hold — i.e. compare
/// nothing and certify anyway.
#[test]
fn a_mask_that_reaches_outside_its_copy_is_malformed() {
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: 8 }],
    )
    .with_mods(vec![
        OpMod {
            compare: CompareMask {
                windows: vec![RelWindow::new(12, 8)],
            },
            ..OpMod::default()
        },
        OpMod::default(),
    ]);
    assert!(matches!(
        validate(&recipe),
        Err(Refusal::Malformed {
            reason: MalformedReason::BadCompareMask { op: 0 },
            ..
        })
    ));
}

/// Elision only points FORWARDS. A programme that could elide into an earlier
/// stage would be claiming a witness taken before the bytes it certifies.
#[test]
fn a_closing_program_cannot_elide_backwards_or_name_an_unused_stage() {
    let build = |stages: Vec<CloseStage>| {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 8,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: 8 }],
        )
        .with_mods(vec![
            OpMod {
                stage: 1,
                ..OpMod::default()
            },
            OpMod {
                stage: 2,
                ..OpMod::default()
            },
        ])
        .with_closing(ClosingProgram { stages })
    };
    assert!(matches!(
        validate(&build(vec![
            CloseStage {
                stage: 2,
                elide_into: None
            },
            CloseStage {
                stage: 1,
                elide_into: Some(2)
            },
        ])),
        Err(Refusal::Malformed {
            reason: MalformedReason::BadClosingProgram { stage: 1 },
            ..
        })
    ));
    assert!(matches!(
        validate(&build(vec![CloseStage {
            stage: 9,
            elide_into: None
        }])),
        Err(Refusal::Malformed {
            reason: MalformedReason::BadClosingProgram { stage: 9 },
            ..
        })
    ));
}

/// SENTINEL. A narrowed consumption with nothing in it must compare as CHANGED,
/// not as agreed.
///
/// "Nothing was compared, therefore nothing moved" is a witness certifying an
/// object it never looked at, and it is reachable the moment a mask resolves to
/// no bytes inside the interval it lands on. The executor prevents that state
/// (an empty resolved mask falls back to `consume_all`); this pins the fallback
/// underneath it, because the two defences fail in opposite directions and only
/// one of them is in the file a future mask change would touch.
#[test]
fn an_empty_consumption_compares_as_changed_not_as_agreed() {
    let consumption = super::annot::Consumption {
        full: false,
        spans: Vec::new(),
        strided: Vec::new(),
    };
    assert!(
        !consumption.images_agree(Span::new(0x1000, 8), &[0u8; 8], &[0u8; 8]),
        "an empty mask must not certify two identical images either: it is the \
         ABSENCE of a comparison that has to fail, not the result of one",
    );
}

/// SENTINEL. A mask whose windows land outside the request it qualifies makes
/// the interval fully consumed rather than emptily consumed.
///
/// This is the executor-side half of the defence above. A `CopyRanges` op whose
/// mask names bytes of a DIFFERENT window than the one that reached this
/// interval used to record `{full: false, spans: []}`.
#[test]
fn a_mask_that_resolves_to_nothing_inside_its_interval_consumes_all_of_it() {
    const A: u64 = 0x10_0000;
    const B: u64 = 0x20_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(A, 0x40);
    inner.map_zeroed(B, 0x40);
    inner.write_u64(B, 0x1234);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            // Two windows, far enough apart to be two intervals. The mask names
            // bytes of the FIRST one only, so it resolves to nothing inside the
            // second.
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 8), RelWindow::new(0x10_0000, 8)],
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: A }],
    )
    .with_mods(mods(
        3,
        &[(
            0,
            OpMod {
                compare: CompareMask {
                    windows: vec![RelWindow::new(0, 8)],
                },
                ..OpMod::default()
            },
        )],
    ));

    // FIRST, the quiet frame. This is the half that separates "fully consumed"
    // from "empty and failing closed": under the fail-closed fallback alone,
    // an interval no mask reaches would gap on EVERY frame, changed or not.
    let quiet = MutatingMemory::new(inner.clone(), u64::MAX, Vec::new());
    let closed = observe(&[recipe.clone()], &quiet, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        closed.witness.changed.is_empty() && closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "an unchanged interval must not gap merely because a mask missed it",
    );

    // THEN the write. It is outside every mask window, and it must still gap:
    // the mask said nothing about this interval, so all of it is consumed.
    let memory = MutatingMemory::new(inner, 3, vec![(B, 0xEEu64.to_le_bytes().to_vec())]);
    let closed = observe(&[recipe], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(memory.mutation_applied(), "the fixture must actually write");
    assert!(
        !closed.witness.changed.is_empty(),
        "an interval no mask reaches is fully consumed, so a write to it gaps",
    );
}

/// SENTINEL. A recipe that reaches an interval ANOTHER recipe already copied
/// cannot be told its A/B pair was taken.
///
/// Adjacent samples are taken while a hole is being filled. Deduplication means
/// the second recipe has no hole, so no second image exists and no disagreement
/// can be found — a consumer that asked for torn detection being told the bytes
/// are clean because nobody looked. The sampling is recorded as INCOMPLETE
/// instead, and an incomplete pair gaps.
///
/// MUTATION: delete the `reconcile_samples` call from the dependency attach and
/// consumer 2 publishes an image nothing certified.
#[test]
fn a_shared_interval_cannot_certify_a_pair_it_never_took() {
    const OBJECT: u64 = 0x10_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(OBJECT, 0x20);

    let ops = || {
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ]
    };
    // Recipe 1 copies first and asks for ONE image.
    let first = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops(),
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    );
    // Recipe 2 reaches the same bytes and asks for a PAIR. It will not get one:
    // by the time it arrives the interval exists and the moment has passed.
    let second = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        ops(),
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    )
    .with_mods(mods(
        3,
        &[(
            0,
            OpMod {
                samples: SampleSpec {
                    count: 2,
                    on_disagree: DisagreeAction::Gap,
                },
                ..OpMod::default()
            },
        )],
    ));

    let closed = observe(&[first, second], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        closed.witness.torn.is_empty(),
        "nothing about the BYTES is known to be wrong: no two images disagreed, \
         because there are not two images",
    );
    assert_eq!(
        closed.witness.uncertified,
        vec![(closed.witness.uncertified[0].0, ConsumerId(2))],
        "the pair recipe 2 asked for was never taken, and that is a failed \
         certification, not an absent one",
    );
    assert!(
        matches!(
            closed.snapshot.gap_reason(ConsumerId(2)),
            Some(GapReason::SamplesDisagreed { .. })
        ),
        "the consumer that asked for the pair is gapped",
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "and the consumer that asked for one image keeps its hour: a shared \
         interval must not export its neighbour's stricter requirement",
    );
}

/// SENTINEL, and the other half of the same rule. Two consumers share one
/// interval; A asks for two images and gets them, B asks for three. B is short
/// and A is not, on the same interval at the same moment — and the policies are
/// per consumer too, so A electing `Record` cannot be overruled by B, and B
/// electing `Gap` cannot be downgraded by A.
///
/// MUTATION: store `wanted`/`gap_on_disagree` interval-wide instead of per ask
/// and one of the two assertions below fails whichever way the flag is joined.
#[test]
fn one_consumers_stricter_sample_request_does_not_reach_its_neighbour() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x4444);

    let build = |id: u32, consumer: u16, count: u8, on_disagree: DisagreeAction| {
        Recipe::new(
            RecipeId(id),
            ConsumerId(consumer),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::WitnessClose { next: vec![2] },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(
            3,
            &[(
                0,
                OpMod {
                    samples: SampleSpec { count, on_disagree },
                    ..OpMod::default()
                },
            )],
        ))
    };

    // A copies first and gets its pair. B deduplicates onto it and wants three.
    let closed = observe(
        &[
            build(1, 1, 2, DisagreeAction::Record),
            build(2, 2, 3, DisagreeAction::Gap),
        ],
        &inner,
        config(),
    )
    .expect("custody")
    .close_witnesses();

    assert_eq!(
        closed
            .witness
            .uncertified
            .iter()
            .map(|(_, consumer)| *consumer)
            .collect::<Vec<_>>(),
        vec![ConsumerId(2)],
        "only the consumer whose own request went unmet is uncertified",
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "A asked for two images and got two: it is not short, and its \
         neighbour's third image is not its problem",
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(2)).is_some(),
        "B asked for three and got two",
    );
}

/// SENTINEL. A consumer that elected to resolve a disagreement ITSELF is not
/// gapped by one, even when its neighbour on the same interval would be.
///
/// `DisagreeAction::Record` exists for exactly one reader shape — d81's active
/// window samples up to three times and stops when two agree — and a policy
/// that its neighbour can overrule is not a policy.
///
/// MUTATION: make `opted_out_of_gapping` return false and consumer 1 is gapped
/// for a disagreement it said it would handle.
#[test]
fn a_consumer_that_resolves_disagreements_itself_is_not_gapped_by_one() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x5555);

    let build = |id: u32, consumer: u16, on_disagree: DisagreeAction| {
        Recipe::new(
            RecipeId(id),
            ConsumerId(consumer),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::WitnessClose { next: vec![2] },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(
            3,
            &[(
                0,
                OpMod {
                    samples: SampleSpec {
                        count: 2,
                        on_disagree,
                    },
                    ..OpMod::default()
                },
            )],
        ))
    };

    // Recipe 1 copies first and takes the pair; the SECOND image is torn.
    // Recipe 2 deduplicates onto the same interval and wants to be gapped.
    let memory = TearingMemory {
        inner,
        tear_on: std::cell::Cell::new(2),
        seen: std::cell::Cell::new(0),
        at: OBJECT + 3,
    };
    let closed = observe(
        &[
            build(1, 1, DisagreeAction::Record),
            build(2, 2, DisagreeAction::Gap),
        ],
        &memory,
        config(),
    )
    .expect("custody")
    .close_witnesses();

    assert_eq!(
        closed.witness.torn.len(),
        1,
        "the two images genuinely disagree: this is a statement about the bytes",
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "consumer 1 said it would resolve a disagreement itself, got {:?}",
        closed.snapshot.gap_reason(ConsumerId(1)),
    );
    assert!(
        matches!(
            closed.snapshot.gap_reason(ConsumerId(2)),
            Some(GapReason::SamplesDisagreed { .. })
        ),
        "and consumer 2, which asked to be gapped, is",
    );
}

/// SENTINEL. Two requests from the SAME consumer are judged separately.
///
/// A met gapping request and an unmet non-gapping one must not combine into an
/// unmet gapping request — which is what merging `wanted` by max and
/// `gap_on_disagree` by OR produces, and it gaps a consumer for an image only
/// the non-gapping request ever wanted.
///
/// MUTATION: restore the merge in `SampleImages::ask` and consumer 7 is gapped.
#[test]
fn two_requests_from_one_consumer_do_not_combine_into_a_third() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x6666);

    let build = |id: u32, count: u8, on_disagree: DisagreeAction| {
        Recipe::new(
            RecipeId(id),
            // THE SAME CONSUMER, reached by two recipes.
            ConsumerId(7),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::WitnessClose { next: vec![2] },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(
            3,
            &[(
                0,
                OpMod {
                    samples: SampleSpec { count, on_disagree },
                    ..OpMod::default()
                },
            )],
        ))
    };

    // Request A: two images, gap me. It is MET — two images are taken.
    // Request B: three images, I will handle it. It is UNMET, and non-gapping.
    let closed = observe(
        &[
            build(1, 2, DisagreeAction::Gap),
            build(2, 3, DisagreeAction::Record),
        ],
        &inner,
        config(),
    )
    .expect("custody")
    .close_witnesses();

    assert!(
        closed.witness.uncertified.is_empty(),
        "the only unmet request said it would resolve the shortfall itself: \
         {:?}",
        closed.witness.uncertified,
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(7)).is_none(),
        "the consumer's gapping request was satisfied, so it keeps its hour",
    );
}

/// Memory that refuses the Nth copy and serves every other.
#[derive(Debug)]
struct FlakyMemory {
    inner: SyntheticMemory,
    refuse_on: u64,
    seen: std::cell::Cell<u64>,
}

impl SnapshotMemory for FlakyMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        let seen = self.seen.get() + 1;
        self.seen.set(seen);
        if seen == self.refuse_on {
            return Err(MemoryRefusal {
                span: Span::new(addr, out.len() as u64),
            });
        }
        self.inner.copy_into(addr, out)
    }
}

/// SENTINEL. The joined sample spec decides HOW MANY IMAGES TO TAKE and is
/// never itself a request.
///
/// A coalesced copy serves the requests of several ops. Taking the maximum
/// count and the strictest policy is right for the copier — it must satisfy the
/// strictest op — and wrong as a consumer request: a recipe whose two ops ask
/// for `(2, Gap)` and `(3, Record)` would be credited with `(3, Gap)`, a
/// request neither op made, and gapped for a third image only the non-gapping
/// op ever wanted.
///
/// This is the COALESCED-SOURCE path, which two separate recipes cannot reach:
/// their requests deduplicate downstream instead of merging into one planned
/// copy. One recipe, one consumer, two ops at the same address.
///
/// MUTATION: record an ask from `attributes` in `take_adjacent_samples` and the
/// consumer is gapped.
#[test]
fn a_coalesced_copys_joined_spec_is_not_recorded_as_a_request() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x7777);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            // Two ops, same address, same width: one coalesced copy serving
            // two requests.
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![2],
            },
            RecipeOp::WitnessClose { next: vec![3] },
            RecipeOp::Freeze,
        ],
        vec![
            Root {
                op: 0,
                addr: OBJECT,
            },
            Root {
                op: 1,
                addr: OBJECT,
            },
        ],
    )
    .with_mods(mods(
        4,
        &[
            (
                0,
                OpMod {
                    // Satisfied: two images are taken.
                    samples: SampleSpec {
                        count: 2,
                        on_disagree: DisagreeAction::Gap,
                    },
                    ..OpMod::default()
                },
            ),
            (
                1,
                OpMod {
                    // Unmet — and it said it would handle that itself.
                    samples: SampleSpec {
                        count: 3,
                        on_disagree: DisagreeAction::Record,
                    },
                    ..OpMod::default()
                },
            ),
        ],
    ));

    // Copy 1 is the opening image, copy 2 the second, copy 3 the third — which
    // the engine refuses. Two images are held against a request for three.
    let memory = FlakyMemory {
        inner,
        refuse_on: 3,
        seen: std::cell::Cell::new(0),
    };
    let closed = observe(&[recipe], &memory, config())
        .expect("custody")
        .close_witnesses();

    assert!(
        closed.witness.uncertified.is_empty(),
        "the only unmet request elected to resolve the shortfall itself: {:?}",
        closed.witness.uncertified,
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "the consumer's GAPPING request asked for two images and got two: {:?}",
        closed.snapshot.gap_reason(ConsumerId(1)),
    );
}

/// SENTINEL. Narrowing a refused scoped copy must not walk off the top of the
/// address space.
///
/// A span whose exclusive end is past 2^64 is representable — `Span::end_excl`
/// is a `u128` for that reason — but its midpoint is not a `u64` sum. Wrapping
/// it splits the request into a tail page and a page at address ZERO, and the
/// second half then copies and OWNS unrelated bytes at the bottom of the
/// address space.
///
/// MUTATION: replace the checked split with `split as u64` and this test either
/// panics on the subtraction or reports a different refusal.
#[test]
fn narrowing_a_scoped_copy_at_the_top_of_the_address_space_refuses() {
    const TOP: u64 = u64::MAX - 0x1FFF;
    let mut memory = SyntheticMemory::new();
    // Mapped, readable, and one byte too short for the request: the copy is
    // refused, which is what sends it into the narrowing path.
    memory.map_zeroed(TOP, 0x1000);
    memory.map_zeroed(0, 0x1000);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x2000,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TOP }],
    )
    .with_mods(mods(
        2,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("an unrepresentable split refuses");
    assert!(
        matches!(record.refusal, Refusal::AddressNotRepresentable { .. }),
        "got {:?}",
        record.refusal,
    );
    // And nothing at the bottom of the address space was copied or owned.
    assert!(
        build
            .snapshot
            .intervals()
            .iter()
            .all(|interval| interval.start() >= TOP),
        "the narrowing must never reach address zero",
    );
}

/// SENTINEL. Every entry point that hands out bytes must consult the void set.
/// `translate` returns an arena offset rather than bytes, and it is the one that
/// is easy to forget.
#[test]
fn every_view_entry_point_names_a_void_rather_than_calling_it_uncaptured() {
    const A: u64 = 0x10_0000;
    const B: u64 = 0x11_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(A, 0x100);
    memory.map_zeroed(B, 0x100);
    memory.set_readable(B, false);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 0x40), RelWindow::new(0x1_0000, 0x40)],
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: A }],
    )
    .with_mods(mods(
        2,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));
    let build = observe(&[recipe], &memory, config()).expect("custody");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    for (what, refusal) in [
        ("bytes", view.bytes(B, 8).err()),
        ("read_into", view.read_into(B, &mut [0u8; 8]).err()),
        ("translate", view.translate(B).err()),
        ("sample_bytes", view.sample_bytes(B, 8, 1).err()),
        ("closing_bytes", view.closing_bytes(B, 8).err()),
    ] {
        assert!(
            matches!(refusal, Some(ViewRefusal::Unreadable { .. })),
            "{what} must name the void, got {refusal:?}",
        );
    }
}

/// SENTINEL, through the EXECUTOR. Two recipes share one interval; one declares
/// a mask and the other does not. A write to a byte only the mask excludes must
/// still gap, because the unmasked recipe reads it.
///
/// The `Annotations`-level test below states the same rule as a unit; this one
/// states it as a property of the copier, and it is the one that fails if the
/// attach pass ever stops recording the unmasked contributor.
#[test]
fn a_recipe_without_a_mask_keeps_its_neighbours_whole_comparison() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);

    let masked = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    )
    .with_mods(mods(
        3,
        &[(
            0,
            OpMod {
                compare: CompareMask {
                    windows: vec![RelWindow::new(0, 8)],
                },
                ..OpMod::default()
            },
        )],
    ));
    // Same object, same width, no mask at all.
    let plain = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    );

    // A byte the mask excludes, and which recipe 2 therefore reads.
    //
    // TRIGGER 2, and the number matters: recipe 1's opening copy is the first
    // and recipe 2 DEDUPLICATES onto it, so the second copy of the observation
    // is the closing one. A trigger of 3 would land after everything and the
    // fixture would write nothing at all — which is why `mutation_applied` is
    // asserted rather than assumed.
    let memory = MutatingMemory::new(inner, 2, vec![(OBJECT + 0x10, vec![0xEE])]);
    let closed = observe(&[masked, plain], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(memory.mutation_applied(), "the fixture must actually write");
    assert!(
        !closed.witness.changed.is_empty(),
        "one masked contributor cannot narrow an interval its neighbour reads whole",
    );
    assert!(
        closed.snapshot.gap_reason(ConsumerId(2)).is_some(),
        "and the reader that consumes those bytes is the one gapped",
    );
}

/// SENTINEL. An interval one contributor reads whole cannot be narrowed by
/// another contributor's mask. A mask that could would hide a real torn read
/// from the reader that consumes those bytes.
#[test]
fn an_unmasked_contributor_makes_the_whole_interval_consumed_again() {
    let mut annot = super::annot::Annotations::new();
    annot.arm();
    let id = IntervalId(0);
    annot.consume_spans(id, &[Span::new(0x1000, 8)]);
    assert!(!annot.consumption(id).expect("recorded").full);
    annot.consume_all(id);
    assert!(annot.consumption(id).expect("recorded").full);
    // And a later mask cannot un-say it.
    annot.consume_spans(id, &[Span::new(0x1000, 8)]);
    annot.consume_strided(
        id,
        StridedMask {
            base: 0x1000,
            stride: 8,
            count: 4,
            windows: vec![(0, 4)],
        },
    );
    let consumption = annot.consumption(id).expect("recorded");
    assert!(consumption.full);
    assert!(consumption.spans.is_empty() && consumption.strided.is_empty());
}
