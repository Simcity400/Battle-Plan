use super::arena::{Arena, ArenaAllocator};
use super::dedup::OwnershipMap;
use super::executor::{
    observe, observe_in, validate, ExtentGuard, ObservationConfig, Recipe, RecipeOp, RecipeOutcome,
    RecipeReport, RelWindow, Root, SnapshotBuild,
};
use super::policy::{
    merge_allowance, plan_copies, AnchorKind, CopyRequest, GapPolicy, PolicyRefusal, Proof,
    Provenance,
};
use super::synthetic::{MutatingMemory, SyntheticMemory};
use super::view::ViewRefusal;
use super::*;

const BASE: u64 = 0x10_0000;
const NODE_STRIDE: u64 = 0x1000;
const HOUR: HourStamp = HourStamp(19390901);

fn config() -> ObservationConfig {
    ObservationConfig::new(HOUR)
}

fn node_addr(index: usize) -> u64 {
    BASE + index as u64 * NODE_STRIDE
}

fn chain_memory(nodes: usize, tail: u64) -> SyntheticMemory {
    let mut memory = SyntheticMemory::new();
    for index in 0..nodes {
        memory.map_zeroed(node_addr(index), 16);
    }
    for index in 0..nodes {
        let next = if index + 1 < nodes {
            node_addr(index + 1)
        } else {
            tail
        };
        memory.write_u64(node_addr(index), next);
        memory.write_u64(node_addr(index) + 8, 0xA0 + index as u64);
    }
    memory
}

fn chain_recipe(id: u32, consumer: u16) -> Recipe {
    Recipe::new(
        RecipeId(id),
        ConsumerId(consumer),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 0,
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: node_addr(0),
        }],
    )
}

fn outcome(build: &SnapshotBuild, id: u32) -> RecipeOutcome {
    build
        .report_for(RecipeId(id))
        .expect("recipe reported")
        .outcome
}

fn refusal(build: &SnapshotBuild, id: u32) -> Refusal {
    match outcome(build, id) {
        RecipeOutcome::Refused(record) => record.refusal,
        RecipeOutcome::Captured => panic!("recipe {id} was captured, expected a refusal"),
    }
}

fn request(start: u64, len: u64, provenance: Provenance) -> CopyRequest {
    CopyRequest {
        span: Span::new(start, len),
        provenance,
        task: 0,
    }
}

fn request_from(start: u64, len: u64, provenance: Provenance, task: u32) -> CopyRequest {
    CopyRequest {
        span: Span::new(start, len),
        provenance,
        task,
    }
}

#[test]
fn proximity_alone_never_merges() {
    let policy = GapPolicy::default();
    let unrelated_roots = vec![
        request(0x1000, 8, Provenance::from_root(0x1000)),
        request(0x1010, 8, Provenance::from_root(0x1010)),
    ];
    assert_eq!(plan_copies(&unrelated_roots, &policy).len(), 2);

    let unrelated_pointers = vec![
        request(0x1000, 8, Provenance::from_pointer(0xAAAA)),
        request(0x1010, 8, Provenance::from_pointer(0xBBBB)),
    ];
    assert_eq!(plan_copies(&unrelated_pointers, &policy).len(), 2);

    let unrelated_objects = vec![
        request(
            0x1000,
            8,
            Provenance::new(
                AnchorKind::Root(0x1000),
                Proof::Object {
                    base: 0x1000,
                    len: 8,
                },
            ),
        ),
        request(
            0x1010,
            8,
            Provenance::new(
                AnchorKind::Root(0x1010),
                Proof::Object {
                    base: 0x1010,
                    len: 8,
                },
            ),
        ),
    ];
    assert_eq!(plan_copies(&unrelated_objects, &policy).len(), 2);
}

#[test]
fn root_addresses_are_not_window_anchors() {
    let policy = GapPolicy::default();
    let from_root = vec![
        request(0x2000, 8, Provenance::from_root(0x2000)),
        request(0x2010, 8, Provenance::from_root(0x2000)),
    ];
    assert_eq!(
        plan_copies(&from_root, &policy).len(),
        2,
        "two reads sharing a ROOT address are not thereby one object"
    );

    let from_pointer = vec![
        request(0x2000, 8, Provenance::from_pointer(0x2000)),
        request(0x2010, 8, Provenance::from_pointer(0x2000)),
    ];
    assert_eq!(
        plan_copies(&from_pointer, &policy).len(),
        1,
        "the same pair reached through one dereferenced pointer is the generic tier"
    );
}

#[test]
fn generic_tier_is_thirty_two_bytes_exactly() {
    let policy = GapPolicy::default();
    let anchor = Provenance::from_pointer(0x9000);
    let at_limit = vec![
        request(0x1000, 8, anchor),
        request(0x1000 + 8 + 32, 8, anchor),
    ];
    assert_eq!(plan_copies(&at_limit, &policy).len(), 1);

    let past_limit = vec![
        request(0x1000, 8, anchor),
        request(0x1000 + 8 + 33, 8, anchor),
    ];
    assert_eq!(plan_copies(&past_limit, &policy).len(), 2);
}

#[test]
fn proven_object_tier_is_sixty_four_bytes_exactly() {
    let policy = GapPolicy::default();
    let prov = Provenance::new(
        AnchorKind::Root(0xDEAD),
        Proof::Object {
            base: 0x1000,
            len: 0x200,
        },
    );

    let at_limit = vec![request(0x1000, 8, prov), request(0x1000 + 8 + 64, 8, prov)];
    assert_eq!(plan_copies(&at_limit, &policy).len(), 1);

    let past_limit = vec![request(0x1000, 8, prov), request(0x1000 + 8 + 65, 8, prov)];
    assert_eq!(plan_copies(&past_limit, &policy).len(), 2);
}

#[test]
fn proven_extent_tier_uses_the_configured_window() {
    let policy = GapPolicy::new(32, 64, 512).expect("monotonic policy");
    let prov = Provenance::new(
        AnchorKind::Root(0xFEED),
        Proof::Extent {
            base: 0x4000,
            len: 0x4000,
        },
    );
    let at_limit = vec![request(0x4000, 8, prov), request(0x4000 + 8 + 512, 8, prov)];
    assert_eq!(plan_copies(&at_limit, &policy).len(), 1);

    let past_limit = vec![request(0x4000, 8, prov), request(0x4000 + 8 + 513, 8, prov)];
    assert_eq!(plan_copies(&past_limit, &policy).len(), 2);
}

#[test]
fn a_proven_merge_may_not_leave_its_proven_region() {
    let policy = GapPolicy::default();
    let prov = Provenance::new(
        AnchorKind::Root(0xC0DE),
        Proof::Object {
            base: 0x1000,
            len: 0x20,
        },
    );
    let escaping = vec![request(0x1000, 8, prov), request(0x1018, 0x40, prov)];
    assert_eq!(plan_copies(&escaping, &policy).len(), 2);

    let contained = vec![request(0x1000, 8, prov), request(0x1018, 8, prov)];
    assert_eq!(plan_copies(&contained, &policy).len(), 1);
}

#[test]
fn touching_and_overlapping_ranges_merge_without_any_evidence() {
    let policy = GapPolicy::exact();
    let touching = vec![
        request(0x1000, 8, Provenance::from_root(0x1000)),
        request(0x1008, 8, Provenance::from_root(0x2222)),
    ];
    let planned = plan_copies(&touching, &policy);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned.copies[0].span, Span::new(0x1000, 16));

    let overlapping = vec![
        request(0x1000, 16, Provenance::from_root(0x1000)),
        request(0x1008, 16, Provenance::from_root(0x2222)),
    ];
    let planned = plan_copies(&overlapping, &policy);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned.copies[0].span, Span::new(0x1000, 24));
}

#[test]
fn the_exact_policy_bridges_nothing() {
    let policy = GapPolicy::exact();
    let anchor = Provenance::from_pointer(0x77);
    let one_byte_hole = vec![request(0x1000, 8, anchor), request(0x1009, 8, anchor)];
    assert_eq!(plan_copies(&one_byte_hole, &policy).len(), 2);
}

#[test]
fn merged_provenance_degrades_to_mixed() {
    let policy = GapPolicy::exact();
    let mixed = vec![
        request(0x1000, 8, Provenance::from_pointer(0x1)),
        request(0x1008, 8, Provenance::from_pointer(0x2)),
    ];
    let planned = plan_copies(&mixed, &policy);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned.copies[0].provenance.anchor, AnchorKind::Mixed);
    assert_eq!(planned.copies[0].provenance.proof, Proof::None);
    assert_eq!(
        merge_allowance(
            &planned.copies[0].provenance,
            &Provenance::from_pointer(0x1),
            Span::new(0x1000, 0x40),
            &GapPolicy::default(),
        ),
        0
    );
}

#[test]
fn policy_construction_rejects_incoherent_tables() {
    assert_eq!(
        GapPolicy::new(64, 32, 128),
        Err(PolicyRefusal::NonMonotonic {
            generic: 64,
            proven_object: 32,
            proven_extent: 128,
        })
    );
    assert_eq!(
        GapPolicy::new(32, 64, MAX_PROVEN_WINDOW_GAP_BYTES + 1),
        Err(PolicyRefusal::WindowTooWide {
            configured: MAX_PROVEN_WINDOW_GAP_BYTES + 1,
            cap: MAX_PROVEN_WINDOW_GAP_BYTES,
        })
    );
    let default = GapPolicy::default();
    assert_eq!(default.generic(), GENERIC_GAP_BYTES);
    assert_eq!(default.proven_object(), PROVEN_OBJECT_GAP_BYTES);
    assert_eq!(default.proven_extent(), DEFAULT_PROVEN_EXTENT_GAP_BYTES);
}

#[test]
fn planning_is_order_independent() {
    let policy = GapPolicy::default();
    let anchor = Provenance::from_pointer(0x5000);
    let forward = vec![
        request(0x1000, 8, anchor),
        request(0x1010, 8, anchor),
        request(0x9000, 8, anchor),
    ];
    let backward = vec![
        request(0x9000, 8, anchor),
        request(0x1010, 8, anchor),
        request(0x1000, 8, anchor),
    ];
    let a = plan_copies(&forward, &policy);
    let b = plan_copies(&backward, &policy);
    assert_eq!(
        a.copies.iter().map(|c| c.span).collect::<Vec<_>>(),
        b.copies.iter().map(|c| c.span).collect::<Vec<_>>()
    );
    assert_eq!(a.len(), 2);
}

#[test]
fn identical_spans_are_ordered_by_the_lowest_request_index_last() {
    let policy = GapPolicy::default();
    let anchor = Provenance::from_pointer(0x5000);
    const FIRST_ASKER: u32 = 0x77;
    const SECOND_ASKER: u32 = 0x99;
    let ascending = vec![
        request_from(0x1000, 8, anchor, 1),
        request_from(0x2000, 8, anchor, FIRST_ASKER),
        request_from(0x2000, 8, anchor, SECOND_ASKER),
        request_from(0x3000, 8, anchor, 3),
    ];
    let shuffled = vec![
        request_from(0x3000, 8, anchor, 3),
        request_from(0x2000, 8, anchor, FIRST_ASKER),
        request_from(0x2000, 8, anchor, SECOND_ASKER),
        request_from(0x1000, 8, anchor, 1),
    ];

    let named = |requests: &[CopyRequest]| -> u32 {
        let plan = plan_copies(requests, &policy);
        let copy = plan
            .copies
            .iter()
            .find(|copy| copy.span == Span::new(0x2000, 8))
            .expect("the identical pair is one transaction");
        assert_eq!(
            copy.sources_len, 2,
            "both requests must land in the same transaction, or there is no tie to break",
        );
        *plan.sources_of(copy).last().expect("a non-empty run")
    };

    assert_eq!(
        named(&ascending),
        FIRST_ASKER,
        "the lowest-indexed contributor is the one the witness names",
    );
    assert_eq!(
        named(&ascending),
        named(&shuffled),
        "and it does not depend on which of the coalescer's two paths ran",
    );
}

#[test]
fn a_depth_twelve_chain_takes_thirteen_forward_waves() {
    let memory = chain_memory(13, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.forward_waves_run, 13);
    assert!(build.forward_waves_run < FORWARD_WAVES);
    assert_eq!(
        build.report_for(RecipeId(1)).unwrap().copy_tasks,
        13,
        "one copy task per node"
    );
    assert_eq!(build.phases_run, 13);
    assert_eq!(build.witnessed_intervals, 0);
}

#[test]
fn depth_past_the_limit_refuses_and_does_not_truncate() {
    let memory = chain_memory(14, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    let expected = Refusal::DepthExceeded {
        recipe: RecipeId(1),
        limit: MEASURED_MAX_DISCOVERY_DEPTH,
        wanted_depth: 13,
    };
    assert_eq!(refusal(&build, 1), expected);
    assert_eq!(
        build.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::Refused(RefusalRecord::bare(expected))),
        "a refused recipe gaps its consumer, it does not deliver 13 of 14 nodes"
    );
}

