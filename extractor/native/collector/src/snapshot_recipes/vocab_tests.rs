use super::annot::StridedMask;
use super::executor::{
    observe, validate, Admissibility, CarrySource, CloseStage, CloseWhen, ClosingProgram,
    CompareMask, CountExpr, CountTerm, DisagreeAction, ObservationConfig, OpMod, Probe, Recipe,
    RecipeOp, RecipeOutcome, RejectAction, RelWindow, Root, SampleSpec, SnapshotBuild, Tolerance,
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

#[test]
fn a_derived_extent_covers_the_probe_tail_a_single_count_field_would_miss() {
    const HEADER: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    const STRIDE: u32 = 0x18;
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
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops.clone(),
        vec![Root {
            op: 0,
            addr: HEADER,
        }],
    )
    .with_mods(table);

    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(
        refusal_of(&build, 1).is_none(),
        "the derived extent captures"
    );
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

    let plain = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops,
        vec![Root {
            op: 0,
            addr: HEADER,
        }],
    );
    let build = observe(&[plain], &memory, config()).expect("custody");
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(
        view.bytes(last, STRIDE as usize).is_err(),
        "a single count field is exactly the shortfall this mechanism exists to close",
    );
}

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
                size: 0x20,
                next: vec![3],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 4,
            },
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![5, 6],
            },
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
            RecipeOp::JoinBounded {
                table: TableId(1),
                key_offset: 0,
                key_width: 4,
                next: 7,
            },
            RecipeOp::CopyHeader {
                size: STRIDE,
                next: vec![8],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 10,
            },
            RecipeOp::Freeze,
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
    assert!(
        refusal_of(&build, 1).is_none(),
        "{:?}",
        refusal_of(&build, 1)
    );
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

#[test]
fn a_carried_key_join_can_anchor_on_the_interval_its_key_came_from() {
    const ROOT: u64 = 0x10_0000;
    const HOP: u64 = 0x11_0000;
    const HEADER: u64 = 0x12_0000;
    const TABLE: u64 = 0x20_0000;
    const INFO: u64 = 0x30_0000;
    const ROW_OBJECT: u64 = 0x40_0000;
    const OBJECT_STRIDE: u64 = 0x1000;
    const STRIDE: u32 = 0x10;
    const CONSUMED_OFFSET: i64 = 0x38;
    const CONSUMED_LEN: u32 = 0x10;

    let build_recipe = |anchor: bool| {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1, 2],
                },
                RecipeOp::DecodePointer {
                    offset: 0x08,
                    guard: None,
                    next: 3,
                },
                RecipeOp::DecodePointer {
                    offset: 0x10,
                    guard: None,
                    next: 11,
                },
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![5],
                },
                RecipeOp::CopyRanges {
                    windows: vec![RelWindow::new(CONSUMED_OFFSET, CONSUMED_LEN)],
                    next: vec![6],
                },
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
                RecipeOp::JoinBounded {
                    table: TableId(1),
                    key_offset: 0,
                    key_width: 4,
                    next: 7,
                },
                RecipeOp::CopyHeader {
                    size: STRIDE,
                    next: vec![8],
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 10,
                },
                RecipeOp::Freeze,
                RecipeOp::CopyHeader {
                    size: 0x10,
                    next: vec![9],
                },
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![12],
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 4,
                },
            ],
            vec![Root { op: 0, addr: ROOT }],
        )
        .with_mods(mods(
            13,
            &[
                (
                    2,
                    OpMod {
                        carry: Some(CarrySource::Field {
                            offset: 0x18,
                            width: 4,
                        }),
                        ..OpMod::default()
                    },
                ),
                (
                    6,
                    OpMod {
                        key_from_carry: true,
                        anchor_on_carry_holder: anchor,
                        ..OpMod::default()
                    },
                ),
            ],
        ))
    };

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.map_zeroed(HOP, 0x20);
    memory.map_zeroed(HEADER, 0x20);
    memory.map_zeroed(TABLE, 8 * STRIDE as usize);
    memory.map_zeroed(INFO, 0x80);
    memory.write_u32(ROOT + 0x18, 3);
    memory.write_u64(ROOT + 0x08, HEADER);
    memory.write_u64(ROOT + 0x10, HOP);
    memory.write_u64(HOP, INFO);
    memory.write_u32(HEADER, 8);
    memory.write_u64(HEADER + 0x08, TABLE);
    for slot in 0..8u64 {
        let object = ROW_OBJECT + slot * OBJECT_STRIDE;
        memory.map_zeroed(object, 0x10);
        memory.write_u64(object, 0xB000 + slot);
        memory.write_u64(TABLE + slot * u64::from(STRIDE), object);
    }

    let without = observe(&[build_recipe(false)], &memory, config()).expect("custody");
    let refusal = refusal_of(&without, 1).expect("the unanchored join must refuse");
    assert!(
        matches!(refusal.refusal, Refusal::FieldNotCaptured { .. }),
        "got {:?}",
        refusal.refusal
    );

    let with = observe(&[build_recipe(true)], &memory, config()).expect("custody");
    assert!(refusal_of(&with, 1).is_none(), "{:?}", refusal_of(&with, 1));
    let view = with.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.u64_le(ROW_OBJECT + 3 * OBJECT_STRIDE)
            .expect("row 3's object was reached through the anchored join"),
        0xB003,
    );
    assert!(
        view.bytes(INFO, 8).is_err(),
        "and the object's base was never copied: that is the whole point",
    );
}

