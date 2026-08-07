//! Unit tests for the snapshot-recipe substrate.
//!
//! Every test here asks a question the substrate could get wrong in a way that
//! would only show up as a wrong capture: a cap that clamps instead of
//! refusing, a merge that bridges bytes nothing proved were safe, a shared
//! extent that gaps the wrong consumers, a view that hands one reader another
//! reader's bytes or another hour's.
//!
//! The tests marked SENTINEL are the ones a mutation must break. Each names
//! the production change it is watching for.

use super::arena::{Arena, ArenaAllocator};
use super::dedup::OwnershipMap;
use super::executor::{
    observe, observe_in, validate, ObservationConfig, Recipe, RecipeOp, RecipeOutcome, RelWindow,
    Root, SnapshotBuild,
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

/// `nodes` 16-byte objects, each pointing at the next. The last one points at
/// `tail` (0 ends the chain, a node address makes it a cycle).
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

/// `CopyHeader(16) -> DecodePointer(+0) -> itself`, terminated by `Freeze`.
fn chain_recipe(id: u32, consumer: u16) -> Recipe {
    Recipe::new(
        RecipeId(id),
        ConsumerId(consumer),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer { offset: 0, next: 0 },
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
        RecipeOutcome::Refused(refusal) => refusal,
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

// ---------------------------------------------------------------------------
// Coalescing policy
// ---------------------------------------------------------------------------

/// SENTINEL. Watches for any merge rule that consults only addresses.
///
/// Two ranges eight bytes apart — far inside every tier's allowance — with no
/// shared provenance must stay two copy transactions. Deleting the `_ => 0`
/// arm of `merge_allowance`, or making it return the generic gap, makes this
/// fail.
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

/// SENTINEL. The gap-curve verdict's §4 correction: a window anchor is a
/// DEREFERENCED POINTER VALUE, never a root read event. Treating
/// `AnchorKind::Root` as a generic anchor makes this fail.
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

/// SENTINEL. A merge may not leave the region that justified it. Dropping the
/// `region.contains_span(&merged)` guard makes this fail.
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
    // The second range starts inside the object and ends past it, so the hull
    // leaves the allocation. The anchor is a root, so nothing weaker rescues
    // the merge.
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
    // And a Mixed anchor cannot then justify a further merge.
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

// ---------------------------------------------------------------------------
// Depth, waves and cycles
// ---------------------------------------------------------------------------

/// A depth-12 chain needs THIRTEEN forward waves, because roots are depth 0.
#[test]
fn a_depth_twelve_chain_takes_thirteen_forward_waves() {
    let memory = chain_memory(13, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(build.forward_waves_run, FORWARD_WAVES);
    assert_eq!(build.forward_waves_run, 13);
    assert_eq!(
        build.report_for(RecipeId(1)).unwrap().copy_tasks,
        13,
        "one copy task per node"
    );
    // No witness was asked for, so the fourteenth phase does not run.
    assert_eq!(build.phases_run, 13);
    assert_eq!(build.witnessed_intervals, 0);
}

/// SENTINEL. Depth 13 is past the measured maximum and must refuse by name,
/// not silently stop discovering.
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
        Some(GapReason::Refused(expected)),
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

/// SENTINEL. A pointer cycle must terminate WITHOUT refusing: removing the
/// visited-set prune in `enqueue` turns this into a `DepthExceeded`.
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

// ---------------------------------------------------------------------------
// Count and byte caps
// ---------------------------------------------------------------------------

#[test]
fn the_byte_cap_refuses_rather_than_stopping_early() {
    let memory = chain_memory(13, 0);
    let mut recipe = chain_recipe(1, 1);
    recipe.limits.max_bytes = 40; // two 16-byte headers fit, the third does not
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
                // Three windows, each far past the 32 B generic tier apart, so
                // they cannot coalesce into fewer transactions.
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
    // The same three windows, now within the generic tier of each other and
    // reached through a dereferenced pointer: one transaction.
    let mut recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 8,
                next: vec![1],
            },
            RecipeOp::DecodePointer { offset: 0, next: 2 },
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
    assert_eq!(report.planned_ranges, 2, "the header plus one merged window");
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

// ---------------------------------------------------------------------------
// Extents and bounded joins
// ---------------------------------------------------------------------------

/// Container at BASE: `u32` count at +0, `u64` array pointer at +8.
/// Array at BASE + 0x1000: `count` `u64` pointers to 8-byte leaves.
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
                register: Some(TableId(1)),
                element: vec![2],
            },
            RecipeOp::DecodePointer { offset: 0, next: 3 },
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
    // Wave 0 header, wave 1 array, wave 2 leaves: elements do NOT cost a wave
    // of their own.
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
    memory.write_u32(BASE, 4); // element count
    memory.write_u64(BASE + 8, BASE + 0x1000); // table base
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
    // Element 2 of a 16-byte-stride table starting at BASE + 0x1000.
    assert_eq!(view.u64_le(BASE + 0x1000 + 32).unwrap(), 0x5002);
}

/// SENTINEL. A key past the decoded element count must refuse, not clamp.
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

// ---------------------------------------------------------------------------
// Memory and field refusals
// ---------------------------------------------------------------------------

/// SENTINEL. Decoding a field the recipe never copied must refuse. Returning
/// zero from `read_uint`'s miss path instead makes this pass silently, with a
/// null pointer and no captured child.
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
            // The pointer lives at +16, outside the 8 bytes that were copied.
            RecipeOp::DecodePointer {
                offset: 16,
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
    // Recipe 2 refuses on its own depth limit, not on recipe 1's unreadable
    // span: the two failures are independent.
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

// ---------------------------------------------------------------------------
// Deduplication and dependency lists
// ---------------------------------------------------------------------------

#[test]
fn a_shared_object_is_copied_once_and_carries_both_dependents() {
    let memory = chain_memory(3, 0);
    let build = observe(&[chain_recipe(1, 1), chain_recipe(2, 2)], &memory, config())
        .expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    assert_eq!(outcome(&build, 2), RecipeOutcome::Captured);
    assert_eq!(memory.copies(), 3, "three objects, two recipes, three copies");
    assert_eq!(build.snapshot.interval_count(), 3);
    for record in build.snapshot.intervals() {
        assert!(record.depends(ConsumerId(1)));
        assert!(record.depends(ConsumerId(2)));
    }
    // Both recipes are still charged for what they asked for: deduplication is
    // a copier saving, not an accounting fiction.
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
        .map(|record| record.span)
        .collect();
    assert_eq!(
        spans,
        vec![Span::new(BASE, 0x40), Span::new(BASE + 0x40, 0x20)]
    );
    assert_eq!(memory.bytes_moved(), 0x60, "0x20 bytes are not re-copied");
    // The already-owned half still gains the second consumer.
    assert!(build.snapshot.intervals()[0].depends(ConsumerId(1)));
    assert!(build.snapshot.intervals()[0].depends(ConsumerId(2)));
    assert!(!build.snapshot.intervals()[1].depends(ConsumerId(1)));
    assert!(build.snapshot.intervals()[1].depends(ConsumerId(2)));
}

#[test]
fn ownership_planning_splits_around_owned_intervals() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x10), 0);
    owners.insert(Span::new(0x1030, 0x10), 0x10);
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

/// SENTINEL. An interval starting exactly at the query address is reachable
/// both by "the last interval starting at or before" and by "intervals
/// starting at or after". Reporting it twice makes an owned span look like two
/// owners, which is what `plan_into`'s `existing` list is counted from.
#[test]
fn an_interval_starting_at_the_query_address_is_reported_once() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x10), 0);
    owners.insert(Span::new(0x1010, 0x10), 0x10);

    assert_eq!(owners.overlapping(Span::new(0x1000, 0x10)).len(), 1);
    assert_eq!(owners.overlapping(Span::new(0x1000, 0x20)).len(), 2);
    assert_eq!(owners.overlapping(Span::new(0x1008, 0x10)).len(), 2);
    assert_eq!(owners.overlapping(Span::new(0x1010, 0x8)).len(), 1);
    assert!(owners.overlapping(Span::new(0x2000, 0x10)).is_empty());
}