#[test]
fn a_lower_depth_limit_is_honoured() {
    let memory = chain_memory(13, 0);
    let mut recipe = chain_recipe(1, 1);
    recipe.limits.max_depth = 3;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::DepthExceeded {
            recipe: RecipeId(1),
            limit: 3,
            wanted_depth: 4,
        }
    );
}

#[test]
fn a_pointer_cycle_terminates_without_refusing() {
    let memory = chain_memory(4, node_addr(0));
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.report_for(RecipeId(1)).unwrap().copy_tasks, 4);
    assert_eq!(build.snapshot.interval_count(), 4);
    assert_eq!(memory.copies(), 4, "the cycle re-copies nothing");
}

#[test]
fn a_self_pointer_terminates() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(node_addr(0), 16);
    memory.write_u64(node_addr(0), node_addr(0));
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.report_for(RecipeId(1)).unwrap().copy_tasks, 1);
}

#[test]
fn a_null_pointer_ends_a_branch_without_refusing() {
    let memory = chain_memory(1, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.report_for(RecipeId(1)).unwrap().frozen_branches, 1);
}

#[test]
fn the_byte_cap_refuses_rather_than_stopping_early() {
    let memory = chain_memory(13, 0);
    let mut recipe = chain_recipe(1, 1);
    recipe.limits.max_bytes = 40;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::ByteLimitExceeded {
            recipe: RecipeId(1),
            limit: 40,
            wanted: 48,
        }
    );
}

#[test]
fn the_range_cap_counts_post_coalescing_transactions() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x1000);
    let mut recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![
                    RelWindow::new(0, 8),
                    RelWindow::new(0x100, 8),
                    RelWindow::new(0x200, 8),
                ],
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    recipe.limits.max_ranges = 2;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::RangeLimitExceeded {
            recipe: RecipeId(1),
            limit: 2,
            wanted: 3,
        }
    );
}

#[test]
fn coalescing_reduces_the_charged_range_count() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 8);
    memory.map_zeroed(BASE + 0x400, 0x100);
    memory.write_u64(BASE, BASE + 0x400);
    let mut recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyRanges {
                windows: vec![
                    RelWindow::new(0, 8),
                    RelWindow::new(0x20, 8),
                    RelWindow::new(0x40, 8),
                ],
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    recipe.limits.max_ranges = 2;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    let report = build.report_for(RecipeId(1)).unwrap();
    assert_eq!(
        report.planned_ranges, 2,
        "the header plus one merged window"
    );
    assert_eq!(report.planned_bytes, 8 + 0x48);
}

#[test]
fn a_single_range_wider_than_the_cap_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x2000);
    let mut recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x1000,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    recipe.limits.max_single_range_bytes = 0x800;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::RangeTooWide {
            recipe: RecipeId(1),
            limit: 0x800,
            wanted: 0x1000,
        }
    );
}

#[test]
fn the_copy_task_cap_refuses() {
    let memory = chain_memory(13, 0);
    let mut recipe = chain_recipe(1, 1);
    recipe.limits.max_copy_tasks = 3;
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::CopyTaskLimitExceeded {
            recipe: RecipeId(1),
            limit: 3,
        }
    );
}

fn extent_memory(count: u32, leaves: u32) -> SyntheticMemory {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 16);
    memory.map_zeroed(BASE + 0x1000, (count.max(1) as usize) * 8);
    memory.write_u32(BASE, count);
    memory.write_u64(BASE + 8, BASE + 0x1000);
    for index in 0..leaves {
        let leaf = BASE + 0x2000 + u64::from(index) * 0x100;
        memory.map_zeroed(leaf, 8);
        memory.write_u64(leaf, 0xBEEF_0000 + u64::from(index));
        memory.write_u64(BASE + 0x1000 + u64::from(index) * 8, leaf);
    }
    memory
}

fn extent_recipe(max_count: u32, max_bytes: u64) -> Recipe {
    Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: 8,
                max_count,
                max_bytes,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(1)),
                element: vec![2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    )
}

#[test]
fn an_extent_is_copied_as_one_transaction_and_its_elements_run_in_the_same_wave() {
    let memory = extent_memory(4, 4);
    let build = observe(&[extent_recipe(16, 4096)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.forward_waves_run, 3);
    let report = build.report_for(RecipeId(1)).unwrap();
    assert_eq!(report.copy_tasks, 1 + 1 + 4);
    assert_eq!(report.frozen_branches, 4);
    assert_eq!(
        memory.copies(),
        6,
        "header + array + four leaves, the array in ONE copy not four"
    );
}

#[test]
fn an_extent_past_its_element_cap_refuses() {
    let memory = extent_memory(10, 10);
    let build = observe(&[extent_recipe(4, 4096)], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::ExtentCountExceeded {
            recipe: RecipeId(1),
            limit: 4,
            decoded: 10,
        }
    );
}

#[test]
fn an_extent_past_its_byte_cap_refuses() {
    let memory = extent_memory(10, 10);
    let build = observe(&[extent_recipe(16, 32)], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::ExtentBytesExceeded {
            recipe: RecipeId(1),
            limit: 32,
            decoded: 80,
        }
    );
}

#[test]
fn an_empty_extent_is_not_a_refusal() {
    let memory = extent_memory(0, 0);
    let build = observe(&[extent_recipe(16, 4096)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
}

fn join_fixture(key: u32) -> (SyntheticMemory, Recipe) {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 24);
    memory.write_u32(BASE, 4);
    memory.write_u64(BASE + 8, BASE + 0x1000);
    memory.write_u32(BASE + 16, key);
    memory.map_zeroed(BASE + 0x1000, 4 * 16);
    for index in 0..4u64 {
        memory.write_u64(BASE + 0x1000 + index * 16, 0x5000 + index);
    }
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 24,
                next: vec![1, 2, 4],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: 16,
                max_count: 16,
                max_bytes: 4096,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(7)),
                element: vec![4],
            },
            RecipeOp::JoinBounded {
                table: TableId(7),
                key_offset: 16,
                key_width: 4,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    (memory, recipe)
}

#[test]
fn a_bounded_join_indexes_the_registered_table() {
    let (memory, recipe) = join_fixture(2);
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    let closed = build.close_witnesses();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(view.u64_le(BASE + 0x1000 + 32).unwrap(), 0x5002);
}

#[test]
fn a_join_key_past_the_table_refuses() {
    let (memory, recipe) = join_fixture(9);
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::JoinOutOfRange {
            recipe: RecipeId(1),
            table: TableId(7),
            key: 9,
            count: 4,
        }
    );
}

#[test]
fn a_join_against_an_unregistered_table_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 24);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 24,
                next: vec![1],
            },
            RecipeOp::JoinBounded {
                table: TableId(3),
                key_offset: 16,
                key_width: 4,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::JoinTableMissing {
            recipe: RecipeId(1),
            table: TableId(3),
        }
    );
}

const STR_OBJ: u64 = 0x20_0000;
const STR_HEAP: u64 = 0x30_0000;

fn msvc_string_memory(heap: Option<&[u8]>) -> SyntheticMemory {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(STR_OBJ, 0x20);
    memory.map_zeroed(STR_HEAP, 0x100);
    match heap {
        Some(text) => {
            memory.write_u64(STR_OBJ, STR_HEAP);
            memory.write_u64(STR_OBJ + 0x10, text.len() as u64);
            memory.write_u64(STR_OBJ + 0x18, 31);
            memory.write(STR_HEAP, text);
        }
        None => {
            let inline = b"short";
            memory.write(STR_OBJ, inline);
            memory.write_u64(STR_OBJ + 0x10, inline.len() as u64);
            memory.write_u64(STR_OBJ + 0x18, 15);
        }
    }
    memory
}

fn msvc_string_recipe(guard: Option<ExtentGuard>, trailer: u32) -> Recipe {
    Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x20,
                next: vec![1],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0x10,
                count_width: 8,
                ptr_offset: 0x00,
                stride: 1,
                max_count: 1 << 20,
                max_bytes: 1 << 20,
                trailer_elements: trailer,
                guard,
                register: None,
                element: Vec::new(),
            },
        ],
        vec![Root {
            op: 0,
            addr: STR_OBJ,
        }],
    )
}

fn capacity_guard() -> Option<ExtentGuard> {
    Some(ExtentGuard::at_least(0x18, 8, 16))
}

#[test]
fn a_short_string_is_captured_without_dereferencing_its_characters() {
    let memory = msvc_string_memory(None);

    let unguarded = observe(&[msvc_string_recipe(None, 1)], &memory, config()).expect("custody");
    assert!(
        matches!(refusal(&unguarded, 1), Refusal::MemoryUnreadable { .. }),
        "an unguarded extent must dereference the characters and refuse, or this \
         test is not watching the failure the guard prevents: got {:?}",
        outcome(&unguarded, 1)
    );

    let guarded = observe(
        &[msvc_string_recipe(capacity_guard(), 1)],
        &memory,
        config(),
    )
    .expect("custody");
    assert_eq!(outcome(&guarded, 1), RecipeOutcome::Captured);
    let view = guarded
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("a captured consumer has a view");
    assert_eq!(view.bytes(STR_OBJ, 5).expect("inline characters"), b"short");
    assert!(matches!(
        view.bytes(0x0000_0074_726F_6873, 1),
        Err(ViewRefusal::NotCaptured { .. })
    ));
}

#[test]
fn a_long_string_is_dereferenced_and_its_payload_is_captured() {
    let text = b"a rather longer string than sixteen bytes";
    let memory = msvc_string_memory(Some(text));
    let build = observe(
        &[msvc_string_recipe(capacity_guard(), 1)],
        &memory,
        config(),
    )
    .expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    let view = build.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(view.bytes(STR_HEAP, text.len()).expect("payload"), text);
}

#[test]
fn the_trailer_puts_the_terminator_inside_the_copy() {
    let text = b"a rather longer string than sixteen bytes";
    let memory = msvc_string_memory(Some(text));

    let without = observe(
        &[msvc_string_recipe(capacity_guard(), 0)],
        &memory,
        config(),
    )
    .expect("custody");
    let view = without
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("view");
    assert!(matches!(
        view.bytes(STR_HEAP + text.len() as u64, 1),
        Err(ViewRefusal::NotCaptured { .. })
    ));

    let with = observe(
        &[msvc_string_recipe(capacity_guard(), 1)],
        &memory,
        config(),
    )
    .expect("custody");
    let view = with.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.bytes(STR_HEAP + text.len() as u64, 1).expect("NUL"),
        &[0u8]
    );
}

#[test]
fn the_trailer_is_inside_the_element_limit() {
    let text = b"0123456789abcdef";
    let memory = msvc_string_memory(Some(text));
    let mut recipe = msvc_string_recipe(capacity_guard(), 1);
    recipe.ops[1] = match recipe.ops[1].clone() {
        RecipeOp::DecodeExtent {
            count_offset,
            count_width,
            ptr_offset,
            stride,
            max_bytes,
            trailer_elements,
            guard,
            register,
            element,
            ..
        } => RecipeOp::DecodeExtent {
            count_offset,
            count_width,
            ptr_offset,
            stride,
            max_count: text.len() as u32,
            max_bytes,
            trailer_elements,
            guard,
            register,
            element,
        },
        other => other,
    };
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::ExtentCountExceeded {
            recipe: RecipeId(1),
            limit: text.len() as u32,
            decoded: text.len() as u32 + 1,
        }
    );
}

#[test]
fn an_uncaptured_guard_word_refuses_rather_than_skipping() {
    let text = b"a rather longer string than sixteen bytes";
    let memory = msvc_string_memory(Some(text));
    let mut recipe = msvc_string_recipe(capacity_guard(), 1);
    recipe.ops[0] = RecipeOp::CopyHeader {
        size: 0x18,
        next: vec![1],
    };
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::FieldNotCaptured {
            recipe: RecipeId(1),
            span: Span::new(STR_OBJ + 0x18, 8),
        }
    );
}

#[test]
fn a_terminal_extent_freezes_once_and_not_once_per_element() {
    let text = b"a rather longer string than sixteen bytes";
    let memory = msvc_string_memory(Some(text));

    let terminal = observe(
        &[msvc_string_recipe(capacity_guard(), 1)],
        &memory,
        config(),
    )
    .expect("custody");
    assert_eq!(terminal.report_for(RecipeId(1)).unwrap().frozen_branches, 1);

    let mut spelled = msvc_string_recipe(capacity_guard(), 1);
    spelled.ops.push(RecipeOp::Freeze);
    spelled.ops[1] = match spelled.ops[1].clone() {
        RecipeOp::DecodeExtent {
            count_offset,
            count_width,
            ptr_offset,
            stride,
            max_count,
            max_bytes,
            trailer_elements,
            guard,
            register,
            ..
        } => RecipeOp::DecodeExtent {
            count_offset,
            count_width,
            ptr_offset,
            stride,
            max_count,
            max_bytes,
            trailer_elements,
            guard,
            register,
            element: vec![2],
        },
        other => other,
    };
    let per_element = observe(&[spelled], &memory, config()).expect("custody");
    assert_eq!(
        per_element.report_for(RecipeId(1)).unwrap().frozen_branches,
        text.len() as u32,
        "the per-element spelling runs the element ops once per byte, which is \
         the cost the empty list exists to avoid",
    );
}