#[test]
fn carry_slot_is_still_sixteen_bytes() {
    assert_eq!(std::mem::size_of::<super::executor::CarrySlot>(), 16);
}

#[test]
fn anchoring_on_a_constant_carry_is_refused_by_name() {
    const ROOT: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    const STRIDE: u32 = 8;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.map_zeroed(TABLE, 4 * STRIDE as usize);
    memory.write_u32(ROOT, 4);
    memory.write_u64(ROOT + 0x08, TABLE);

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
                ptr_offset: 0x08,
                stride: STRIDE,
                max_count: 64,
                max_bytes: 1 << 20,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(1)),
                element: vec![4],
            },
            RecipeOp::JoinBounded {
                table: TableId(1),
                key_offset: 0,
                key_width: 4,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: STRIDE,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: ROOT }],
    )
    .with_mods(mods(
        5,
        &[(
            2,
            OpMod {
                carry: Some(CarrySource::Constant(1)),
                key_from_carry: true,
                anchor_on_carry_holder: true,
                ..OpMod::default()
            },
        )],
    ));

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let refusal = refusal_of(&build, 1).expect("a constant carry has no holder");
    assert!(
        matches!(refusal.refusal, Refusal::CarryHolderUnknown { op: 2, .. }),
        "got {:?}",
        refusal.refusal
    );
}

#[test]
fn anchoring_without_a_carried_key_is_malformed() {
    let ops = vec![
        RecipeOp::CopyHeader {
            size: 0x20,
            next: vec![1],
        },
        RecipeOp::JoinBounded {
            table: TableId(1),
            key_offset: 0,
            key_width: 4,
            next: 2,
        },
        RecipeOp::CopyHeader {
            size: 8,
            next: vec![3],
        },
        RecipeOp::Freeze,
    ];
    let anchored_without_carried_key = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops.clone(),
        vec![Root {
            op: 0,
            addr: 0x10_0000,
        }],
    )
    .with_mods(mods(
        4,
        &[(
            1,
            OpMod {
                anchor_on_carry_holder: true,
                ..OpMod::default()
            },
        )],
    ));
    assert!(matches!(
        validate(&anchored_without_carried_key),
        Err(Refusal::Malformed {
            reason: MalformedReason::ModOnWrongOp { op: 1 },
            ..
        })
    ));

    let anchored_on_a_copy = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops,
        vec![Root {
            op: 0,
            addr: 0x10_0000,
        }],
    )
    .with_mods(mods(
        4,
        &[(
            0,
            OpMod {
                key_from_carry: true,
                anchor_on_carry_holder: true,
                ..OpMod::default()
            },
        )],
    ));
    assert!(matches!(
        validate(&anchored_on_a_copy),
        Err(Refusal::Malformed {
            reason: MalformedReason::ModOnWrongOp { .. },
            ..
        })
    ));
}

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
    memory.write_u32(OWNER_A + 0x10, 0);
    memory.write_u32(OWNER_B + 0x10, 1);
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
    assert!(
        refusal_of(&build, 1).is_none(),
        "{:?}",
        refusal_of(&build, 1)
    );
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
    assert!(
        view.bytes(GOOD, 8).is_ok(),
        "the admitted branch still runs"
    );
    assert!(
        view.bytes(OUTSIDE, 8).is_err(),
        "and the rejected pointer was not followed",
    );
}

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