// ---------------------------------------------------------------------------
// Custody
// ---------------------------------------------------------------------------

#[test]
fn custody_exhaustion_fails_the_observation_closed() {
    let memory = chain_memory(3, 0);
    let arena = Arena::with_budget(24);
    let result = observe_in(&[chain_recipe(1, 1)], &memory, config(), arena);
    assert_eq!(
        result.err(),
        Some(CustodyRefusal::BudgetExhausted {
            budget: 24,
            held: 16,
            requested: 16,
        })
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
    let arena = Arena::with_allocator(DEFAULT_CUSTODY_BUDGET_BYTES, Box::new(NeverAllocates));
    let result = observe_in(&[chain_recipe(1, 1)], &memory, config(), arena);
    assert_eq!(
        result.err(),
        Some(CustodyRefusal::AllocationFailed {
            held: 0,
            requested: 16,
        })
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
            RecipeOp::DecodePointer { offset: 0, next: 0 },
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

// ---------------------------------------------------------------------------
// A/B witnesses
// ---------------------------------------------------------------------------

fn witness_recipe(id: u32, consumer: u16, root: u64) -> Recipe {
    Recipe::new(
        RecipeId(id),
        ConsumerId(consumer),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer { offset: 0, next: 3 },
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
    let build =
        observe(&[witness_recipe(1, 1, node_addr(0))], &memory, config()).expect("custody");
    assert_eq!(build.witnessed_intervals, 2);
    assert_eq!(build.phases_run, 3, "two forward waves plus the closing wave");
    let closed = build.close_witnesses();
    assert!(closed.witness.changed.is_empty());
    assert!(closed.witness.gapped.is_empty());
    assert!(closed.snapshot.view_for(ConsumerId(1), HOUR).is_ok());
}

/// SENTINEL. A parent that moved invalidates its dependent subtree. Dropping
/// the edge walk in `close_witnesses` leaves the child interval valid.
#[test]
fn a_changed_parent_invalidates_its_dependent_subtree() {
    // Copies 1 and 2 are the forward waves; copy 3 is the first closing copy.
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
        4, // the second closing copy: the leaf
        vec![(node_addr(1) + 8, 0x77u64.to_le_bytes().to_vec())],
    );
    let build =
        observe(&[witness_recipe(1, 1, node_addr(0))], &mutating, config()).expect("custody");
    let closed = build.close_witnesses();
    assert_eq!(closed.witness.changed.len(), 1);
    assert!(closed.witness.invalidated.is_empty());
    assert!(closed.snapshot.gap_reason(ConsumerId(1)).is_some());
}

/// SENTINEL. Per-bracket independence: one family's failed witness must not
/// cost another family its hour.
#[test]
fn a_failed_witness_gaps_only_the_affected_consumers() {
    let build_fixture = || {
        let mut memory = SyntheticMemory::new();
        for index in 0..4 {
            memory.map_zeroed(node_addr(index), 16);
        }
        // Two disjoint two-node chains: 0 -> 1 and 2 -> 3.
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

    // Four forward copies, then four closing ones; copy 5 is the first close.
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

/// Refuses reads of one address once `deny_after` copies have succeeded.
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

/// SENTINEL. A witness that cannot be re-copied is a FAILED witness, never an
/// absent one. Treating `closing_offset == None` as unchanged lets a region
/// that became unreadable mid-bracket certify itself.
#[test]
fn a_witness_that_cannot_be_re_copied_counts_as_changed() {
    let staged = StagedMemory {
        inner: chain_memory(2, 0),
        deny_after: 2,
        seen: std::cell::Cell::new(0),
        deny: node_addr(1),
    };
    let build =
        observe(&[witness_recipe(1, 1, node_addr(0))], &staged, config()).expect("custody");
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
    let recipes = vec![
        chain_recipe(1, 1),                 // no witness
        witness_recipe(2, 2, node_addr(0)), // witness
    ];
    let build = observe(&recipes, &memory, config()).expect("custody");
    assert_eq!(
        build.witnessed_intervals, 2,
        "sharing widens the closing wave, it never narrows it"
    );
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

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
    // The translated offset indexes a buffer this process owns; it has no
    // relationship to the original address beyond the mapping.
    assert_eq!(view.translate(node_addr(0)).unwrap(), 0);
    assert_eq!(view.translate(node_addr(0) + 4).unwrap(), 4);
    assert_eq!(view.translate(node_addr(1)).unwrap(), 16);
    assert_eq!(view.hour(), HOUR);
    assert_eq!(view.consumer(), ConsumerId(1));
}

/// SENTINEL. Own-hour only. Dropping the hour check in `view_for` hands hour-H
/// bytes to an hour-H+1 reader.
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

/// SENTINEL. A consumer may only see what its own recipe asked for. Sharing an
/// extent is a copier detail, never a widening of what a reader may look at.
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

/// SENTINEL. The page index must answer exactly what a plain ordered search
/// answers, INCLUDING for intervals that span into a page where a later
/// interval starts.
///
/// A bucket names the first interval STARTING in its page, so without the
/// step-back in `Snapshot::locate` an address inside a page-spanning interval
/// resolves to the interval AFTER it and reports as never captured. The
/// fixture below is built to contain that shape: `sizes` mixes widths from 8
/// bytes to more than two pages, so several intervals begin in a page that an
/// earlier interval is still covering.
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
        // Advance by the object plus a hole, so intervals stay disjoint and
        // the next one often begins inside a page the last one reached into.
        addr += u64::from(size) + 0x30 + (index as u64 % 7) * 0x40;
    }
    let closed = observe(&recipes, &memory, config())
        .expect("custody")
        .close_witnesses();
    assert_eq!(closed.snapshot.interval_count(), spans.len());
    assert!(closed.snapshot.index().buckets() > 1);

    // The shape this test exists for must actually be present.
    let intervals = closed.snapshot.intervals();
    let straddles = intervals.windows(2).any(|pair| {
        let earlier_end = pair[0].span.end_excl();
        let later_page = pair[1].span.start >> super::view::PAGE_SHIFT;
        let earlier_start_page = pair[0].span.start >> super::view::PAGE_SHIFT;
        earlier_start_page < later_page && earlier_end > u128::from(pair[1].span.start & !0xFFF)
    });
    assert!(
        straddles,
        "fixture must contain an interval spanning into a page where a later interval starts"
    );

    let last = intervals.last().unwrap().span;
    let top = u64::try_from(last.end_excl()).unwrap() + 0x2000;
    let mut addr = BASE - 0x2000;
    while addr < top {
        let (indexed, searched) = closed.snapshot.locate_both(addr);
        assert_eq!(indexed, searched, "disagreement at {addr:#x}");
        addr += 1;
    }
}

// ---------------------------------------------------------------------------
// Recipe validation
// ---------------------------------------------------------------------------

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
        RecipeOp::DecodePointer { offset: 0, next: 2 },
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
    assert_eq!(malformed(dangling), MalformedReason::DanglingBranch { op: 0 });

    let mut zero_size = base();
    zero_size.ops[0] = RecipeOp::CopyHeader {
        size: 0,
        next: vec![1],
    };
    assert_eq!(malformed(zero_size), MalformedReason::ZeroSizedCopy { op: 0 });

    let mut deep = base();
    deep.limits.max_depth = MEASURED_MAX_DISCOVERY_DEPTH + 1;
    assert_eq!(
        malformed(deep),
        MalformedReason::DepthLimitAboveMeasuredMaximum {
            limit: MEASURED_MAX_DISCOVERY_DEPTH + 1
        }
    );

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
            register: None,
            element: vec![2],
        },
        RecipeOp::Freeze,
    ];
    assert_eq!(malformed(zero_stride), MalformedReason::ZeroStride { op: 1 });
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

// ---------------------------------------------------------------------------
// Span arithmetic and constants
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Findings from the round-1 review
// ---------------------------------------------------------------------------

/// SENTINEL (review finding 1). Overlapping another consumer's object by one
/// byte must not hand over the rest of it. Reverting the view's `reaches`
/// check to a plain `depends` check makes this fail.
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
    // A owns [BASE, BASE+0x40). B asks for [BASE+0x20, BASE+0x60), so it
    // overlaps A's interval only from +0x20 on.
    let closed = observe(
        &[
            header(1, 1, BASE, 0x40),
            header(2, 2, BASE + 0x20, 0x40),
        ],
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
    // What B did ask for it can still read, across the split.
    assert!(b.bytes(BASE + 0x20, 0x20).is_ok());
    let mut out = [0u8; 0x40];
    b.read_into(BASE + 0x20, &mut out).expect("B's own bytes");
    // And A still sees its whole object.
    let a = closed.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    assert!(a.bytes(BASE, 0x40).is_ok());
}

/// SENTINEL (review finding 3). A window whose offset carries its base off the
/// address space is a refusal, not a window to skip.
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

/// SENTINEL (review finding 3). The same rule for a decode-op field offset.
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

/// SENTINEL (review finding 4). Two headers can name one `DecodeExtent` op
/// against the same array base and decode different counts. Keying the
/// visited set on the base alone prunes the longer one as a duplicate of the
/// shorter, copies a prefix, and reports `Captured`.
#[test]
fn two_extents_at_one_base_with_different_counts_are_both_expanded() {
    let mut memory = SyntheticMemory::new();
    // Two container headers, both pointing at the same array.
    memory.map_zeroed(BASE, 16);
    memory.map_zeroed(BASE + 0x100, 16);
    let array = BASE + 0x1000;
    memory.map_zeroed(array, 16);
    memory.write_u32(BASE, 1); // the short one first
    memory.write_u64(BASE + 8, array);
    memory.write_u32(BASE + 0x100, 2); // the long one second
    memory.write_u64(BASE + 0x100 + 8, array);
    // Two leaves, one per element.
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
                register: None,
                element: vec![2],
            },
            RecipeOp::DecodePointer { offset: 0, next: 3 },
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
    // BOTH leaves must be present: the count-2 extent is not a duplicate of
    // the count-1 extent that happens to start at the same address.
    assert_eq!(view.u64_le(BASE + 0x2000).unwrap(), 0xC0DE);
    assert_eq!(
        view.u64_le(BASE + 0x2000 + 0x100).unwrap(),
        0xC0DF,
        "the second element was pruned as a duplicate"
    );
}