#[test]
fn a_tree_sentinels_unconstructed_value_is_not_dereferenced() {
    const HEAD: u64 = 0x40_0000;
    const NODE: u64 = 0x40_1000;
    const ROLE: u64 = 0x40_2000;
    const ISNIL: u32 = 0x19;
    const VALUE: u32 = 0x28;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HEAD, 0x30);
    memory.map_zeroed(NODE, 0x30);
    memory.map_zeroed(ROLE, 0x10);
    memory.write(HEAD + u64::from(ISNIL), &[1u8]);
    memory.write_u64(HEAD, NODE);
    memory.write_u64(HEAD + 0x10, NODE);
    memory.write_u64(HEAD + u64::from(VALUE), 0x7F_0000_0000);
    memory.write_u64(NODE, HEAD);
    memory.write_u64(NODE + 0x10, HEAD);
    memory.write_u64(NODE + u64::from(VALUE), ROLE);

    let node_ops = |guard: Option<ExtentGuard>| {
        vec![
            RecipeOp::CopyHeader {
                size: 0x30,
                next: vec![1, 2, 3],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 0,
            },
            RecipeOp::DecodePointer {
                offset: 0x10,
                guard: None,
                next: 0,
            },
            RecipeOp::DecodePointer {
                offset: VALUE,
                guard,
                next: 4,
            },
            RecipeOp::CopyHeader {
                size: 0x10,
                next: vec![5],
            },
            RecipeOp::Freeze,
        ]
    };
    let recipe = |guard| {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            node_ops(guard),
            vec![Root { op: 0, addr: HEAD }],
        )
    };

    let unguarded = observe(&[recipe(None)], &memory, config()).expect("custody");
    assert!(
        matches!(refusal(&unguarded, 1), Refusal::MemoryUnreadable { .. }),
        "an unguarded value chase must reach the sentinel's slot, or this test \
         is not watching the failure the guard prevents: got {:?}",
        outcome(&unguarded, 1)
    );

    let guarded = observe(
        &[recipe(Some(ExtentGuard::exactly(ISNIL, 1, 0)))],
        &memory,
        config(),
    )
    .expect("custody");
    assert_eq!(outcome(&guarded, 1), RecipeOutcome::Captured);
    let view = guarded
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("view");
    assert!(view.bytes(ROLE, 0x10).is_ok());
    assert!(view.bytes(HEAD, 0x30).is_ok());
    assert!(matches!(
        view.bytes(0x7F_0000_0000, 1),
        Err(ViewRefusal::NotCaptured { .. })
    ));
}

#[test]
fn decoding_a_field_outside_the_copied_object_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 64);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: 16,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::FieldNotCaptured {
            recipe: RecipeId(1),
            span: Span::new(BASE + 16, 8),
        }
    );
}

#[test]
fn an_unreadable_span_refuses_the_recipe_and_only_that_recipe() {
    let mut memory = chain_memory(2, 0);
    memory.set_readable(node_addr(1), false);
    let mut shallow = chain_recipe(2, 2);
    shallow.limits.max_depth = 0;
    let build = observe(&[chain_recipe(1, 1), shallow], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::MemoryUnreadable {
            recipe: RecipeId(1),
            span: Span::new(node_addr(1), 16),
        }
    );
    assert_eq!(
        refusal(&build, 2),
        Refusal::DepthExceeded {
            recipe: RecipeId(2),
            limit: 0,
            wanted_depth: 1,
        }
    );
}

#[test]
fn an_unreadable_span_leaves_other_recipes_captured() {
    let mut memory = chain_memory(2, 0);
    memory.map_zeroed(node_addr(5), 16);
    memory.set_readable(node_addr(1), false);
    let mut elsewhere = chain_recipe(2, 2);
    elsewhere.roots = vec![Root {
        op: 0,
        addr: node_addr(5),
    }];
    let build = observe(&[chain_recipe(1, 1), elsewhere], &memory, config()).expect("custody");
    assert!(matches!(
        refusal(&build, 1),
        Refusal::MemoryUnreadable { .. }
    ));
    assert_eq!(outcome(&build, 2), RecipeOutcome::Captured);
    assert!(build.snapshot.gap_reason(ConsumerId(2)).is_none());
}

#[test]
fn a_shared_object_is_copied_once_and_carries_both_dependents() {
    let memory = chain_memory(3, 0);
    let build =
        observe(&[chain_recipe(1, 1), chain_recipe(2, 2)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(outcome(&build, 2), RecipeOutcome::Captured);
    assert_eq!(
        memory.copies(),
        3,
        "three objects, two recipes, three copies"
    );
    assert_eq!(build.snapshot.interval_count(), 3);
    let links = build.snapshot.links();
    for record in build.snapshot.intervals() {
        assert!(links.depends(record, ConsumerId(1)));
        assert!(links.depends(record, ConsumerId(2)));
    }
    assert_eq!(build.report_for(RecipeId(1)).unwrap().planned_bytes, 48);
    assert_eq!(build.report_for(RecipeId(2)).unwrap().planned_bytes, 48);
}

#[test]
fn a_partial_overlap_copies_only_the_hole() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x200);
    let first = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 0x40,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let second = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        vec![
            RecipeOp::CopyHeader {
                size: 0x40,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: BASE + 0x20,
        }],
    );
    let build = observe(&[first, second], &memory, config()).expect("custody");
    assert_eq!(build.snapshot.interval_count(), 2);
    let spans: Vec<Span> = build
        .snapshot
        .intervals()
        .iter()
        .map(|record| record.span())
        .collect();
    assert_eq!(
        spans,
        vec![Span::new(BASE, 0x40), Span::new(BASE + 0x40, 0x20)]
    );
    assert_eq!(memory.bytes_moved(), 0x60, "0x20 bytes are not re-copied");
    let links = build.snapshot.links();
    assert!(links.depends(&build.snapshot.intervals()[0], ConsumerId(1)));
    assert!(links.depends(&build.snapshot.intervals()[0], ConsumerId(2)));
    assert!(!links.depends(&build.snapshot.intervals()[1], ConsumerId(1)));
    assert!(links.depends(&build.snapshot.intervals()[1], ConsumerId(2)));
}

#[test]
fn ownership_planning_splits_around_owned_intervals() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    owners.insert(Span::new(0x1030, 0x10), 0x10).unwrap();
    let plan = owners.plan(Span::new(0x0FF0, 0x60));
    assert_eq!(
        plan.holes,
        vec![
            Span::new(0x0FF0, 0x10),
            Span::new(0x1010, 0x20),
            Span::new(0x1040, 0x10),
        ]
    );
    assert_eq!(plan.existing.len(), 2);

    let fully_owned = owners.plan(Span::new(0x1000, 0x10));
    assert!(fully_owned.holes.is_empty());
    assert_eq!(fully_owned.existing.len(), 1);
}

#[test]
fn an_interval_starting_at_the_query_address_is_reported_once() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    owners.insert(Span::new(0x1010, 0x10), 0x10).unwrap();

    assert_eq!(owners.overlapping(Span::new(0x1000, 0x10)).len(), 1);
    assert_eq!(owners.overlapping(Span::new(0x1000, 0x20)).len(), 2);
    assert_eq!(owners.overlapping(Span::new(0x1008, 0x10)).len(), 2);
    assert_eq!(owners.overlapping(Span::new(0x1010, 0x8)).len(), 1);
    assert!(owners.overlapping(Span::new(0x2000, 0x10)).is_empty());
}

#[test]
fn custody_exhaustion_fails_the_observation_closed() {
    let memory = chain_memory(3, 0);
    let fits = observe_in(&[chain_recipe(1, 1)], &memory, config(), Arena::new())
        .expect("the default budget fits a three-node chain");
    let needed = fits.custody_held;
    assert!(needed > 0, "an observation that copied bytes held some");

    let short = observe_in(
        &[chain_recipe(1, 1)],
        &memory,
        config(),
        Arena::with_budget(needed - 1),
    );
    match short.err() {
        Some(CustodyRefusal::BudgetExhausted {
            budget,
            held,
            requested,
        }) => {
            assert_eq!(budget, needed - 1);
            assert!(
                u128::from(held) + u128::from(requested) > u128::from(budget),
                "a budget refusal must name a request that does not fit: \
                 held {held} + requested {requested} against {budget}",
            );
        }
        other => panic!("expected a budget refusal, got {other:?}"),
    }
}

#[test]
fn custody_counts_the_index_and_link_tables_and_not_only_the_copies() {
    let memory = chain_memory(3, 0);
    let build = observe_in(&[chain_recipe(1, 1)], &memory, config(), Arena::new())
        .expect("the default budget fits it");
    let copied = build.snapshot.custody_bytes();
    assert_eq!(copied, 48, "three 16 B nodes");
    assert!(
        build.custody_table_bytes > 0,
        "the index and link tables must be charged; they were not",
    );
    assert_eq!(
        build.custody_held,
        build.custody_arena_bytes + build.custody_table_bytes,
        "held is exactly the arena's capacity plus the tables",
    );
    assert!(
        build.custody_arena_bytes >= copied,
        "the arena is charged for the capacity it took ({}), which is at least the {copied} \
         bytes copied into it",
        build.custody_arena_bytes,
    );
    assert!(
        build.custody_table_bytes > copied,
        "at this shape the tables are the larger half: {} table bytes against {copied} copied",
        build.custody_table_bytes,
    );
}

#[test]
fn the_arena_is_charged_for_its_capacity_and_not_for_its_length() {
    let mut arena = Arena::with_budget(1024);
    for _ in 0..3 {
        arena.alloc_zeroed(16).expect("well inside the budget");
    }
    assert_eq!(arena.copied_bytes(), 48);
    assert!(
        arena.held() > arena.copied_bytes(),
        "the arena holds {} bytes of capacity for {} bytes of copies, and custody counts \
         the capacity",
        arena.held(),
        arena.copied_bytes(),
    );

    let before = arena.held();
    arena
        .alloc_zeroed(16)
        .expect("this fits in the capacity already paid for");
    assert_eq!(arena.copied_bytes(), 64);
    assert_eq!(
        arena.held(),
        before,
        "no growth happened, so nothing may be charged for one",
    );

    let mut tight = Arena::with_budget(72);
    for _ in 0..4 {
        tight.alloc_zeroed(16).expect("inside the budget");
    }
    tight
        .alloc_zeroed(8)
        .expect("the clamp is what leaves room for this");
    assert_eq!(tight.copied_bytes(), 72);
    assert_eq!(
        tight.held(),
        72,
        "clamped to the room, never doubled past it",
    );
    assert!(
        matches!(
            tight.alloc_zeroed(1),
            Err(CustodyRefusal::BudgetExhausted { .. })
        ),
        "and one byte past the ceiling is still a refusal",
    );
}

#[derive(Debug)]
struct NeverAllocates;

impl ArenaAllocator for NeverAllocates {
    fn reserve(&self, _buf: &mut Vec<u8>, _additional: usize) -> Result<(), ()> {
        Err(())
    }
}

#[test]
fn an_allocation_failure_fails_the_observation_closed() {
    let memory = chain_memory(1, 0);
    let ledger = super::arena::CustodyLedger::new(DEFAULT_CUSTODY_BUDGET_BYTES);
    let arena = Arena::on_ledger(std::sync::Arc::clone(&ledger), Box::new(NeverAllocates));
    let result = observe_in(&[chain_recipe(1, 1)], &memory, config(), arena);
    assert_eq!(
        result.err(),
        Some(CustodyRefusal::AllocationFailed {
            held: ledger.held(),
            requested: 16,
        })
    );
    // And the ownership map charged the SAME ledger: a map with a ledger of
    assert!(
        ledger.table_bytes() > 0,
        "the ownership map's tables must charge the arena's ledger",
    );
    assert_eq!(
        ledger.arena_bytes(),
        0,
        "the allocator refused, so no copied byte is in custody",
    );
}

#[test]
fn the_default_custody_budget_is_the_owner_amended_figure() {
    assert_eq!(DEFAULT_CUSTODY_BUDGET_BYTES, 512 * 1024 * 1024);
    assert_eq!(Arena::new().budget(), DEFAULT_CUSTODY_BUDGET_BYTES);
    assert_eq!(
        ObservationConfig::new(HOUR).custody_budget(),
        DEFAULT_CUSTODY_BUDGET_BYTES
    );
}

#[test]
fn custody_accounts_the_closing_image_too() {
    let memory = chain_memory(2, 0);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 0,
            },
            RecipeOp::WitnessClose { next: vec![3] },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: node_addr(0),
        }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(build.witnessed_intervals, 2);
    assert_eq!(
        build.snapshot.custody_bytes(),
        64,
        "two 16-byte objects, opening and closing"
    );
}

fn witness_recipe(id: u32, consumer: u16, root: u64) -> Recipe {
    Recipe::new(
        RecipeId(id),
        ConsumerId(consumer),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 3,
            },
            RecipeOp::WitnessClose { next: vec![4] },
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: root }],
    )
}