#[test]
fn a_refusal_carries_the_stage_subject_index_and_value_that_caused_it() {
    const HEADER: u64 = 0x10_0000;
    const TABLE: u64 = 0x20_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HEADER, 0x20);
    memory.map_zeroed(TABLE, 4 * 8);
    memory.write_u32(HEADER, 4);
    memory.write_u64(HEADER + 8, TABLE);
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
    assert!(matches!(
        record.refusal,
        Refusal::JoinOutOfRange { key: 9, .. }
    ));
    assert_eq!(record.witness.stage, 42, "the stage the reader calls this");
    assert_eq!(record.witness.op, 2);
    assert_eq!(record.witness.subject, HEADER);
    assert_eq!(record.witness.raw, 9, "the key that was out of range");
    assert_eq!(record.witness.related, TABLE, "the table it indexed");
    assert_eq!(
        build.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::Refused(record)),
    );
}

/// MUTATION: make `acquire` return `Ok(Err(hole))` regardless of tolerance and
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
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops.clone(),
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

    let memory = build_memory();
    let strict = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops,
        vec![Root { op: 0, addr: A }],
    );
    let build = observe(&[strict], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("Required still refuses the recipe");
    assert!(matches!(record.refusal, Refusal::MemoryUnreadable { .. }));
}

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

    let closed = masked(vec![(OBJECT, 0xEEu64.to_le_bytes().to_vec())]);
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(matches!(
        closed.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::WitnessChanged { .. })
    ));
}

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
                compare: CompareMask {
                    windows: vec![RelWindow::new(0, 4)],
                },
                ..OpMod::default()
            },
        )],
    ));

    let memory = MutatingMemory::new(
        inner.clone(),
        3,
        vec![(TABLE + 5 * u64::from(STRIDE) + 8, vec![0xEE])],
    );
    let closed = observe(&[recipe.clone()], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        memory.mutation_applied(),
        "the fixture must actually mutate"
    );
    assert!(
        closed.witness.changed.is_empty(),
        "the write is outside every element's consumed window",
    );

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
    let strided: usize = closed
        .snapshot
        .intervals()
        .iter()
        .filter_map(|record| closed.snapshot.annotations().consumption(record.id()))
        .map(|consumption| consumption.strided.len())
        .sum();
    assert_eq!(
        strided, 1,
        "a per-element mask is stated once for the array"
    );
}

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
            Root { op: 0, addr: FIRST },
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
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::Final,
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
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::Final,
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

fn mid_walk_world(back_reference: bool) -> SyntheticMemory {
    const ROOT: u64 = 0x10_0000;
    const CHILD: u64 = 0x20_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(ROOT, 0x20);
    memory.map_zeroed(CHILD, 0x10);
    memory.write_u64(ROOT, if back_reference { ROOT } else { CHILD });
    memory.write_u64(ROOT + 0x08, 0xAAAA);
    memory.write_u64(CHILD, 0xBBBB);
    memory
}

const MID_WALK_ROOT: u64 = 0x10_0000;

fn mid_walk_recipe(root_when: CloseWhen) -> Recipe {
    Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1, 3],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![4],
            },
            RecipeOp::WitnessClose { next: vec![4] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: MID_WALK_ROOT,
        }],
    )
    .with_mods(mods(
        5,
        &[
            (
                0,
                OpMod {
                    stage: 1,
                    ..OpMod::default()
                },
            ),
            (
                2,
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
                elide_into: None,
                when: root_when,
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::Final,
            },
        ],
    })
}