/// SENTINEL (review finding 5). Deduplication makes one interval both a
/// discovered child of A's chain and a root of B's. When A's parent moves, B
/// reached those bytes by its own route and its hour survives.
#[test]
fn a_changed_parent_does_not_gap_a_consumer_that_took_another_route() {
    let fixture = || {
        let mut memory = SyntheticMemory::new();
        memory.map_zeroed(node_addr(0), 16);
        memory.map_zeroed(node_addr(1), 16);
        memory.write_u64(node_addr(0), node_addr(1));
        memory
    };
    // A walks 0 -> 1. B roots directly at 1.
    let recipes = vec![
        witness_recipe(1, 1, node_addr(0)),
        witness_recipe(2, 2, node_addr(1)),
    ];
    let baseline = observe(&recipes, &fixture(), config()).expect("custody");
    assert_eq!(
        baseline.witnessed_intervals, 2,
        "one shared interval plus A's parent"
    );

    // Two forward copies, then two closing ones; copy 3 changes node 0 only.
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

/// SENTINEL (review finding 7). Two owned intervals at the top and the bottom
/// of the address space are not adjacent.
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
    // The one byte that WAS copied still reads.
    let mut one = [0u8; 1];
    view.read_into(u64::MAX, &mut one).expect("own byte");
}

/// SENTINEL (review finding 9). 512 MiB is a ceiling, not a default a caller
/// may raise.
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
    // Lowering it is fine.
    let lowered = ObservationConfig::new(HOUR)
        .with_custody_budget(1024)
        .expect("lowering is allowed");
    assert_eq!(lowered.custody_budget(), 1024);

    // And the arena-supplying entry point is held to the same ceiling.
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
fn the_phase_count_is_fourteen() {
    assert_eq!(MEASURED_MAX_DISCOVERY_DEPTH, 12);
    assert_eq!(FORWARD_WAVES, 13);
    assert_eq!(TOTAL_PHASES, 14);
}