#[test]
fn a_still_world_certifies_every_witness() {
    let memory = chain_memory(2, 0);
    let build = observe(&[witness_recipe(1, 1, node_addr(0))], &memory, config()).expect("custody");
    assert_eq!(build.witnessed_intervals, 2);
    assert_eq!(
        build.phases_run, 3,
        "two forward waves plus the closing wave"
    );
    let closed = build.close_witnesses();
    assert!(closed.witness.changed.is_empty());
    assert!(closed.witness.gapped.is_empty());
    assert!(closed.snapshot.view_for(ConsumerId(1), HOUR).is_ok());
}

#[test]
fn a_changed_parent_invalidates_its_dependent_subtree() {
    let mutating = MutatingMemory::new(
        chain_memory(2, 0),
        3,
        vec![(node_addr(0) + 8, 0xFFu64.to_le_bytes().to_vec())],
    );
    let build =
        observe(&[witness_recipe(1, 1, node_addr(0))], &mutating, config()).expect("custody");
    assert!(mutating.mutation_applied());
    let closed = build.close_witnesses();
    assert_eq!(closed.witness.changed.len(), 1);
    let changed = closed.witness.changed[0];
    assert_eq!(
        closed.witness.invalidated.len(),
        1,
        "the child discovered through the changed parent is invalidated"
    );
    assert_eq!(
        closed.snapshot.gap_reason(ConsumerId(1)),
        Some(GapReason::WitnessChanged { interval: changed })
    );
    assert_eq!(
        closed.snapshot.view_for(ConsumerId(1), HOUR).err(),
        Some(ViewRefusal::ConsumerGapped(GapReason::WitnessChanged {
            interval: changed
        }))
    );
}

#[test]
fn a_changed_leaf_invalidates_nothing_below_it() {
    let mutating = MutatingMemory::new(
        chain_memory(2, 0),
        4,
        vec![(node_addr(1) + 8, 0x77u64.to_le_bytes().to_vec())],
    );
    let build =
        observe(&[witness_recipe(1, 1, node_addr(0))], &mutating, config()).expect("custody");
    let closed = build.close_witnesses();
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(closed.witness.invalidated.is_empty());
    assert!(closed.snapshot.gap_reason(ConsumerId(1)).is_some());
}

#[test]
fn a_failed_witness_gaps_only_the_affected_consumers() {
    let build_fixture = || {
        let mut memory = SyntheticMemory::new();
        for index in 0..4 {
            memory.map_zeroed(node_addr(index), 16);
        }
        memory.write_u64(node_addr(0), node_addr(1));
        memory.write_u64(node_addr(2), node_addr(3));
        memory
    };
    let recipes = vec![
        witness_recipe(1, 1, node_addr(0)),
        witness_recipe(2, 2, node_addr(2)),
    ];
    let baseline = observe(&recipes, &build_fixture(), config()).expect("custody");
    assert_eq!(baseline.witnessed_intervals, 4);

    let mutating = MutatingMemory::new(
        build_fixture(),
        5,
        vec![(node_addr(0) + 8, 0x1234u64.to_le_bytes().to_vec())],
    );
    let build = observe(&recipes, &mutating, config()).expect("custody");
    let closed = build.close_witnesses();
    assert!(closed.snapshot.gap_reason(ConsumerId(1)).is_some());
    assert_eq!(
        closed.snapshot.gap_reason(ConsumerId(2)),
        None,
        "the second family's bytes did not move, so it keeps its hour"
    );
    assert!(closed.snapshot.view_for(ConsumerId(2), HOUR).is_ok());
}

#[derive(Debug)]
struct StagedMemory {
    inner: SyntheticMemory,
    deny_after: u64,
    seen: std::cell::Cell<u64>,
    deny: u64,
}

impl SnapshotMemory for StagedMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        if self.seen.get() >= self.deny_after && addr == self.deny {
            return Err(MemoryRefusal {
                span: Span::new(addr, out.len() as u64),
            });
        }
        let result = self.inner.copy_into(addr, out);
        if result.is_ok() {
            self.seen.set(self.seen.get() + 1);
        }
        result
    }
}

#[test]
fn a_witness_that_cannot_be_re_copied_counts_as_changed() {
    let staged = StagedMemory {
        inner: chain_memory(2, 0),
        deny_after: 2,
        seen: std::cell::Cell::new(0),
        deny: node_addr(1),
    };
    let build = observe(&[witness_recipe(1, 1, node_addr(0))], &staged, config()).expect("custody");
    assert_eq!(build.witnessed_intervals, 2);
    let closed = build.close_witnesses();
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(closed.snapshot.gap_reason(ConsumerId(1)).is_some());
}

#[test]
fn a_recipe_without_a_witness_close_enrols_nothing() {
    let memory = chain_memory(2, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(build.witnessed_intervals, 0);
    assert!(!build.report_for(RecipeId(1)).unwrap().witness_required);
    let closed = build.close_witnesses();
    assert!(closed.witness.changed.is_empty());
}

#[test]
fn a_shared_interval_is_witnessed_when_any_dependent_asks() {
    let memory = chain_memory(2, 0);
    let recipes = vec![chain_recipe(1, 1), witness_recipe(2, 2, node_addr(0))];
    let build = observe(&recipes, &memory, config()).expect("custody");
    assert_eq!(
        build.witnessed_intervals, 2,
        "sharing widens the closing wave, it never narrows it"
    );
}

fn two_node_snapshot() -> super::witness::ClosedSnapshot {
    let memory = chain_memory(2, 0);
    observe(&[chain_recipe(1, 1)], &memory, config())
        .expect("custody")
        .close_witnesses()
}

#[test]
fn a_view_translates_original_addresses_onto_the_arena() {
    let closed = two_node_snapshot();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(view.u64_le(node_addr(0)).unwrap(), node_addr(1));
    assert_eq!(view.u64_le(node_addr(0) + 8).unwrap(), 0xA0);
    assert_eq!(view.u64_le(node_addr(1) + 8).unwrap(), 0xA1);
    assert_eq!(view.u8_at(node_addr(1)).unwrap(), 0);
    assert_eq!(view.translate(node_addr(0)).unwrap(), 0);
    assert_eq!(view.translate(node_addr(0) + 4).unwrap(), 4);
    assert_eq!(view.translate(node_addr(1)).unwrap(), 16);
    assert_eq!(view.hour(), HOUR);
    assert_eq!(view.consumer(), ConsumerId(1));
}

#[test]
fn a_view_refuses_a_foreign_hour() {
    let closed = two_node_snapshot();
    let asked = HourStamp(HOUR.0 + 1);
    assert_eq!(
        closed.snapshot.view_for(ConsumerId(1), asked).err(),
        Some(ViewRefusal::ForeignHour {
            snapshot: HOUR,
            asked,
        })
    );
}

#[test]
fn a_view_refuses_bytes_the_observation_never_copied() {
    let closed = two_node_snapshot();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.bytes(node_addr(0) + 0x800, 8).err(),
        Some(ViewRefusal::NotCaptured {
            span: Span::new(node_addr(0) + 0x800, 8),
        })
    );
    assert_eq!(
        view.u64_le(node_addr(1) + 16).err(),
        Some(ViewRefusal::NotCaptured {
            span: Span::new(node_addr(1) + 16, 8),
        })
    );
}

#[test]
fn a_view_refuses_another_consumers_bytes() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 16);
    memory.map_zeroed(BASE + 0x1000, 16);
    let mine = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let theirs = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: BASE + 0x1000,
        }],
    );
    let closed = observe(&[mine, theirs], &memory, config())
        .expect("custody")
        .close_witnesses();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(view.bytes(BASE, 8).is_ok());
    assert_eq!(
        view.bytes(BASE + 0x1000, 8).err(),
        Some(ViewRefusal::NotOwned {
            span: Span::new(BASE + 0x1000, 8),
        })
    );
    assert_eq!(
        view.translate(BASE + 0x1000).err(),
        Some(ViewRefusal::NotOwned {
            span: Span::new(BASE + 0x1000, 1),
        })
    );
}

#[test]
fn a_read_spanning_two_transactions_stitches_and_a_borrow_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 32);
    memory.write_u64(BASE, 0x1111);
    memory.write_u64(BASE + 8, 0x2222);
    // Two recipes, ONE consumer: adjacent spans acquired as two transactions.
    let head = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let tail = Recipe::new(
        RecipeId(2),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: BASE + 8,
        }],
    );
    let closed = observe(&[head, tail], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert_eq!(closed.snapshot.interval_count(), 2);
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.bytes(BASE, 16).err(),
        Some(ViewRefusal::NotContiguous {
            span: Span::new(BASE, 16),
        })
    );
    let mut out = [0u8; 16];
    view.read_into(BASE, &mut out).expect("stitched");
    assert_eq!(u64::from_le_bytes(out[..8].try_into().unwrap()), 0x1111);
    assert_eq!(u64::from_le_bytes(out[8..].try_into().unwrap()), 0x2222);
}

#[test]
fn page_index_agrees_with_the_ordered_search() {
    let mut memory = SyntheticMemory::new();
    let mut recipes = Vec::new();
    let sizes: [u32; 8] = [8, 0x40, 0x1800, 0x10, 0x2400, 0x8, 0x900, 0x60];
    let mut addr = BASE;
    let mut spans: Vec<(u64, u32)> = Vec::new();
    for index in 0..48usize {
        let size = sizes[index % sizes.len()];
        memory.map_zeroed(addr, size as usize);
        spans.push((addr, size));
        recipes.push(Recipe::new(
            RecipeId(index as u32),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr }],
        ));
        addr += u64::from(size) + 0x30 + (index as u64 % 7) * 0x40;
    }
    let closed = observe(&recipes, &memory, config())
        .expect("custody")
        .close_witnesses();
    assert_eq!(closed.snapshot.interval_count(), spans.len());
    assert!(closed.snapshot.index().buckets() > 1);

    let intervals = closed.snapshot.intervals();
    let straddles = intervals.windows(2).any(|pair| {
        let earlier_end = pair[0].span().end_excl();
        let later_page = pair[1].span().start >> super::view::PAGE_SHIFT;
        let earlier_start_page = pair[0].span().start >> super::view::PAGE_SHIFT;
        earlier_start_page < later_page && earlier_end > u128::from(pair[1].span().start & !0xFFF)
    });
    assert!(
        straddles,
        "fixture must contain an interval spanning into a page where a later interval starts"
    );

    let last = intervals.last().unwrap().span();
    let top = u64::try_from(last.end_excl()).unwrap() + 0x2000;
    let mut addr = BASE - 0x2000;
    while addr < top {
        let (indexed, searched) = closed.snapshot.locate_both(addr);
        assert_eq!(indexed, searched, "disagreement at {addr:#x}");
        addr += 1;
    }
}

fn malformed(recipe: Recipe) -> MalformedReason {
    match validate(&recipe) {
        Err(Refusal::Malformed { reason, .. }) => reason,
        other => panic!("expected a malformed recipe, got {other:?}"),
    }
}

#[test]
fn validation_rejects_structural_defects() {
    let base = || {
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
            vec![Root { op: 0, addr: BASE }],
        )
    };
    assert!(validate(&base()).is_ok());

    let mut empty = base();
    empty.ops.clear();
    assert_eq!(malformed(empty), MalformedReason::NoOps);

    let mut rootless = base();
    rootless.roots.clear();
    assert_eq!(malformed(rootless), MalformedReason::NoRoots);

    let mut out_of_range = base();
    out_of_range.roots = vec![Root { op: 9, addr: BASE }];
    assert_eq!(
        malformed(out_of_range),
        MalformedReason::OpIndexOutOfRange { op: 9 }
    );

    let mut decode_root = base();
    decode_root.roots = vec![Root { op: 1, addr: BASE }];
    assert_eq!(
        malformed(decode_root),
        MalformedReason::RootIsNotACopyOp { op: 1 }
    );

    let mut copy_in_decode_slot = base();
    copy_in_decode_slot.ops[0] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![0],
    };
    assert_eq!(
        malformed(copy_in_decode_slot),
        MalformedReason::ExpectedDecodeOp { op: 0 }
    );

    let mut decode_in_copy_slot = base();
    decode_in_copy_slot.ops = vec![
        RecipeOp::CopyHeader {
            size: 8,
            next: vec![1],
        },
        RecipeOp::DecodePointer {
            offset: 0,
            guard: None,
            next: 2,
        },
        RecipeOp::Freeze,
    ];
    assert_eq!(
        malformed(decode_in_copy_slot),
        MalformedReason::ExpectedCopyOp { op: 2 }
    );

    let mut dangling = base();
    dangling.ops[0] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![],
    };
    assert_eq!(
        malformed(dangling),
        MalformedReason::DanglingBranch { op: 0 }
    );

    let mut zero_size = base();
    zero_size.ops[0] = RecipeOp::CopyHeader {
        size: 0,
        next: vec![1],
    };
    assert_eq!(
        malformed(zero_size),
        MalformedReason::ZeroSizedCopy { op: 0 }
    );

    let mut deep = base();
    deep.limits.max_depth = MAX_RECIPE_DEPTH + 1;
    assert_eq!(
        malformed(deep),
        MalformedReason::DepthLimitAboveCeiling {
            limit: MAX_RECIPE_DEPTH + 1
        }
    );
    let mut past_the_measurement = base();
    past_the_measurement.limits.max_depth = MEASURED_MAX_DISCOVERY_DEPTH + 1;
    assert!(validate(&past_the_measurement).is_ok());

    let mut zero_limit = base();
    zero_limit.limits.max_ranges = 0;
    assert_eq!(
        malformed(zero_limit),
        MalformedReason::ZeroLimit {
            limit: "max_ranges"
        }
    );

    let mut bad_width = base();
    bad_width.ops = vec![
        RecipeOp::CopyHeader {
            size: 16,
            next: vec![1],
        },
        RecipeOp::DecodeExtent {
            count_offset: 0,
            count_width: 3,
            ptr_offset: 8,
            stride: 8,
            max_count: 4,
            max_bytes: 64,
            trailer_elements: 0,
            guard: None,
            register: None,
            element: vec![2],
        },
        RecipeOp::Freeze,
    ];
    assert_eq!(
        malformed(bad_width),
        MalformedReason::BadWidth { op: 1, width: 3 }
    );

    let mut zero_stride = base();
    zero_stride.ops = vec![
        RecipeOp::CopyHeader {
            size: 16,
            next: vec![1],
        },
        RecipeOp::DecodeExtent {
            count_offset: 0,
            count_width: 4,
            ptr_offset: 8,
            stride: 0,
            max_count: 4,
            max_bytes: 64,
            trailer_elements: 0,
            guard: None,
            register: None,
            element: vec![2],
        },
        RecipeOp::Freeze,
    ];
    assert_eq!(
        malformed(zero_stride),
        MalformedReason::ZeroStride { op: 1 }
    );
}