#[test]
fn a_stage_that_closed_mid_walk_is_not_gapped_by_a_later_write() {
    const TRIGGER: u64 = 3;
    let edit = vec![(
        MID_WALK_ROOT + 0x08,
        0xFFFF_FFFF_FFFF_FFFFu64.to_le_bytes().to_vec(),
    )];

    let mid = MutatingMemory::new(mid_walk_world(false), TRIGGER, edit.clone());
    let build =
        observe(&[mid_walk_recipe(CloseWhen::AfterWave(0))], &mid, config()).expect("custody");
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
        order.first().copied(),
        Some(MID_WALK_ROOT),
        "the mid-walk close is the head of the sequence: {order:?}",
    );
    assert_eq!(order.len(), 2, "both intervals are closed exactly once");
    let mut once = build.closing_order.clone();
    once.sort();
    once.dedup();
    assert_eq!(
        once.len(),
        build.closing_order.len(),
        "no interval appears twice in the closing sequence",
    );
    assert_eq!(
        build.witnessed_intervals as usize,
        build.closing_order.len(),
        "the reported witness count is the sequence that ran",
    );
    let closed = build.close_witnesses();
    assert!(mid.mutation_applied(), "the fixture must actually write");
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "a write after the stage closed is not a torn read: {:?}",
        closed.snapshot.gap_reason(ConsumerId(1)),
    );

    let late = MutatingMemory::new(mid_walk_world(false), TRIGGER, edit);
    let closed = observe(&[mid_walk_recipe(CloseWhen::Final)], &late, config())
        .expect("custody")
        .close_witnesses();
    assert!(late.mutation_applied());
    assert!(
        matches!(
            closed.snapshot.gap_reason(ConsumerId(1)),
            Some(GapReason::WitnessChanged { .. }) | Some(GapReason::WitnessDependency { .. })
        ),
        "the same write under a final-wave close must still gap: {:?}",
        closed.snapshot.gap_reason(ConsumerId(1)),
    );
}

#[test]
fn a_wave_that_reaches_an_already_closed_interval_refuses() {
    let memory = mid_walk_world(true);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1, 3],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x08, 8)],
                next: vec![4],
            },
            RecipeOp::WitnessClose { next: vec![4] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: MID_WALK_ROOT,
        }],
    )
    .with_mods(mods(
        5,
        &[
            (
                0,
                OpMod {
                    stage: 1,
                    ..OpMod::default()
                },
            ),
            (
                2,
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
                elide_into: None,
                when: CloseWhen::AfterWave(0),
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::Final,
            },
        ],
    });

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("the reopened interval must refuse");
    match record.refusal {
        Refusal::ClosedIntervalReopened {
            span,
            closed_after_wave,
            reopened_in_wave,
            ..
        } => {
            assert_eq!(span.start, MID_WALK_ROOT);
            assert_eq!(closed_after_wave, 0);
            assert_eq!(reopened_in_wave, 1);
        }
        other => panic!("expected ClosedIntervalReopened, got {other:?}"),
    }
}

#[test]
fn a_stage_reached_after_its_declared_close_refuses() {
    let memory = mid_walk_world(false);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1, 3],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![4],
            },
            RecipeOp::WitnessClose { next: vec![4] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: MID_WALK_ROOT,
        }],
    )
    .with_mods(mods(
        5,
        &[(
            2,
            OpMod {
                stage: 1,
                ..OpMod::default()
            },
        )],
    ))
    .with_closing(ClosingProgram {
        stages: vec![CloseStage {
            stage: 1,
            elide_into: None,
            when: CloseWhen::AfterWave(0),
        }],
    });

    let build = observe(&[recipe], &memory, config()).expect("custody");
    let record = refusal_of(&build, 1).expect("a stage reached after its close must refuse");
    match record.refusal {
        Refusal::ClosedIntervalReopened {
            closed_after_wave,
            reopened_in_wave,
            ..
        } => {
            assert_eq!(closed_after_wave, 0, "the wave the stage declared");
            assert_eq!(reopened_in_wave, 1, "the wave it was actually reached in");
        }
        other => panic!("expected ClosedIntervalReopened, got {other:?}"),
    }
}

#[test]
fn a_pruned_duplicates_edge_onto_a_closed_parent_is_kept_and_still_invalidates() {
    const ROOT: u64 = 0x10_0000;
    const LEFT: u64 = 0x21_0000;
    const RIGHT: u64 = 0x22_0000;
    const SHARED: u64 = 0x30_0000;
    const TRIGGER: u64 = 4;

    let run = |left_first: bool| {
        let mut world = SyntheticMemory::new();
        world.map_zeroed(ROOT, 0x20);
        world.map_zeroed(LEFT, 0x20);
        world.map_zeroed(RIGHT, 0x20);
        world.map_zeroed(SHARED, 0x10);
        world.write_u64(ROOT, LEFT);
        world.write_u64(ROOT + 0x08, RIGHT);
        world.write_u64(LEFT, SHARED);
        world.write_u64(RIGHT, SHARED);
        world.write_u64(SHARED, 0xC0DE);
        let edit = vec![(
            RIGHT + 0x08,
            0xFFFF_FFFF_FFFF_FFFFu64.to_le_bytes().to_vec(),
        )];
        let memory = MutatingMemory::new(world, TRIGGER, edit);

        let recipe = Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: if left_first { vec![1, 2] } else { vec![2, 1] },
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 3,
                },
                RecipeOp::DecodePointer {
                    offset: 0x08,
                    guard: None,
                    next: 4,
                },
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![5],
                },
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![6],
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 7,
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 7,
                },
                RecipeOp::CopyHeader {
                    size: 0x10,
                    next: vec![8],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: ROOT }],
        )
        .with_mods(mods(
            9,
            &[(
                4,
                OpMod {
                    stage: 1,
                    ..OpMod::default()
                },
            )],
        ))
        .with_closing(ClosingProgram {
            stages: vec![CloseStage {
                stage: 1,
                elide_into: None,
                when: CloseWhen::AfterWave(1),
            }],
        });

        let build = observe(&[recipe], &memory, config()).expect("custody");
        assert!(
            refusal_of(&build, 1).is_none(),
            "a late edge onto a closed parent is not a refusal: {:?}",
            refusal_of(&build, 1)
        );
        let closed = build.close_witnesses();
        assert!(
            memory.mutation_applied(),
            "the fixture must actually have rewritten `right`"
        );
        let start_of = |id: IntervalId| {
            closed
                .snapshot
                .intervals()
                .iter()
                .find(|record| record.id() == id)
                .expect("record")
                .start()
        };
        let changed: Vec<u64> = closed
            .witness
            .changed
            .iter()
            .map(|id| start_of(*id))
            .collect();
        let invalidated: Vec<u64> = closed
            .witness
            .invalidated
            .iter()
            .map(|id| start_of(*id))
            .collect();
        (changed, invalidated)
    };

    for left_first in [true, false] {
        let (changed, invalidated) = run(left_first);
        assert_eq!(
            changed,
            vec![RIGHT],
            "left_first={left_first}: exactly the interval the fixture rewrote \
             between its two images"
        );
        assert!(
            invalidated.contains(&SHARED),
            "left_first={left_first}: `shared` was reached through `right` and the \
             edge recording it was attached after `right` closed: {invalidated:?}"
        );
    }
}