#[test]
fn a_malformed_recipe_refuses_before_any_memory_is_touched() {
    let memory = chain_memory(2, 0);
    let mut recipe = chain_recipe(1, 1);
    recipe.ops[0] = RecipeOp::CopyHeader {
        size: 0,
        next: vec![1, 2],
    };
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert!(matches!(
        refusal(&build, 1),
        Refusal::Malformed {
            reason: MalformedReason::ZeroSizedCopy { .. },
            ..
        }
    ));
    assert_eq!(memory.copies(), 0);
}

#[test]
fn span_arithmetic_does_not_wrap_at_the_top_of_the_address_space() {
    let top = Span::new(u64::MAX - 7, 8);
    assert_eq!(top.end_excl(), u128::from(u64::MAX) + 1);
    assert!(top.contains_addr(u64::MAX));
    assert_eq!(top.gap_to(&Span::new(u64::MAX, 1)), Some(0));
    let low = Span::new(0x1000, 8);
    assert_eq!(low.gap_to(&Span::new(0x1008, 8)), Some(0));
    assert_eq!(low.gap_to(&Span::new(0x1009, 8)), Some(1));
    assert_eq!(low.gap_to(&Span::new(0x0FF0, 8)), None);
    assert_eq!(low.hull(&Span::new(0x1100, 8)), Span::new(0x1000, 0x108));
}

#[test]
fn a_partial_overlap_does_not_grant_the_rest_of_the_interval() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x200);
    let header = |id: u32, consumer: u16, addr: u64, size: u32| {
        Recipe::new(
            RecipeId(id),
            ConsumerId(consumer),
            vec![
                RecipeOp::CopyHeader {
                    size,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr }],
        )
    };
    let closed = observe(
        &[header(1, 1, BASE, 0x40), header(2, 2, BASE + 0x20, 0x40)],
        &memory,
        config(),
    )
    .expect("custody")
    .close_witnesses();

    let b = closed.snapshot.view_for(ConsumerId(2), HOUR).expect("view");
    assert_eq!(
        b.bytes(BASE, 0x20).err(),
        Some(ViewRefusal::NotOwned {
            span: Span::new(BASE, 0x20),
        }),
        "B never requested the first half of A's object"
    );
    assert_eq!(
        b.translate(BASE).err(),
        Some(ViewRefusal::NotOwned {
            span: Span::new(BASE, 1),
        })
    );
    assert!(b.bytes(BASE + 0x20, 0x20).is_ok());
    let mut out = [0u8; 0x40];
    b.read_into(BASE + 0x20, &mut out).expect("B's own bytes");
    let a = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(a.bytes(BASE, 0x40).is_ok());
}

#[test]
fn an_unrepresentable_window_address_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(u64::MAX - 7, 8);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 1), RelWindow::new(16, 1)],
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: u64::MAX,
        }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::AddressNotRepresentable {
            recipe: RecipeId(1),
            base: u64::MAX,
            offset: 16,
        }
    );
}

#[test]
fn an_unrepresentable_field_address_refuses() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(u64::MAX - 15, 16);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: 32,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![Root {
            op: 0,
            addr: u64::MAX - 15,
        }],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(
        refusal(&build, 1),
        Refusal::AddressNotRepresentable {
            recipe: RecipeId(1),
            base: u64::MAX - 15,
            offset: 32,
        }
    );
}

#[test]
fn two_extents_at_one_base_with_different_counts_are_both_expanded() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 16);
    memory.map_zeroed(BASE + 0x100, 16);
    let array = BASE + 0x1000;
    memory.map_zeroed(array, 16);
    memory.write_u32(BASE, 1);
    memory.write_u64(BASE + 8, array);
    memory.write_u32(BASE + 0x100, 2);
    memory.write_u64(BASE + 0x100 + 8, array);
    for index in 0..2u64 {
        let leaf = BASE + 0x2000 + index * 0x100;
        memory.map_zeroed(leaf, 8);
        memory.write_u64(leaf, 0xC0DE + index);
        memory.write_u64(array + index * 8, leaf);
    }

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: 8,
                max_count: 8,
                max_bytes: 4096,
                trailer_elements: 0,
                guard: None,
                register: None,
                element: vec![2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 3,
            },
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![
            Root { op: 0, addr: BASE },
            Root {
                op: 0,
                addr: BASE + 0x100,
            },
        ],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    let closed = build.close_witnesses();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(view.u64_le(BASE + 0x2000).unwrap(), 0xC0DE);
    assert_eq!(
        view.u64_le(BASE + 0x2000 + 0x100).unwrap(),
        0xC0DF,
        "the second element was pruned as a duplicate"
    );
}

#[test]
fn a_changed_parent_does_not_gap_a_consumer_that_took_another_route() {
    let fixture = || {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(node_addr(0), 16);
        memory.map_zeroed(node_addr(1), 16);
        memory.write_u64(node_addr(0), node_addr(1));
        memory
    };
    let recipes = vec![
        witness_recipe(1, 1, node_addr(0)),
        witness_recipe(2, 2, node_addr(1)),
    ];
    let baseline = observe(&recipes, &fixture(), config()).expect("custody");
    assert_eq!(
        baseline.witnessed_intervals, 2,
        "one shared interval plus A's parent"
    );

    let mutating = MutatingMemory::new(
        fixture(),
        3,
        vec![(node_addr(0) + 8, 0x9999u64.to_le_bytes().to_vec())],
    );
    let closed = observe(&recipes, &mutating, config())
        .expect("custody")
        .close_witnesses();
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(
        closed.snapshot.gap_reason(ConsumerId(1)).is_some(),
        "A read the parent that moved"
    );
    assert_eq!(
        closed.snapshot.gap_reason(ConsumerId(2)),
        None,
        "B never went through that parent, and its own bytes were certified"
    );
    assert!(closed.snapshot.view_for(ConsumerId(2), HOUR).is_ok());
}

#[test]
fn a_stitched_read_does_not_wrap_the_address_space() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(u64::MAX, 1);
    memory.map_zeroed(0, 1);
    let header = |id: u32, addr: u64| {
        Recipe::new(
            RecipeId(id),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 1,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr }],
        )
    };
    let closed = observe(&[header(1, u64::MAX), header(2, 0)], &memory, config())
        .expect("custody")
        .close_witnesses();
    assert_eq!(closed.snapshot.interval_count(), 2);
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    let mut out = [0u8; 2];
    assert_eq!(
        view.read_into(u64::MAX, &mut out).err(),
        Some(ViewRefusal::NotCaptured {
            span: Span::new(u64::MAX, 2),
        }),
        "the second byte is at 2^64, not at address zero"
    );
    let mut one = [0u8; 1];
    view.read_into(u64::MAX, &mut one).expect("own byte");
}

#[test]
fn the_custody_ceiling_cannot_be_raised() {
    assert_eq!(
        ObservationConfig::new(HOUR)
            .with_custody_budget(DEFAULT_CUSTODY_BUDGET_BYTES + 1)
            .err(),
        Some(CustodyRefusal::BudgetExhausted {
            budget: DEFAULT_CUSTODY_BUDGET_BYTES,
            held: 0,
            requested: DEFAULT_CUSTODY_BUDGET_BYTES + 1,
        })
    );
    let lowered = ObservationConfig::new(HOUR)
        .with_custody_budget(1024)
        .expect("lowering is allowed");
    assert_eq!(lowered.custody_budget(), 1024);

    let memory = chain_memory(1, 0);
    assert_eq!(
        observe_in(
            &[chain_recipe(1, 1)],
            &memory,
            config(),
            Arena::with_budget(DEFAULT_CUSTODY_BUDGET_BYTES + 1),
        )
        .err(),
        Some(CustodyRefusal::BudgetExhausted {
            budget: DEFAULT_CUSTODY_BUDGET_BYTES,
            held: 0,
            requested: DEFAULT_CUSTODY_BUDGET_BYTES + 1,
        })
    );
}

#[test]
fn the_wave_ceiling_covers_the_deepest_container_a_reader_accepts() {
    assert_eq!(MEASURED_MAX_DISCOVERY_DEPTH, 12);
    assert_eq!(MAX_RECIPE_DEPTH, 30);
    assert_eq!(FORWARD_WAVES, 31);
    assert_eq!(TOTAL_PHASES, 32);
    assert!(MAX_RECIPE_DEPTH >= 6 + 24);
    assert!(MAX_RECIPE_DEPTH > MEASURED_MAX_DISCOVERY_DEPTH);
    assert_eq!(Limits::default().max_depth, MEASURED_MAX_DISCOVERY_DEPTH);
}

// The swept ownership index

fn three_run_map(spans: &[Span]) -> OwnershipMap {
    let mut owners = OwnershipMap::new();
    for (index, span) in spans.iter().enumerate() {
        owners.insert(*span, index * 0x100).unwrap();
        if index == 1 {
            owners.end_wave().unwrap();
        } else if index == 3 {
            owners.end_batch().unwrap();
        }
    }
    owners
}

fn sweep_agrees_with_single_queries(owners: &OwnershipMap, queries: &[Span]) {
    let mut cursor = owners.sweep();
    let mut swept = Vec::new();
    for query in queries {
        owners.sweep_overlapping(&mut cursor, *query, &mut swept);
        assert_eq!(
            swept,
            owners.overlapping(*query),
            "sweep disagreed with the bisecting query at {query:?}"
        );
    }
}

#[test]
fn a_sweep_over_adjacent_intervals_reports_each_one_once() {
    let spans: Vec<Span> = (0..5)
        .map(|index| Span::new(0x1000 + index * 0x10, 0x10))
        .collect();
    let owners = three_run_map(&spans);

    let queries: Vec<Span> = vec![
        Span::new(0x1000, 1),
        Span::new(0x100F, 2),
        Span::new(0x1010, 0x10),
        Span::new(0x1020, 0x21),
        Span::new(0x1040, 0x10),
    ];
    sweep_agrees_with_single_queries(&owners, &queries);

    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    let counts: Vec<usize> = queries
        .iter()
        .map(|query| {
            owners.sweep_overlapping(&mut cursor, *query, &mut out);
            out.len()
        })
        .collect();
    assert_eq!(counts, vec![1, 2, 1, 3, 1]);

    let all = owners.overlapping(Span::new(0x1000, 0x50));
    assert_eq!(all.len(), 5);
    let starts: Vec<u64> = all
        .iter()
        .map(|id| owners.record(*id).span().start)
        .collect();
    assert_eq!(starts, vec![0x1000, 0x1010, 0x1020, 0x1030, 0x1040]);

    let mut cursor = owners.sweep();
    owners.sweep_overlapping(&mut cursor, Span::new(0x1000, 1), &mut out);
    assert_eq!(out.len(), 1);
    for last_byte in [0x101Fu64, 0x102F, 0x103F, 0x104F] {
        owners.sweep_overlapping(&mut cursor, Span::new(last_byte, 1), &mut out);
        assert_eq!(
            out.len(),
            1,
            "the interval ending at {last_byte:#x} still contains its own last byte"
        );
        assert_eq!(owners.record(out[0]).span().start, last_byte - 0xF);
    }
}

#[test]
fn a_sweep_reports_an_interval_starting_at_the_query_address_once() {
    let spans = [
        Span::new(0x1000, 0x10),
        Span::new(0x1010, 0x10),
        Span::new(0x1020, 0x10),
        Span::new(0x1030, 0x10),
        Span::new(0x1040, 0x10),
    ];
    let owners = three_run_map(&spans);
    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    for start in [0x1000u64, 0x1010, 0x1020, 0x1030, 0x1040] {
        owners.sweep_overlapping(&mut cursor, Span::new(start, 0x10), &mut out);
        assert_eq!(out.len(), 1, "one owner at {start:#x}, not two");
        assert_eq!(owners.record(out[0]).span().start, start);
    }
}

#[test]
fn a_sweep_re_reports_an_interval_two_queries_share() {
    let spans = [
        Span::new(0x1000, 0x40),
        Span::new(0x1040, 0x40),
        Span::new(0x1080, 0x40),
        Span::new(0x10C0, 0x40),
        Span::new(0x1100, 0x40),
    ];
    let owners = three_run_map(&spans);
    let queries = [
        Span::new(0x1000, 0x50),
        Span::new(0x1020, 0x50),
        Span::new(0x1020, 0x100),
        Span::new(0x1030, 1),
    ];
    sweep_agrees_with_single_queries(&owners, &queries);

    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    owners.sweep_overlapping(&mut cursor, queries[0], &mut out);
    assert_eq!(out.len(), 2);
    owners.sweep_overlapping(&mut cursor, queries[3], &mut out);
    assert_eq!(out.len(), 1, "the first interval is still reachable");
}

#[test]
fn a_sweep_over_empty_and_single_interval_shapes() {
    let empty = OwnershipMap::new();
    let mut cursor = empty.sweep();
    let mut out = vec![IntervalId(7)];
    empty.sweep_overlapping(&mut cursor, Span::new(0x1000, 0x10), &mut out);
    assert!(out.is_empty(), "no owners, and the buffer is cleared");
    assert_eq!(
        empty.plan(Span::new(0x1000, 0x10)).holes,
        vec![Span::new(0x1000, 0x10)]
    );
    assert!(empty.find(0x1000).is_none());

    let mut single = OwnershipMap::new();
    single.insert(Span::new(0x1000, 0x10), 0);
    let mut cursor = single.sweep();
    single.sweep_overlapping(&mut cursor, Span::new(0x0FF0, 0x10), &mut out);
    assert!(out.is_empty(), "ends exactly where the interval starts");
    single.sweep_overlapping(&mut cursor, Span::new(0x0FF0, 0x11), &mut out);
    assert_eq!(out.len(), 1, "one byte of overlap is an overlap");
    single.sweep_overlapping(&mut cursor, Span::new(0x1010, 0x10), &mut out);
    assert!(out.is_empty(), "starts exactly where the interval ends");

    let mut with_empty = OwnershipMap::new();
    with_empty.insert(Span::new(0x1000, 0), 0);
    with_empty.insert(Span::new(0x1000, 0x10), 0);
    assert_eq!(with_empty.overlapping(Span::new(0x1000, 0x10)).len(), 1);
    let mut cursor = with_empty.sweep();
    with_empty.sweep_overlapping(&mut cursor, Span::new(0x1000, 0), &mut out);
    assert!(out.is_empty());
}

#[test]
fn a_sweep_does_not_pass_an_interval_a_later_query_still_needs() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x1000), 0).unwrap();
    owners.end_wave().unwrap();
    owners.insert(Span::new(0x3000, 0x10), 0x1000).unwrap();

    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    for start in [0x1000u64, 0x1400, 0x1800, 0x1FFF] {
        owners.sweep_overlapping(&mut cursor, Span::new(start, 1), &mut out);
        assert_eq!(out.len(), 1, "the wide interval still covers {start:#x}");
    }
    owners.sweep_overlapping(&mut cursor, Span::new(0x2000, 1), &mut out);
    assert!(out.is_empty(), "one past the wide interval");
    owners.sweep_overlapping(&mut cursor, Span::new(0x3008, 1), &mut out);
    assert_eq!(
        out.len(),
        1,
        "the gallop did not overshoot the far interval"
    );
}

#[test]
fn a_swept_acquisition_plan_splits_the_same_holes() {
    let spans = [
        Span::new(0x1000, 0x10),
        Span::new(0x1030, 0x10),
        Span::new(0x1080, 0x10),
        Span::new(0x2000, 0x10),
        Span::new(0x2040, 0x10),
    ];
    let owners = three_run_map(&spans);
    let queries = [
        Span::new(0x0FF0, 0x60),
        Span::new(0x1070, 0x30),
        Span::new(0x1FF0, 0x70),
    ];
    let mut cursor = owners.sweep();
    let mut swept = super::dedup::AcquisitionPlan::default();
    for query in queries {
        owners.sweep_plan(&mut cursor, query, &mut swept);
        assert_eq!(swept, owners.plan(query), "at {query:?}");
    }

    assert_eq!(
        owners.plan(Span::new(0x0FF0, 0x60)).holes,
        vec![
            Span::new(0x0FF0, 0x10),
            Span::new(0x1010, 0x20),
            Span::new(0x1040, 0x10),
        ]
    );

    // what lets `acquire` fill a hole and have the next planned copy see it.
    let mut live = three_run_map(&spans);
    live.end_batch();
    let mut cursor = live.sweep();
    let mut plan = super::dedup::AcquisitionPlan::default();
    live.sweep_plan(&mut cursor, Span::new(0x1010, 0x20), &mut plan);
    assert_eq!(plan.holes, vec![Span::new(0x1010, 0x20)]);
    live.insert(Span::new(0x1010, 0x20), 0x500);
    live.sweep_plan(&mut cursor, Span::new(0x1010, 0x30), &mut plan);
    assert!(
        plan.holes.is_empty(),
        "the hole this batch just filled is owned now"
    );
    assert_eq!(
        plan.existing.len(),
        2,
        "the interval this batch created and the one after it"
    );
    live.sweep_plan(&mut cursor, Span::new(0x1010, 0x40), &mut plan);
    assert_eq!(plan.holes, vec![Span::new(0x1040, 0x10)]);
}

#[test]
fn folding_the_index_preserves_address_order_and_content() {
    let spans: Vec<Span> = (0..24)
        .map(|index| Span::new(0x1000 + index * 0x20, 0x10))
        .collect();
    let mut owners = OwnershipMap::new();
    for chunk in [(16usize, 24usize), (8, 16), (0, 8)] {
        for index in chunk.0..chunk.1 {
            owners.insert(spans[index], index * 0x10).unwrap();
        }
        owners.end_batch().unwrap();
    }
    owners.end_wave().unwrap();

    let all = owners.overlapping(Span::new(0x1000, 24 * 0x20));
    assert_eq!(all.len(), 24);
    let starts: Vec<u64> = all
        .iter()
        .map(|id| owners.record(*id).span().start)
        .collect();
    let mut expected: Vec<u64> = spans.iter().map(|span| span.start).collect();
    expected.sort_unstable();
    assert_eq!(
        starts, expected,
        "the fold must leave the index in address order"
    );

    for span in &spans {
        assert!(
            owners.find(span.start).is_some(),
            "{span:?} survived the fold"
        );
    }

    let (records, _links) = owners.into_frozen().unwrap();
    assert_eq!(records.len(), 24);
    assert!(records
        .windows(2)
        .all(|pair| pair[0].span().start < pair[1].span().start));
}

#[test]
fn a_repeated_discovery_edge_is_stored_once_up_to_the_scan_bound() {
    let mut owners = OwnershipMap::new();
    let parent = owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    let child = owners.insert(Span::new(0x2000, 0x10), 0x10).unwrap();
    let other = owners.insert(Span::new(0x3000, 0x10), 0x20).unwrap();

    owners.add_edge(parent, child, ConsumerId(1)).unwrap();
    owners.add_edge(parent, child, ConsumerId(1)).unwrap();
    owners.add_edge(parent, child, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(
        owners.links().children(&record).count(),
        1,
        "the same edge three times is one edge"
    );

    owners.add_edge(parent, child, ConsumerId(2)).unwrap();
    owners.add_edge(parent, other, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    let mut edges: Vec<(IntervalId, ConsumerId)> = owners.links().children(&record).collect();
    edges.sort_unstable();
    assert_eq!(
        edges,
        vec![
            (child, ConsumerId(1)),
            (child, ConsumerId(2)),
            (other, ConsumerId(1)),
        ]
    );

    owners.add_edge(parent, parent, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), 3);
}

#[test]
fn an_edge_deeper_than_the_dedup_scan_is_still_not_recorded_twice() {
    let mut owners = OwnershipMap::new();
    let parent = owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    let first = owners.insert(Span::new(0x2000, 0x10), 0x10).unwrap();
    owners.add_edge(parent, first, ConsumerId(1)).unwrap();

    let mut buried = Vec::new();
    for index in 0..400u64 {
        let child = owners
            .insert(Span::new(0x3000 + index * 0x10, 0x10), 0)
            .unwrap();
        owners.add_edge(parent, child, ConsumerId(1)).unwrap();
        buried.push(child);
    }
    let record = *owners.record(parent);
    let before = owners.links().children(&record).count();
    assert_eq!(before, 401);

    owners.add_edge(parent, first, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(
        owners.links().children(&record).count(),
        before,
        "the deepest edge is already recorded, however long the chain got"
    );

    owners.add_edge(parent, buried[7], ConsumerId(1)).unwrap();
    owners.add_edge(parent, buried[399], ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), before);

    let fresh = owners.insert(Span::new(0x9000, 0x10), 0).unwrap();
    owners.add_edge(parent, fresh, ConsumerId(1)).unwrap();
    owners.add_edge(parent, fresh, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), before + 1);

    let mut edges: Vec<IntervalId> = owners
        .links()
        .children(&record)
        .map(|(child, _)| child)
        .collect();
    edges.sort_unstable();
    edges.dedup();
    assert_eq!(edges.len(), before + 1, "no duplicates survived");

    let other = owners.insert(Span::new(0xA000, 0x10), 0).unwrap();
    owners.add_edge(other, first, ConsumerId(1)).unwrap();
    owners.add_edge(other, first, ConsumerId(1)).unwrap();
    let record = *owners.record(other);
    assert_eq!(owners.links().children(&record).count(), 1);
}

#[test]
fn an_out_of_order_insert_keeps_the_index_sorted() {
    let mut owners = OwnershipMap::new();
    for start in [0x3000u64, 0x1000, 0x2000, 0x0500] {
        owners.insert(Span::new(start, 0x10), 0).unwrap();
    }
    let all = owners.overlapping(Span::new(0, 0x4000));
    let starts: Vec<u64> = all
        .iter()
        .map(|id| owners.record(*id).span().start)
        .collect();
    assert_eq!(starts, vec![0x0500, 0x1000, 0x2000, 0x3000]);
    for start in [0x3000u64, 0x1000, 0x2000, 0x0500] {
        assert!(owners.find(start).is_some());
    }
}

#[test]
fn the_swept_index_agrees_with_the_bisecting_one_over_a_populated_map() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut starts: Vec<u64> = (0..400).map(|_| (next() % 0x4000) * 0x10).collect();
    starts.sort_unstable();
    starts.dedup();

    let mut owners = OwnershipMap::new();
    for (index, start) in starts.iter().enumerate() {
        owners.insert(Span::new(*start, 0x10), index).unwrap();
        if index % 37 == 5 {
            owners.end_batch().unwrap();
        }
        if index % 91 == 11 {
            owners.end_wave().unwrap();
        }
    }

    let mut queries: Vec<Span> = (0..600)
        .map(|_| Span::new((next() % 0x4200) * 0x10, (next() % 0x60) + 1))
        .collect();
    queries.sort_unstable_by_key(|span| (span.start, span.len));

    sweep_agrees_with_single_queries(&owners, &queries);

    let mut cursor = owners.sweep();
    let mut swept = super::dedup::AcquisitionPlan::default();
    for query in &queries {
        owners.sweep_plan(&mut cursor, *query, &mut swept);
        assert_eq!(swept, owners.plan(*query), "plan disagreed at {query:?}");
    }

    for start in &starts {
        assert_eq!(
            owners.find(*start).map(|id| owners.record(id).span().start),
            Some(*start)
        );
        assert!(owners.find(start.wrapping_sub(1)).is_none() || starts.contains(&(start - 0x10)));
    }
}

#[test]
fn the_index_holds_an_interval_at_the_top_of_the_address_space() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(u64::MAX - 7, 8), 0).unwrap();
    assert_eq!(owners.find(u64::MAX).map(|id| id.0), Some(0));
    assert_eq!(owners.find(u64::MAX - 8), None);
    assert_eq!(owners.overlapping(Span::new(u64::MAX - 7, 8)).len(), 1);
    assert!(owners.plan(Span::new(u64::MAX - 7, 8)).holes.is_empty());
    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    owners.sweep_overlapping(&mut cursor, Span::new(u64::MAX, 1), &mut out);
    assert_eq!(out.len(), 1);
}

#[test]
fn an_index_entry_clips_a_span_that_runs_off_the_top_of_the_address_space() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(u64::MAX - 3, 8), 0).unwrap();
    assert_eq!(owners.find(u64::MAX).map(|id| id.0), Some(0));
    assert_eq!(owners.find(u64::MAX - 3).map(|id| id.0), Some(0));
    assert_eq!(owners.find(u64::MAX - 4), None);
    assert_eq!(owners.overlapping(Span::new(u64::MAX, 1)).len(), 1);
    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    owners.sweep_overlapping(&mut cursor, Span::new(u64::MAX, 1), &mut out);
    assert_eq!(
        out.len(),
        1,
        "a clipped entry is still reachable at the top"
    );
}