#[test]
fn a_field_lost_to_a_scoped_void_stops_the_branch_and_one_never_copied_refuses() {
    const TABLE: u64 = 0x10_0000;
    const READABLE: u64 = 0x20_0000;
    let page = SCOPED_VOID_GRANULARITY_BYTES;

    let world = || {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(TABLE, page as usize);
        memory.map_zeroed(TABLE + page, page as usize);
        memory.map_zeroed(READABLE, 0x10);
        memory.write_u64(TABLE, READABLE);
        memory.set_readable(TABLE + page, false);
        memory
    };

    let scoped = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, (2 * page) as u32)],
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 3,
            },
            RecipeOp::DecodePointer {
                offset: page as u32,
                guard: None,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TABLE }],
    )
    .with_mods(mods(
        5,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));

    let memory = world();
    let build = observe(&[scoped], &memory, config()).expect("custody");
    assert!(
        refusal_of(&build, 1).is_none(),
        "a read lost to the recipe own void ends its branch and nothing else: {:?}",
        refusal_of(&build, 1),
    );
    assert!(
        build
            .report_for(RecipeId(1))
            .expect("reported")
            .scoped_voids
            >= 1,
        "the fixture must actually void a page, or the arms are the same test",
    );
    let closed = build.close_witnesses();
    let view = closed
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("the consumer keeps its hour");
    assert!(
        view.bytes(READABLE, 0x10).is_ok(),
        "the readable half of the speculative copy still publishes",
    );

    let short = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 8)],
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: page as u32,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TABLE }],
    );
    let memory = world();
    let build = observe(&[short], &memory, config()).expect("custody");
    let record = refusal_of(&build, 2).expect("an uncopied field is a recipe defect");
    assert!(
        matches!(record.refusal, Refusal::FieldNotCaptured { .. }),
        "and it says so: {:?}",
        record.refusal,
    );

    let voider = Recipe::new(
        RecipeId(3),
        ConsumerId(3),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, (2 * page) as u32)],
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TABLE }],
    )
    .with_mods(mods(
        3,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));
    let borrower = Recipe::new(
        RecipeId(3),
        ConsumerId(4),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 8)],
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: page as u32,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TABLE }],
    );
    let memory = world();
    let build = observe(&[voider, borrower], &memory, config()).expect("custody");
    let record = match build
        .report_for(RecipeId(3))
        .map(|report| report.outcome)
        .expect("reported")
    {
        RecipeOutcome::Refused(record) => Some(record),
        RecipeOutcome::Captured => None,
    };
    let _ = record;
    let borrower_refused = build
        .reports
        .iter()
        .find(|report| report.consumer == ConsumerId(4))
        .map(|report| report.outcome);
    assert!(
        matches!(borrower_refused, Some(RecipeOutcome::Refused(_))),
        "a void another recipe recorded must not excuse this one's uncopied \
         field, even under a shared RecipeId: {borrower_refused:?}",
    );

    let straddler = Recipe::new(
        RecipeId(5),
        ConsumerId(5),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, page as u32 + 1)],
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: page as u32,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: TABLE }],
    )
    .with_mods(mods(
        4,
        &[(
            0,
            OpMod {
                tolerance: Tolerance::Scoped,
                ..OpMod::default()
            },
        )],
    ));
    let memory = world();
    let build = observe(&[straddler], &memory, config()).expect("custody");
    let outcome = build.report_for(RecipeId(5)).expect("reported").outcome;
    assert!(
        matches!(outcome, RecipeOutcome::Refused(_)),
        "a decode whose span reaches past everything the recipe asked for must \
         refuse, whatever a void it touches says: {outcome:?}",
    );
}

#[test]
fn a_closing_program_whose_waves_run_backwards_is_malformed() {
    let recipe = mid_walk_recipe(CloseWhen::Final);
    let backwards = Recipe::new(
        recipe.id,
        recipe.consumer,
        recipe.ops.clone(),
        recipe.roots.clone(),
    )
    .with_mods(recipe.mods.clone())
    .with_closing(ClosingProgram {
        stages: vec![
            CloseStage {
                stage: 1,
                elide_into: None,
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::AfterWave(0),
            },
        ],
    });
    assert!(
        validate(&backwards).is_err(),
        "a program whose waves run backwards down the list is malformed",
    );
    let forwards = Recipe::new(
        recipe.id,
        recipe.consumer,
        recipe.ops.clone(),
        recipe.roots.clone(),
    )
    .with_mods(recipe.mods.clone())
    .with_closing(ClosingProgram {
        stages: vec![
            CloseStage {
                stage: 1,
                elide_into: None,
                when: CloseWhen::AfterWave(0),
            },
            CloseStage {
                stage: 2,
                elide_into: None,
                when: CloseWhen::Final,
            },
        ],
    });
    assert!(validate(&forwards).is_ok());
}

#[test]
fn a_conditional_copies_the_layout_its_probe_selects() {
    const INLINE: u64 = 0x10_0000;
    const OUTLINE: u64 = 0x11_0000;
    const PAYLOAD: u64 = 0x30_0000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(INLINE, 0x40);
    memory.map_zeroed(OUTLINE, 0x40);
    memory.map_zeroed(PAYLOAD, 0x40);
    memory.write_u32(INLINE, 1);
    memory.write_u64(INLINE + 0x20, 0xDEAD);
    memory.write_u32(OUTLINE, 0);
    memory.write_u64(OUTLINE + 0x10, PAYLOAD);
    memory.write_u64(PAYLOAD, 0xBEEF);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x18,
                next: vec![1],
            },
            RecipeOp::SelectByValue {
                probe: Probe::FieldEquals {
                    offset: 0,
                    width: 4,
                    value: 1,
                },
                then: 2,
                otherwise: Some(3),
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0x20, 8)],
                next: vec![5],
            },
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
    assert!(
        refusal_of(&build, 1).is_none(),
        "{:?}",
        refusal_of(&build, 1)
    );
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
                then: 2,
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

#[test]
fn a_modifier_table_is_empty_or_complete() {
    let ops = vec![
        RecipeOp::CopyHeader {
            size: 8,
            next: vec![1],
        },
        RecipeOp::Freeze,
    ];
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops,
        vec![Root { op: 0, addr: 8 }],
    )
    .with_mods(vec![OpMod::default()]);
    assert!(matches!(
        validate(&recipe),
        Err(Refusal::Malformed {
            reason: MalformedReason::ModsLengthMismatch { ops: 2, mods: 1 },
            ..
        })
    ));
}

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
                elide_into: None,
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: 1,
                elide_into: Some(2),
                when: CloseWhen::Final,
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
            elide_into: None,
            when: CloseWhen::Final,
        }])),
        Err(Refusal::Malformed {
            reason: MalformedReason::BadClosingProgram { stage: 9 },
            ..
        })
    ));
}

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

    let quiet = MutatingMemory::new(inner.clone(), u64::MAX, Vec::new());
    let closed = observe(&[recipe.clone()], &quiet, config())
        .expect("custody")
        .close_witnesses();
    assert!(
        closed.witness.changed.is_empty() && closed.snapshot.gap_reason(ConsumerId(1)).is_none(),
        "an unchanged interval must not gap merely because a mask missed it",
    );

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
    let first = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        ops(),
        vec![Root {
            op: 0,
            addr: OBJECT,
        }],
    );
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

#[test]
fn two_requests_from_one_consumer_do_not_combine_into_a_third() {
    const OBJECT: u64 = 0x10_0000;
    let mut inner = SyntheticMemory::new();
    inner.map_zeroed(OBJECT, 0x20);
    inner.write_u64(OBJECT, 0x6666);

    let build = |id: u32, count: u8, on_disagree: DisagreeAction| {
        Recipe::new(
            RecipeId(id),
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
                    samples: SampleSpec {
                        count: 3,
                        on_disagree: DisagreeAction::Record,
                    },
                    ..OpMod::default()
                },
            ),
        ],
    ));

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

#[test]
fn narrowing_a_scoped_copy_at_the_top_of_the_address_space_refuses() {
    const TOP: u64 = u64::MAX - 0x1FFF;
    let mut memory = SyntheticMemory::new();
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
    assert!(
        build
            .snapshot
            .intervals()
            .iter()
            .all(|interval| interval.start() >= TOP),
        "the narrowing must never reach address zero",
    );
}

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