#[test]
fn every_request_attaches_however_the_coalescer_ordered_them() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x400);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyRanges {
                windows: vec![
                    RelWindow::new(0x200, 8),
                    RelWindow::new(0x100, 8),
                    RelWindow::new(0x104, 8),
                    RelWindow::new(0x000, 8),
                ],
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let closed = observe(&[recipe], &memory, config())
        .expect("custody")
        .close_witnesses();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    for offset in [0x000u64, 0x100, 0x104, 0x200] {
        assert!(
            view.bytes(BASE + offset, 8).is_ok(),
            "the window at +{offset:#x} must be readable"
        );
    }
    assert!(
        view.bytes(BASE + 0x300, 8).is_err(),
        "and nothing the recipe did not ask for"
    );
}

#[test]
fn the_record_model_stays_narrow() {
    assert_eq!(
        std::mem::size_of::<dedup::IntervalRecord>(),
        32,
        "an interval record must stay at two per cache line",
    );
    assert_eq!(
        dedup::INDEX_ENTRY_BYTES,
        16,
        "an index entry must stay at four per cache line",
    );
}

#[test]
fn a_maximum_length_interval_carries_both_flags_without_disturbing_its_length() {
    let mut owners = OwnershipMap::new();
    let widest = DEFAULT_CUSTODY_BUDGET_BYTES;
    let id = owners.insert(Span::new(0x1000, widest), 0).unwrap();
    assert_eq!(owners.record(id).span(), Span::new(0x1000, widest));
    assert!(!owners.record(id).witnessed());

    owners.enrol_witness(id).unwrap();
    assert!(owners.record(id).witnessed());
    assert_eq!(
        owners.record(id).span(),
        Span::new(0x1000, widest),
        "the witness flag must not be readable as length",
    );

    let child = owners.insert(Span::new(0x1000 + widest, 0x10), 0).unwrap();
    owners.add_edge(id, child, ConsumerId(1)).unwrap();
    assert_eq!(
        owners.record(id).span(),
        Span::new(0x1000, widest),
        "no flag may be readable as length",
    );
    owners.record_mut(id).set_closing_offset(0);
    assert_eq!(owners.record(id).closing_offset(), Some(0));
    assert_eq!(owners.record(id).span(), Span::new(0x1000, widest));
}

#[test]
#[should_panic(expected = "cannot be told from")]
fn a_closing_offset_at_the_sentinel_is_refused() {
    let mut owners = OwnershipMap::new();
    let id = owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    owners.record_mut(id).set_closing_offset(u32::MAX as usize);
}

#[test]
#[should_panic(expected = "is past the")]
fn an_opening_offset_past_the_custody_ceiling_is_refused() {
    let mut owners = OwnershipMap::new();
    owners
        .insert(Span::new(0x1000, 0x10), u32::MAX as usize)
        .unwrap();
}

#[test]
#[should_panic(expected = "it can only exist if custody admitted it")]
fn an_interval_longer_than_the_record_can_hold_is_refused() {
    let mut owners = OwnershipMap::new();
    owners
        .insert(Span::new(0x1000, u64::from(u32::MAX)), 0)
        .unwrap();
}

#[test]
fn the_visited_sets_two_task_kinds_do_not_prune_each_other() {
    let mut visited = executor::VisitedSet::default();
    let tag = 7u64;
    assert!(visited.insert_op(0x1000, tag));
    assert!(!visited.insert_op(0x1000, tag), "a repeat is not fresh");
    assert!(visited.insert_extent(0x1000, 0x40, tag));
    assert!(!visited.insert_extent(0x1000, 0x40, tag));
    assert!(visited.insert_extent(0x1000, 0x80, tag));
    assert_eq!(visited.len(), 3);
}

fn windowed_report(through_pointer: bool, gap: u64) -> RecipeReport {
    const HOLDER: u64 = BASE;
    const TARGET: u64 = BASE + 0x8000;
    let window_len = 8u32;
    let far = (u64::from(window_len) + gap) as i64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HOLDER, 16);
    memory.map_zeroed(TARGET, (far as u64 + u64::from(window_len)) as usize);
    memory.write_u64(HOLDER, TARGET);

    let windows = vec![
        RelWindow::new(0, window_len),
        RelWindow::new(far, window_len),
    ];
    let recipe = if through_pointer {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 16,
                    next: vec![1],
                },
                RecipeOp::DecodePointer {
                    offset: 0,
                    guard: None,
                    next: 2,
                },
                RecipeOp::CopyRanges {
                    windows,
                    next: vec![3],
                },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: HOLDER,
            }],
        )
    } else {
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyRanges {
                    windows,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root {
                op: 0,
                addr: TARGET,
            }],
        )
    };

    let build = observe(&[recipe], &memory, config()).expect("custody budget");
    *build.report_for(RecipeId(1)).expect("the recipe ran")
}

#[test]
fn the_task_anchor_rebuilds_the_address_and_the_kind_it_erased() {
    const HEADER: u32 = 1;

    let bridged = windowed_report(true, u64::from(GENERIC_GAP_BYTES));
    assert_eq!(
        bridged.outcome,
        RecipeOutcome::Captured,
        "the windowed recipe ran to Freeze",
    );
    assert_eq!(
        bridged.planned_ranges,
        HEADER + 1,
        "two windows under ONE dereferenced pointer share an anchor, so a 32 B hole is \
         inside the generic tier and they coalesce into one transaction",
    );

    let too_far = windowed_report(true, u64::from(GENERIC_GAP_BYTES) + 1);
    assert_eq!(
        too_far.planned_ranges,
        HEADER + 2,
        "33 B is past the generic tier; the pair must separate again",
    );

    let unbridged = windowed_report(false, u64::from(GENERIC_GAP_BYTES));
    assert_eq!(unbridged.outcome, RecipeOutcome::Captured);
    assert_eq!(
        unbridged.planned_ranges, 2,
        "a ROOT anchor never satisfies the generic tier, so the identical pair stays two \
         transactions",
    );
}

#[test]
fn two_window_tasks_under_different_pointers_do_not_share_an_anchor() {
    const HOLDER_A: u64 = BASE;
    const HOLDER_B: u64 = BASE + 0x1000;
    const TARGET_A: u64 = BASE + 0x8000;
    const TARGET_B: u64 = TARGET_A + 8 + GENERIC_GAP_BYTES as u64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HOLDER_A, 16);
    memory.map_zeroed(HOLDER_B, 16);
    memory.map_zeroed(TARGET_A, (TARGET_B - TARGET_A) as usize + 8);
    memory.write_u64(HOLDER_A, TARGET_A);
    memory.write_u64(HOLDER_B, TARGET_B);

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 2,
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(0, 8)],
                next: vec![3],
            },
            RecipeOp::Freeze,
        ],
        vec![
            Root {
                op: 0,
                addr: HOLDER_A,
            },
            Root {
                op: 0,
                addr: HOLDER_B,
            },
        ],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody budget");
    let report = build.report_for(RecipeId(1)).expect("the recipe ran");
    assert_eq!(report.outcome, RecipeOutcome::Captured);
    assert_eq!(
        report.planned_ranges, 4,
        "two windows reached through DIFFERENT pointers carry different anchors, so the \
         generic tier cannot bridge between them",
    );
    assert_eq!(
        build.snapshot.custody_bytes(),
        16 + 16 + 8 + 8,
        "and the hole between the windows was not copied",
    );
}

#[test]
fn a_reserved_table_is_charged_to_custody_exactly_once() {
    use super::executor::DispatchProbe;

    const ROOTS: usize = 4_096;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, ROOTS * 64);
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
        (0..ROOTS)
            .map(|index| Root {
                op: 0,
                addr: BASE + index as u64 * 64,
            })
            .collect(),
    );
    let recipes = [recipe];
    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");

    // Nothing has been copied and no interval exists, so the ownership map has
    assert_eq!(
        probe.ledger_table_bytes(),
        probe.own_tables_charged(),
        "before the reservation the two accountings already agree",
    );

    let before_reserve = probe.ledger_table_bytes();
    let capacity_before = probe.wave_capacity(0);

    probe
        .reserve_root_queue()
        .expect("the default budget fits it");
    let after_reserve = probe.ledger_table_bytes();
    let reserved_capacity = probe.wave_capacity(0);
    assert!(
        reserved_capacity >= ROOTS && reserved_capacity > capacity_before,
        "the reservation gave the queue room for the roots: {reserved_capacity} slots, was \
         {capacity_before}",
    );
    assert!(
        after_reserve > before_reserve,
        "and charged for them: {after_reserve} table bytes, was {before_reserve}",
    );

    assert_eq!(probe.seed_roots(), ROOTS, "every root was queued");
    assert_eq!(
        probe.wave_capacity(0),
        reserved_capacity,
        "seeding did not have to grow the queue the reservation sized",
    );
    probe.charge_tables().expect("the default budget fits it");

    assert_eq!(
        probe.ledger_table_bytes(),
        probe.own_tables_charged(),
        "the reservation was charged once: the ledger holds {} table bytes against a \
         reconciled footprint of {}",
        probe.ledger_table_bytes(),
        probe.own_tables_charged(),
    );
    assert!(
        probe.ledger_table_bytes() >= after_reserve,
        "reconciliation never releases what the reservation charged",
    );
}

#[test]
fn a_bounded_joins_task_anchors_on_the_element_address_it_computed() {
    const TABLE: u64 = BASE + 0x1000;
    const STRIDE: u64 = 16;
    const ELEMENTS: u32 = 4;
    const KEY: u32 = 2;
    const NEAR: i64 = 0x200;
    const FAR: i64 = NEAR + 8 + GENERIC_GAP_BYTES as i64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 24);
    memory.write_u32(BASE, ELEMENTS);
    memory.write_u64(BASE + 8, TABLE);
    memory.write_u32(BASE + 16, KEY);
    memory.map_zeroed(TABLE, 0x400);
    let element = TABLE + u64::from(KEY) * STRIDE;
    memory.write_u64(element + NEAR as u64, 0xE1E1_0000_0002);
    memory.write_u64(element + FAR as u64, 0xFAFA_0000_0002);
    for other in [KEY - 1, KEY + 1] {
        let base = TABLE + u64::from(other) * STRIDE;
        memory.write_u64(base + NEAR as u64, 0xE1E1_0000_0000 + u64::from(other));
        memory.write_u64(base + FAR as u64, 0xFAFA_0000_0000 + u64::from(other));
    }

    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 24,
                next: vec![1, 2],
            },
            RecipeOp::DecodeExtent {
                count_offset: 0,
                count_width: 4,
                ptr_offset: 8,
                stride: STRIDE as u32,
                max_count: 16,
                max_bytes: 4096,
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(7)),
                element: vec![4],
            },
            RecipeOp::JoinBounded {
                table: TableId(7),
                key_offset: 16,
                key_width: 4,
                next: 3,
            },
            RecipeOp::CopyRanges {
                windows: vec![RelWindow::new(NEAR, 8), RelWindow::new(FAR, 8)],
                next: vec![4],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );

    let table_end = TABLE + u64::from(ELEMENTS) * STRIDE;
    assert!(
        element + NEAR as u64 > table_end,
        "the near window must clear the element array, or it merges into the extent's own \
         copy and the merged anchor is Mixed",
    );
    assert!(
        element + FAR as u64 + 8 > table_end,
        "and the merged footprint must leave the proven extent, or the 128 B proven tier \
         decides the merge instead of the 32 B anchor tier",
    );

    let build = observe(&[recipe], &memory, config()).expect("custody budget");
    let report = build.report_for(RecipeId(1)).expect("the recipe ran");
    assert_eq!(report.outcome, RecipeOutcome::Captured);
    assert_eq!(
        report.planned_ranges, 3,
        "the join's windows share the pointer anchor of the element address the join \
         computed, so the generic tier bridges the 32 B hole between them",
    );
    assert_eq!(
        build.snapshot.custody_bytes(),
        24 + 64 + 48,
        "the bridged hole was copied, which is what a merge at the generic tier means",
    );

    let closed = build.close_witnesses();
    let view = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert_eq!(
        view.u64_le(element + NEAR as u64).expect("captured"),
        0xE1E1_0000_0002,
        "the near window was taken at element {KEY}, not at a neighbour",
    );
    assert_eq!(
        view.u64_le(element + FAR as u64).expect("captured"),
        0xFAFA_0000_0002,
        "and so was the far one",
    );
}