#[test]
fn an_unmasked_contributor_makes_the_whole_interval_consumed_again() {
    let mut annot = super::annot::Annotations::new();
    annot.arm();
    let id = IntervalId(0);
    annot.consume_spans(id, &[Span::new(0x1000, 8)]);
    assert!(!annot.consumption(id).expect("recorded").full);
    annot.consume_all(id);
    assert!(annot.consumption(id).expect("recorded").full);
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

/// SENTINEL for the ownership check on `read_uint`.
/// THE CONTROL ARM IS THE HALF THAT MAKES THIS A TEST OF OWNERSHIP. Widen A's
#[test]
fn a_recipe_cannot_decode_out_of_another_recipes_copy() {
    const ROOT: u64 = 0x10_0000;
    const TARGET: u64 = 0x30_0000;

    let build = |a_header: u32| {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(ROOT, 0x20);
        memory.map_zeroed(TARGET, 0x10);
        memory.write_u64(ROOT + 8, TARGET);
        let reader = Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: a_header,
                    next: vec![1],
                },
                RecipeOp::DecodePointer {
                    offset: 8,
                    guard: None,
                    next: 2,
                },
                RecipeOp::CopyHeader {
                    size: 0x10,
                    next: vec![3],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: ROOT }],
        );
        let neighbour = Recipe::new(
            RecipeId(2),
            ConsumerId(2),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x20,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: ROOT }],
        );
        observe(&[neighbour, reader], &memory, config()).expect("custody")
    };

    let refused = build(8);
    assert_eq!(
        refusal_of(&refused, 1).map(|record| record.refusal),
        Some(Refusal::FieldNotCaptured {
            recipe: RecipeId(1),
            span: Span::new(ROOT + 8, 8),
        }),
        "recipe 1 decoded a pointer out of bytes only recipe 2 asked for",
    );
    assert_eq!(
        refused.report_for(RecipeId(2)).expect("reported").outcome,
        RecipeOutcome::Captured,
        "and the recipe that DID own the bytes keeps its hour",
    );
    assert!(
        refused.snapshot.view_for(ConsumerId(1), HOUR).is_err(),
        "a refused recipe gaps its consumer",
    );
    assert!(
        refused
            .snapshot
            .view_for(ConsumerId(2), HOUR)
            .expect("consumer 2 is not gapped")
            .bytes(ROOT + 8, 8)
            .is_ok(),
        "the bytes are in the arena and owned — the refusal is about custody",
    );

    let allowed = build(0x10);
    assert_eq!(
        allowed.report_for(RecipeId(1)).expect("reported").outcome,
        RecipeOutcome::Captured,
    );
    assert!(
        allowed
            .snapshot
            .view_for(ConsumerId(1), HOUR)
            .expect("not gapped")
            .bytes(TARGET, 0x10)
            .is_ok(),
        "and the pointer it owns is followed",
    );
}

/// it resolves the interval itself and the ownership check has to be spelled at
#[test]
fn a_sample_probe_cannot_read_another_recipes_images() {
    const OBJECT: u64 = 0x10_0000;

    let build = |a_header: u32| {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(OBJECT, 0x40);
        memory.write_u64(OBJECT + 8, 0x1234);
        let sampled = OpMod {
            samples: SampleSpec {
                count: 2,
                on_disagree: DisagreeAction::Record,
            },
            ..OpMod::default()
        };
        let prober = Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: a_header,
                    next: vec![1],
                },
                RecipeOp::SelectByValue {
                    probe: Probe::SamplesAgree { offset: 8, len: 8 },
                    then: 2,
                    otherwise: Some(3),
                },
                RecipeOp::CopyRanges {
                    windows: vec![RelWindow::new(0x20, 8)],
                    next: vec![4],
                },
                RecipeOp::CopyRanges {
                    windows: vec![RelWindow::new(0x30, 8)],
                    next: vec![4],
                },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(5, &[(0, sampled.clone())]));
        let owner = Recipe::new(
            RecipeId(2),
            ConsumerId(2),
            vec![
                RecipeOp::CopyHeader {
                    size: 0x10,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: OBJECT,
            }],
        )
        .with_mods(mods(2, &[(0, sampled)]));
        observe(&[owner, prober], &memory, config()).expect("custody")
    };

    let refused = build(8);
    assert_eq!(
        refusal_of(&refused, 1).map(|record| record.refusal),
        Some(Refusal::ConditionalUnresolved {
            recipe: RecipeId(1),
            op: 1,
        }),
        "recipe 1 probed the stability of bytes only recipe 2 asked for",
    );
    assert_eq!(
        refused.report_for(RecipeId(2)).expect("reported").outcome,
        RecipeOutcome::Captured,
    );

    let allowed = build(0x10);
    assert_eq!(
        allowed.report_for(RecipeId(1)).expect("reported").outcome,
        RecipeOutcome::Captured,
    );
    let view = allowed
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("not gapped");
    assert!(
        view.bytes(OBJECT + 0x20, 8).is_ok() && view.bytes(OBJECT + 0x30, 8).is_err(),
        "the agreeing arm ran and the fallback did not",
    );
}