#[test]
fn the_root_reservation_is_bounded_by_what_a_recipe_may_enqueue() {
    use super::executor::DispatchProbe;

    const ROOTS: usize = 100_000;
    const TASKS: u32 = 16;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, ROOTS * 64);
    let mut recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        (0..ROOTS)
            .map(|index| Root {
                op: 0,
                addr: BASE + index as u64 * 64,
            })
            .collect(),
    );
    recipe.limits.max_copy_tasks = TASKS;
    let recipes = [recipe];

    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");
    probe
        .reserve_root_queue()
        .expect("the default budget fits it");
    assert!(
        probe.wave_capacity(0) <= 2 * TASKS as usize,
        "the queue was sized for the {TASKS} tasks the limit admits, not the {ROOTS} roots \
         the recipe declared; it has room for {}",
        probe.wave_capacity(0),
    );

    let build = observe(&recipes, &memory, config()).expect("no custody refusal");
    let report = build.report_for(RecipeId(1)).expect("the recipe ran");
    assert!(
        matches!(
            report.outcome,
            RecipeOutcome::Refused(RefusalRecord {
                refusal: Refusal::CopyTaskLimitExceeded { limit: TASKS, .. },
                ..
            })
        ),
        "the declared roots are refused by the recipe's own task limit, not by custody: {:?}",
        report.outcome,
    );
}

#[test]
fn a_recipe_that_fails_validation_does_not_size_the_root_reservation() {
    use super::executor::DispatchProbe;

    const ROOTS: usize = 50_000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 64);

    let malformed = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![RecipeOp::CopyHeader {
            size: 16,
            next: vec![0],
        }],
        (0..ROOTS).map(|_| Root { op: 0, addr: BASE }).collect(),
    );
    let sound = Recipe::new(
        RecipeId(2),
        ConsumerId(2),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ],
        vec![Root { op: 0, addr: BASE }],
    );
    let recipes = [malformed, sound];

    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");
    probe.validate_recipes();
    probe
        .reserve_root_queue()
        .expect("the default budget fits it");
    assert!(
        probe.wave_capacity(0) <= 32,
        "only the sound recipe's single root may be reserved for, not the malformed \
         recipe's {ROOTS}; the queue has room for {}",
        probe.wave_capacity(0),
    );

    let sound_only = observe(&recipes[1..], &memory, config()).expect("no custody refusal");
    let build = observe(&recipes, &memory, config()).expect("no custody refusal");
    let added = build.custody_held - sound_only.custody_held;
    assert!(
        added < ROOTS as u64 * 48,
        "adding the malformed recipe added {added} bytes of custody. Its roots are entitled \
         to the discovery visited set's preallocation and nothing else; a queue reserved for \
         them is {} bytes on top of that",
        ROOTS as u64 * 64,
    );

    assert!(
        matches!(
            outcome(&build, 1),
            RecipeOutcome::Refused(RefusalRecord {
                refusal: Refusal::Malformed { .. },
                ..
            })
        ),
        "the malformed recipe refuses on its own account: {:?}",
        outcome(&build, 1),
    );
    assert_eq!(
        outcome(&build, 2),
        RecipeOutcome::Captured,
        "and the sound recipe sharing the observation is untouched by it",
    );
}

#[test]
fn the_reconciliation_counts_the_wave_that_is_in_flight() {
    use super::executor::DispatchProbe;

    let memory = chain_memory(64, 0);
    let recipes = [chain_recipe(1, 1)];
    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");
    probe.reserve_root_queue().expect("custody");
    probe.seed_roots();
    probe.charge_tables().expect("custody");

    let tasks = probe.take_wave(0);
    assert!(!tasks.is_empty(), "there is a wave to hold");
    let in_flight = DispatchProbe::<SyntheticMemory>::wave_bytes(&tasks);
    assert!(in_flight > 0, "and holding it costs something");

    probe.copy_pass(&tasks).expect("custody");
    probe.decode_pass(&tasks);
    assert!(
        probe.queued_at(1) > 0,
        "the decode dispatch discovered into the next wave, so a table grew while the \
         wave was out of sight",
    );

    probe.charge_tables_holding(&tasks).expect("custody");

    // The ledger also carries the ownership map's records and link arenas,
    let floor = probe.own_tables_bytes(0) + in_flight;
    assert!(
        probe.own_tables_charged() >= floor,
        "custody must never sit below the footprint: the executor is charged for {} against \
         {floor} held ({} in tables it still owns, {in_flight} in the wave in hand)",
        probe.own_tables_charged(),
        probe.own_tables_bytes(0),
    );
    assert!(probe.ledger_table_bytes() >= probe.own_tables_charged());
}

#[test]
fn discovery_past_depth_zero_runs_on_queues_that_were_not_reserved() {
    let memory = chain_memory(6, 0);
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 0,
            },
            RecipeOp::Freeze,
        ],
        vec![
            Root {
                op: 0,
                addr: node_addr(0),
            },
            Root {
                op: 0,
                addr: node_addr(3),
            },
        ],
    );
    let build = observe(&[recipe], &memory, config()).expect("custody budget");
    assert_eq!(
        build.report_for(RecipeId(1)).unwrap().outcome,
        RecipeOutcome::Captured,
    );
    assert_eq!(
        build.snapshot.interval_count(),
        6,
        "every node of the chain was captured exactly once, from both entry points",
    );
    assert_eq!(
        build.snapshot.custody_bytes(),
        6 * 16,
        "and nothing was copied twice",
    );
}

#[test]
fn the_visited_key_does_not_alias_two_recipes() {
    let mut visited = executor::VisitedSet::default();
    let low = executor::visit_tag(0, 3);
    let past_a_u32 = executor::visit_tag(1usize << 32, 3);
    assert_ne!(low, past_a_u32, "the recipe index must survive the packing");
    assert!(visited.insert_op(0x1000, low));
    assert!(
        visited.insert_op(0x1000, past_a_u32),
        "a second recipe's task at the same address is not a duplicate",
    );
    assert_ne!(executor::visit_tag(0, 1), executor::visit_tag(1, 0));
}

#[test]
fn a_gap_request_wins_whatever_order_the_requests_arrive_in() {
    use super::annot::Annotations;
    use super::{ConsumerId, IntervalId};

    let interval = IntervalId(7);
    let consumer = ConsumerId(81);

    let mut gap_first = Annotations::new();
    gap_first.arm();
    gap_first.ask_close(interval, consumer, true);
    gap_first.ask_close(interval, consumer, false);
    assert!(
        gap_first.close_gaps(interval, consumer),
        "a consumer that asked to be gapped must be gapped, whatever it asked \
         for afterwards",
    );

    let mut record_first = Annotations::new();
    record_first.arm();
    record_first.ask_close(interval, consumer, false);
    record_first.ask_close(interval, consumer, true);
    assert!(
        record_first.close_gaps(interval, consumer),
        "the order requests arrive in must not decide the policy",
    );

    let mut record_only = Annotations::new();
    record_only.arm();
    record_only.ask_close(interval, consumer, false);
    assert!(
        !record_only.close_gaps(interval, consumer),
        "a consumer whose every request said `Record` renders its own verdict",
    );

    let neighbour = ConsumerId(47);
    assert!(
        record_only.close_gaps(interval, neighbour),
        "a consumer that never asked takes the substrate's verdict",
    );

    assert!(record_only.close_gaps(IntervalId(9), consumer));
}

#[test]
fn one_annotation_entry_keeps_its_five_parts_independently_absent() {
    use super::annot::Annotations;
    let interval = IntervalId(3);
    let mut annot = Annotations::new();
    annot.arm();
    annot.tag_stage(interval, 7);

    assert_eq!(annot.stages_of(interval), &[7]);
    assert!(
        annot.consumption(interval).is_none(),
        "a stage tag records no consumption, and an absent consumption is how \
         'this contributor reads all of it' is spelled",
    );
    assert!(
        annot.consumed_touches(interval, &Span::new(0, 8)),
        "and absent therefore reads as fully consumed",
    );
    assert!(
        annot.samples(interval).is_none(),
        "a stage tag records no sampling request",
    );
    assert!(
        annot.closing_images(interval).is_none(),
        "a stage tag takes no closing image",
    );
    assert_eq!(
        annot.closing_images_wanted(interval),
        1,
        "and an interval nobody asked about takes the one closing image every \
         recipe took before the sequence existed",
    );
    assert!(
        annot.close_gaps(interval, ConsumerId(1)),
        "nobody asked, so the substrate's own verdict stands",
    );
}

#[test]
fn sampled_intervals_iterate_in_ascending_id_order_not_insertion_order() {
    use super::annot::Annotations;
    let consumer = ConsumerId(2);
    let mut annot = Annotations::new();
    annot.arm();
    for id in [9u32, 4, 1] {
        annot.entry(IntervalId(id)).ask_samples(consumer, 2, true);
    }

    let seen: Vec<u32> = annot
        .sampled_intervals()
        .map(|(id, _images)| id.0)
        .collect();
    assert_eq!(
        seen,
        vec![1, 4, 9],
        "close_witnesses walks this iterator, so its order is the order the \
         torn and uncertified lists reach the wire in",
    );
}

/// `IntervalId` and `Annotations::entry` are both `pub`, so "the ownership map
#[test]
fn a_distant_interval_id_goes_to_the_sparse_tail_and_still_iterates_in_order() {
    use super::annot::Annotations;
    let consumer = ConsumerId(4);

    let mut annot = Annotations::new();
    annot.arm();
    annot
        .entry(IntervalId(u32::MAX))
        .ask_samples(consumer, 2, false);
    assert!(
        annot.capacity_bytes() < 64 * 1024,
        "a distant id must not allocate the gap; charged {} bytes",
        annot.capacity_bytes(),
    );
    annot.entry(IntervalId(7)).ask_samples(consumer, 2, false);
    assert!(
        annot.capacity_bytes() < 64 * 1024,
        "and the near id does not change that; charged {} bytes",
        annot.capacity_bytes(),
    );
    let seen: Vec<u32> = annot
        .sampled_intervals()
        .map(|(id, _images)| id.0)
        .collect();
    assert_eq!(
        seen,
        vec![7, u32::MAX],
        "ascending, once each, across both tables",
    );
    assert!(annot.samples(IntervalId(7)).is_some());
    assert!(annot.samples(IntervalId(u32::MAX)).is_some());
    assert!(annot.samples(IntervalId(8)).is_none());

    let mut straddle = Annotations::new();
    straddle.arm();
    for id in [1025u32, 0, 1026] {
        straddle
            .entry(IntervalId(id))
            .ask_samples(consumer, 2, false);
    }
    let straddled: Vec<u32> = straddle
        .sampled_intervals()
        .map(|(id, _images)| id.0)
        .collect();
    assert_eq!(
        straddled,
        vec![0, 1025, 1026],
        "the dense walk and the sparse tail are CHAINED, so every dense id must \
         sort below every sparse one",
    );
}

#[test]
fn a_sampling_ask_appends_without_replacing_the_images_already_taken() {
    use super::annot::Annotations;
    let interval = IntervalId(5);
    let consumer = ConsumerId(9);
    let mut annot = Annotations::new();
    annot.arm();

    annot.entry(interval).ask_samples(consumer, 2, true);
    let images = annot.samples(interval).expect("the ask created the record");
    assert_eq!(images.held(), 1);
    assert_eq!(images.success_mask, 1);
    assert_eq!(images.asks.len(), 1);
    let asks = images.asks.clone();

    annot.set_samples(
        interval,
        super::annot::SampleImages {
            extra: vec![0x40],
            success_mask: 0b11,
            asks,
        },
    );
    annot.entry(interval).ask_samples(ConsumerId(10), 3, false);
    let images = annot.samples(interval).expect("still recorded");
    assert_eq!(
        images.held(),
        2,
        "the ask must not replace the images the copy wave took",
    );
    assert_eq!(images.success_mask, 0b11);
    assert_eq!(images.asks.len(), 2, "and both consumers' asks stand");
    assert_eq!(
        annot.closing_images_wanted(interval),
        3,
        "the closing wave takes as many images as the greediest ask wanted",
    );
}

#[test]
fn the_fixture_memorys_indexed_lookup_answers_what_the_scan_answered() {
    let mut overlapping = SyntheticMemory::new();
    overlapping.map(0x1000, vec![0xAAu8; 0x200]);
    overlapping.map(0x1080, vec![0xBBu8; 0x80]);
    let mut out = [0u8; 4];
    overlapping
        .copy_into(0x1100, &mut out)
        .expect("both regions contain this address");
    assert_eq!(
        out, [0xAAu8; 4],
        "the region mapped FIRST serves the read, which is what the scan did",
    );

    let mut disjoint = SyntheticMemory::new();
    disjoint.map(0x1000, vec![0x11u8; 0x10]);
    disjoint.map(0x1010, vec![0x22u8; 0x10]);
    let mut out = [0u8; 8];
    disjoint
        .copy_into(0x100C, &mut out)
        .expect_err("a read across two regions is not a read of either");
    disjoint
        .copy_into(0x1010, &mut out)
        .expect("and each region still serves its own bytes");
    assert_eq!(out, [0x22u8; 8]);

    disjoint.map(0x2000, vec![0x33u8; 0x10]);
    disjoint
        .copy_into(0x2000, &mut out)
        .expect("the index cannot outlive the region list it indexed");
    assert_eq!(out, [0x33u8; 8]);

    let mut boundary = SyntheticMemory::new();
    boundary.map(0x1000, vec![0x11u8; 0x10]);
    boundary.map(0x1010, vec![0x22u8; 0x10]);
    boundary.set_readable(0x1000, false);
    boundary
        .copy_into(0x1010, &mut [])
        .expect_err("the region mapped FIRST owns the boundary, and it refuses");
}
