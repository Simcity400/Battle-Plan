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
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 0 },
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

/// SENTINEL, HOST #172. Two requests carrying the IDENTICAL span must be
/// ordered the same way whichever path `plan_copies_into` takes, and the LAST
/// of them — the one `copy_attributes` attributes the merged copy to — must be
/// the one with the lowest request index.
///
/// The two arrangements differ only in where the unrelated requests sit, so the
/// equal pair keeps both its members and its relative emission order (and
/// therefore its request indices) across them. One arrangement is ascending
/// apart from the duplicate and the other is not, which is exactly the fork:
/// under the old `<` test the first took the fast path and emitted the pair in
/// emission order while the second sorted and emitted it in whatever order
/// `sort_unstable` chose.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL:
///  * `key <= previous` back to `key < previous` — the ascending arrangement
///    takes the fast path again and the two arrangements name different tasks;
///  * the `!index` third component back to `index`, or dropped — the pair comes
///    out lowest-index-first and the attributed task becomes the higher one.
#[test]
fn identical_spans_are_ordered_by_the_lowest_request_index_last() {
    let policy = GapPolicy::default();
    let anchor = Provenance::from_pointer(0x5000);
    // Indices 1 and 2 in BOTH arrangements, so "lowest index" names the same
    // request either way and the assertion is about the ORDER and not about
    // which request happened to be numbered lower.
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
        // `copy_attributes` is last-wins over exactly this run.
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

// ---------------------------------------------------------------------------
// Depth, waves and cycles
// ---------------------------------------------------------------------------

/// A depth-12 chain needs THIRTEEN forward waves, because roots are depth 0.
#[test]
fn a_depth_twelve_chain_takes_thirteen_forward_waves() {
    let memory = chain_memory(13, 0);
    let build = observe(&[chain_recipe(1, 1)], &memory, config()).expect("custody");
    assert_eq!(outcome(&build, 1), RecipeOutcome::Captured);
    // WAVES RUN, not waves available. `FORWARD_WAVES` is the ceiling; an
    // observation runs one wave per depth that actually had a task, which is
    // what makes raising the ceiling free for a shallow recipe.
    assert_eq!(build.forward_waves_run, 13);
    assert!(build.forward_waves_run < FORWARD_WAVES);
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
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 2 },
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
                trailer_elements: 0,
                guard: None,
                register: Some(TableId(1)),
                element: vec![2],
            },
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 3 },
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
// The guarded extent: MSVC-string-shaped containers
// ---------------------------------------------------------------------------

/// The address of the union-shaped object these fixtures lay out.
const STR_OBJ: u64 = 0x20_0000;
/// Where a heap payload is mapped in these fixtures.
const STR_HEAP: u64 = 0x30_0000;

/// One 0x20-byte MSVC `std::basic_string` at [`STR_OBJ`], plus a heap region.
///
/// `heap` chooses the arm: `Some(text)` writes a capacity at or above 16 and a
/// pointer at `+0x00`; `None` writes the characters INLINE and a capacity of 15
/// — the shape whose first eight bytes are ASCII, not an address.
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

/// `CopyHeader(0x20)` then a guarded, byte-strided, NUL-trailing extent.
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

/// SENTINEL. This is the whole reason [`ExtentGuard`] exists.
///
/// An MSVC `std::basic_string` at capacity 15 keeps its characters in the first
/// sixteen bytes of its own header. An UNGUARDED extent reads those characters
/// as a pointer and tries to copy at `0x0000_0074_726F_6873` ("short" little
/// endian), which is a whole-recipe `MemoryUnreadable`. Deleting the guard
/// check from `run_site`'s `DecodeExtent` arm makes the first half of this
/// fail; deleting the `value < guard.min` early return makes the second.
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
    // The branch ended CLEANLY: one frozen branch for the header's own chain
    // is not claimed, because the guard returned before the extent existed.
    let view = guarded
        .snapshot
        .view_for(ConsumerId(1), HOUR)
        .expect("a captured consumer has a view");
    // The characters are inside the header the recipe already copied, which is
    // exactly why not dereferencing loses nothing.
    assert_eq!(view.bytes(STR_OBJ, 5).expect("inline characters"), b"short");
    // And nothing at all was copied at the ASCII-as-address.
    assert!(matches!(
        view.bytes(0x0000_0074_726F_6873, 1),
        Err(ViewRefusal::NotCaptured { .. })
    ));
}

/// The other arm: capacity at or above the threshold DOES dereference, and the
/// payload lands in the snapshot.
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

/// SENTINEL. `trailer_elements` is why the copied path can reach the same
/// verdict as the live one.
///
/// The string decoder refuses a buffer whose byte at `[size]` is not NUL, so
/// the terminator has to be INSIDE the copy. With `trailer_elements: 0` the
/// byte at `size` was never copied and the post-bracket decode has nothing to
/// test; with 1 it is there. Setting the field back to zero, or adding it after
/// the limit check so it is not counted, makes this fail.
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
    let view = without.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
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

/// SENTINEL. The trailer is bounded like everything else.
///
/// A count of `max_count` with a one-element trailer asks for `max_count + 1`
/// and must refuse by name. Moving the addition after the check turns a hard
/// limit into a limit plus a trailer.
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
            // Exactly the declared size: the trailer is the byte that breaks it.
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

/// SENTINEL. A guard word the recipe never copied is a RECIPE defect, and it
/// must refuse rather than read as "the guard said no".
///
/// Treating an uncaptured guard as below the threshold would silently drop
/// every heap payload in the observation while reporting `Captured`.
#[test]
fn an_uncaptured_guard_word_refuses_rather_than_skipping() {
    let text = b"a rather longer string than sixteen bytes";
    let memory = msvc_string_memory(Some(text));
    let mut recipe = msvc_string_recipe(capacity_guard(), 1);
    // Copy only as far as the size field: the capacity at +0x18 is outside.
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

/// A terminal extent — no per-element decode ops — captures its bytes, counts
/// one frozen branch and pushes no per-element site.
///
/// The alternative spelling, `element: vec![freeze]`, is not equivalent: it
/// would count one frozen branch PER BYTE. That difference is what this pins.
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
    assert_eq!(
        terminal.report_for(RecipeId(1)).unwrap().frozen_branches,
        1
    );

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
        // The container's DECLARED elements, and not the trailer: the extra
        // byte is copied and is not an element, so nothing runs against it.
        text.len() as u32,
        "the per-element spelling runs the element ops once per byte, which is \
         the cost the empty list exists to avoid",
    );
}

/// SENTINEL. The guarded POINTER chase, and the container it exists for.
///
/// An MSVC `_Tree` sentinel is a node whose value slot was never constructed.
/// Chasing it copies whatever the allocator left there — here, an address in no
/// mapped region, which is a whole-recipe `MemoryUnreadable`. A guard on
/// `_Isnil` chases the value on real nodes and not on the sentinel.
///
/// Removing `guard` from the `DecodePointer` arm of `run_site`, or widening
/// `ExtentGuard::exactly` into an at-least, makes the first half fail.
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
    // The sentinel: `_Isnil` set, both children pointing at the one real node,
    // and a value slot holding a plausible-looking address that is mapped
    // nowhere.
    memory.write(HEAD + u64::from(ISNIL), &[1u8]);
    memory.write_u64(HEAD, NODE);
    memory.write_u64(HEAD + 0x10, NODE);
    memory.write_u64(HEAD + u64::from(VALUE), 0x7F_0000_0000);
    // The real node: nil children point back at the sentinel, value is a role.
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
    let view = guarded.snapshot.view_for(ConsumerId(1), HOUR).expect("view");
    // The real node's role IS copied ...
    assert!(view.bytes(ROLE, 0x10).is_ok());
    // ... the sentinel itself is copied, because a post-bracket walk has to be
    // able to see `_Isnil` for itself ...
    assert!(view.bytes(HEAD, 0x30).is_ok());
    // ... and nothing was taken at the address its dead value slot named.
    assert!(matches!(
        view.bytes(0x7F_0000_0000, 1),
        Err(ViewRefusal::NotCaptured { .. })
    ));
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
    let links = build.snapshot.links();
    for record in build.snapshot.intervals() {
        assert!(links.depends(record, ConsumerId(1)));
        assert!(links.depends(record, ConsumerId(2)));
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
        .map(|record| record.span())
        .collect();
    assert_eq!(
        spans,
        vec![Span::new(BASE, 0x40), Span::new(BASE + 0x40, 0x20)]
    );
    assert_eq!(memory.bytes_moved(), 0x60, "0x20 bytes are not re-copied");
    // The already-owned half still gains the second consumer.
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

/// SENTINEL. An interval starting exactly at the query address is reachable
/// both by "the last interval starting at or before" and by "intervals
/// starting at or after". Reporting it twice makes an owned span look like two
/// owners, which is what `plan_into`'s `existing` list is counted from.
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

// ---------------------------------------------------------------------------
// Custody
// ---------------------------------------------------------------------------

#[test]
fn custody_exhaustion_fails_the_observation_closed() {
    let memory = chain_memory(3, 0);
    // What this observation costs is MEASURED, not hard-coded, and then taken
    // away a byte at a time. A literal budget would be a test about whatever
    // the index and link tables happen to reserve first, which is not the
    // property: the property is that one byte short of enough is a refusal
    // naming the ceiling, and that it produces no snapshot.
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

/// The 512 MiB ceiling is a ceiling on WHAT THE OBSERVATION HOLDS, not on the
/// subset of it that happens to be copied bytes.
///
/// The tables that index the copies — the record table, the three address
/// runs, the two link arenas — are the same order of magnitude as the copies
/// and were once outside the accounting entirely. An observation could then
/// report 0.0% of budget while its dependency chains took the machine out.
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

/// SENTINEL. The arena is charged for its CAPACITY, not for the length of the
/// copies in it. `Vec`'s amortised growth doubles, so an arena charged by
/// length can be admitted at the ceiling while holding twice it — which is the
/// whole ceiling defeated by a factor of two.
///
/// A budget one byte under what the arena's capacity needs must therefore be a
/// refusal, even though the LENGTH would have fitted.
#[test]
fn the_arena_is_charged_for_its_capacity_and_not_for_its_length() {
    // Directly on the arena. An observation puts its tables' own growth
    // schedule on top of this one, and the property under test is the arena's
    // alone.
    let mut arena = Arena::with_budget(1024);
    for _ in 0..3 {
        arena.alloc_zeroed(16).expect("well inside the budget");
    }
    assert_eq!(arena.copied_bytes(), 48);
    // THE POINT. Doubling from a capacity of 32 takes 64 to hold 48 bytes of
    // copies. Charging by length would report 48 and leave 16 bytes of real
    // memory outside the ceiling — and at 384 MiB that same doubling is
    // 384 MiB outside it. `>` is what separates the two policies; `>=` would
    // pass under either.
    assert!(
        arena.held() > arena.copied_bytes(),
        "the arena holds {} bytes of capacity for {} bytes of copies, and custody counts \
         the capacity",
        arena.held(),
        arena.copied_bytes(),
    );

    // A request that fits inside capacity already charged is not charged
    // again. Admitting the request length on top of a capacity-based `held`
    // double-counts it, and refuses an allocation for want of room the arena
    // is already holding.
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

    // And the growth is CLAMPED to the room rather than doubling past it. This
    // arena can hold 72 bytes; an unclamped doubling from 64 would ask for 128
    // and take the observation over its own ceiling.
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
    // The ledger is held OUTSIDE the observation, so what the observation
    // charged is still readable after it refused. That is what lets this
    // assert the refusal's `held` exactly rather than against a literal.
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
    // its own would leave this at zero and give the observation two ceilings.
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
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 0 },
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
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 3 },
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
        RecipeOp::DecodePointer { offset: 0, guard: None, next: 2 },
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

    // The ceiling is MAX_RECIPE_DEPTH. A recipe that declares more than the
    // substrate can run is refused before any memory is touched, exactly as it
    // was when the ceiling was t35's measured number.
    let mut deep = base();
    deep.limits.max_depth = MAX_RECIPE_DEPTH + 1;
    assert_eq!(
        malformed(deep),
        MalformedReason::DepthLimitAboveCeiling {
            limit: MAX_RECIPE_DEPTH + 1
        }
    );
    // And the measured depth is INSIDE the ceiling, not equal to it: a recipe
    // walking a container deeper than t35's census chains is legal now, which
    // is the whole reason the two constants are different.
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
                trailer_elements: 0,
                guard: None,
                register: None,
                element: vec![2],
            },
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 3 },
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
fn the_wave_ceiling_covers_the_deepest_container_a_reader_accepts() {
    // t35's measurement is unchanged: this is a fact about the census chains
    // and nothing in this lane re-measured it.
    assert_eq!(MEASURED_MAX_DISCOVERY_DEPTH, 12);
    // The RECIPE ceiling is the deepest chain a legal container can force. See
    // `MAX_RECIPE_DEPTH`: d47's `_Tree` walk is 6 waves of chain plus the
    // tree's height, and its accepted contract is 4,096 nodes, whose red-black
    // height bound `2*log2(n + 1)` is 24.
    assert_eq!(MAX_RECIPE_DEPTH, 30);
    assert_eq!(FORWARD_WAVES, 31);
    assert_eq!(TOTAL_PHASES, 32);
    assert!(MAX_RECIPE_DEPTH >= 6 + 24);
    assert!(MAX_RECIPE_DEPTH > MEASURED_MAX_DISCOVERY_DEPTH);
    // The DEFAULT is still the measured number: raising the ceiling opts
    // nobody in, it only lets a recipe that needs the depth ask for it.
    assert_eq!(Limits::default().max_depth, MEASURED_MAX_DISCOVERY_DEPTH);
}

// ---------------------------------------------------------------------------
// The swept ownership index
// ---------------------------------------------------------------------------
//
// The index is three address-sorted runs walked by a shared `Cursor` instead
// of one ordered map probed per transaction. Everything below is a way that
// substitution could be wrong while the observation-level tests above still
// pass: an entry reported twice, an entry the cursor walked past too early, a
// run boundary that swallows an interval, a fold that loses the ordering the
// next sweep depends on.

/// Owned intervals in a fixed shape, split deliberately across all three index
/// runs: `spans[0..2]` in the body, `spans[2..4]` in the wave's pending run,
/// `spans[4..]` in the current batch's fresh run.
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

/// A batch swept with one cursor must report exactly what the same queries
/// report one at a time.
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

/// SENTINEL. Adjacent intervals share a boundary address, so a sweep that
/// advanced its cursor on `end <= start` instead of `end < start`, or that
/// tested `entry.start < last` instead of `<=`, drops or duplicates exactly
/// one of them. Both mistakes are invisible on non-touching fixtures.
#[test]
fn a_sweep_over_adjacent_intervals_reports_each_one_once() {
    // Five intervals with no holes at all between them.
    let spans: Vec<Span> = (0..5)
        .map(|index| Span::new(0x1000 + index * 0x10, 0x10))
        .collect();
    let owners = three_run_map(&spans);

    let queries: Vec<Span> = vec![
        Span::new(0x1000, 1),    // the first byte of the first interval
        Span::new(0x100F, 2),    // the seam between the first two
        Span::new(0x1010, 0x10), // exactly the second
        Span::new(0x1020, 0x21), // the third, fourth and one byte of the fifth
        Span::new(0x1040, 0x10), // exactly the last
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

    // And the whole run in one query is five owners, in address order, once
    // each.
    let all = owners.overlapping(Span::new(0x1000, 0x50));
    assert_eq!(all.len(), 5);
    let starts: Vec<u64> = all
        .iter()
        .map(|id| owners.record(*id).span().start)
        .collect();
    assert_eq!(starts, vec![0x1000, 0x1010, 0x1020, 0x1030, 0x1040]);

    // SENTINEL WITHIN THE SENTINEL. A query starting on the LAST byte of an
    // interval, reached from a cursor sitting further back, is the only shape
    // that separates the advance's `ends before this query starts` from `ends
    // at or before it`. The queries above never make the cursor gallop onto
    // such an entry, so the off-by-one hides from all of them.
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

/// SENTINEL. An interval starting exactly at the query address is the case
/// that double-counted under the ordered map (see
/// `an_interval_starting_at_the_query_address_is_reported_once`). The swept
/// index reaches it by a different route — cursor position versus run merge —
/// so it is pinned again against the sweep.
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

/// A batch of queries that overlap EACH OTHER — which is what two recipes'
/// requests in one wave look like — must not let the cursor skip an interval
/// an earlier query already consumed.
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

/// An empty batch and a one-interval batch: the degenerate shapes a wave
/// actually takes at the deep end of a chain.
#[test]
fn a_sweep_over_empty_and_single_interval_shapes() {
    let empty = OwnershipMap::new();
    let mut cursor = empty.sweep();
    let mut out = vec![IntervalId(7)];
    empty.sweep_overlapping(&mut cursor, Span::new(0x1000, 0x10), &mut out);
    assert!(out.is_empty(), "no owners, and the buffer is cleared");
    assert_eq!(empty.plan(Span::new(0x1000, 0x10)).holes, vec![Span::new(0x1000, 0x10)]);
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

    // An empty span is not an address range and owns nothing, in either
    // direction: it is not indexed and it overlaps nothing.
    let mut with_empty = OwnershipMap::new();
    with_empty.insert(Span::new(0x1000, 0), 0);
    with_empty.insert(Span::new(0x1000, 0x10), 0);
    assert_eq!(with_empty.overlapping(Span::new(0x1000, 0x10)).len(), 1);
    let mut cursor = with_empty.sweep();
    with_empty.sweep_overlapping(&mut cursor, Span::new(0x1000, 0), &mut out);
    assert!(out.is_empty());
}

/// SENTINEL. The cursor may pass an interval only once no later query can
/// reach it. An interval that spans the hole between two queries is the case
/// that catches an advance keyed on the interval's START.
#[test]
fn a_sweep_does_not_pass_an_interval_a_later_query_still_needs() {
    let mut owners = OwnershipMap::new();
    owners.insert(Span::new(0x1000, 0x1000), 0).unwrap(); // one wide interval
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
    assert_eq!(out.len(), 1, "the gallop did not overshoot the far interval");
}

/// The acquisition plan swept across a batch must split holes exactly as the
/// single-query plan does, including when the batch's own inserts fill them.
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

    // The batch's own inserts are visible to the rest of the batch: this is
    // what lets `acquire` fill a hole and have the next planned copy see it.
    // The batch run starts empty, exactly as the executor arranges, so the
    // batch's ascending holes append to it.
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
    // And a query reaching past the owned run still finds the rest.
    live.sweep_plan(&mut cursor, Span::new(0x1010, 0x40), &mut plan);
    assert_eq!(plan.holes, vec![Span::new(0x1040, 0x10)]);
}

/// SENTINEL. Folding is what keeps the runs sorted, and a sweep against an
/// unsorted run reports the wrong owners. Every fold order must land on the
/// same index.
#[test]
fn folding_the_index_preserves_address_order_and_content() {
    let spans: Vec<Span> = (0..24)
        .map(|index| Span::new(0x1000 + index * 0x20, 0x10))
        .collect();
    // Insert in an order that puts low addresses in late runs.
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
    let starts: Vec<u64> = all.iter().map(|id| owners.record(*id).span().start).collect();
    let mut expected: Vec<u64> = spans.iter().map(|span| span.start).collect();
    expected.sort_unstable();
    assert_eq!(starts, expected, "the fold must leave the index in address order");

    for span in &spans {
        assert!(owners.find(span.start).is_some(), "{span:?} survived the fold");
    }

    // And the frozen table comes out in address order however it was built.
    let (records, _links) = owners.into_frozen().unwrap();
    assert_eq!(records.len(), 24);
    assert!(records.windows(2).all(|pair| pair[0].span().start < pair[1].span().start));
}

/// SENTINEL. A discovery edge already on a parent's chain is not stored
/// twice, and the search for it is bounded so one parent's element array
/// cannot make the attach quadratic. Both halves are pinned: short chains are
/// exact, and a chain past the bound stays correct for the only thing that
/// reads it.
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

    // A DIFFERENT consumer down the same edge is a different edge: that label
    // is what keeps invalidation per consumer.
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

    // A self-edge is not an edge.
    owners.add_edge(parent, parent, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), 3);
}

/// SENTINEL (Codex review finding A). The edge dedup's chain walk is bounded,
/// so a parent with a long chain answers from a spilled set instead. The
/// bound must not become a hole: an edge added FIRST is the DEEPEST one on a
/// chain that prepends, so it is exactly the edge a bounded walk stops short
/// of. Recording it a second time after the chain has outgrown the bound must
/// still be a no-op.
///
/// The executor reaches this with one container: every element's
/// `DecodePointer` reads its field out of the one array interval, so the
/// array is the parent of every element's edge, and `link_pruned` replays a
/// pruned duplicate after all of them.
#[test]
fn an_edge_deeper_than_the_dedup_scan_is_still_not_recorded_twice() {
    let mut owners = OwnershipMap::new();
    let parent = owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    let first = owners.insert(Span::new(0x2000, 0x10), 0x10).unwrap();
    owners.add_edge(parent, first, ConsumerId(1)).unwrap();

    // Bury it under far more than the scan bound.
    let mut buried = Vec::new();
    for index in 0..400u64 {
        let child = owners.insert(Span::new(0x3000 + index * 0x10, 0x10), 0).unwrap();
        owners.add_edge(parent, child, ConsumerId(1)).unwrap();
        buried.push(child);
    }
    let record = *owners.record(parent);
    let before = owners.links().children(&record).count();
    assert_eq!(before, 401);

    // The deepest edge, again.
    owners.add_edge(parent, first, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(
        owners.links().children(&record).count(),
        before,
        "the deepest edge is already recorded, however long the chain got"
    );

    // So is one from the middle, and one from the very top.
    owners.add_edge(parent, buried[7], ConsumerId(1)).unwrap();
    owners.add_edge(parent, buried[399], ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), before);

    // A genuinely new edge past the bound is still recorded, once.
    let fresh = owners.insert(Span::new(0x9000, 0x10), 0).unwrap();
    owners.add_edge(parent, fresh, ConsumerId(1)).unwrap();
    owners.add_edge(parent, fresh, ConsumerId(1)).unwrap();
    let record = *owners.record(parent);
    assert_eq!(owners.links().children(&record).count(), before + 1);

    // And the chain still holds exactly the right edges.
    let mut edges: Vec<IntervalId> = owners
        .links()
        .children(&record)
        .map(|(child, _)| child)
        .collect();
    edges.sort_unstable();
    edges.dedup();
    assert_eq!(edges.len(), before + 1, "no duplicates survived");

    // Spilling is per parent: a parent that never spilled is untouched by one
    // that did.
    let other = owners.insert(Span::new(0xA000, 0x10), 0).unwrap();
    owners.add_edge(other, first, ConsumerId(1)).unwrap();
    owners.add_edge(other, first, ConsumerId(1)).unwrap();
    let record = *owners.record(other);
    assert_eq!(owners.links().children(&record).count(), 1);
}

/// An out-of-order `insert` — which the batch sweep never produces, but which
/// a direct caller can — must still leave a run a sweep can walk.
#[test]
fn an_out_of_order_insert_keeps_the_index_sorted() {
    let mut owners = OwnershipMap::new();
    for start in [0x3000u64, 0x1000, 0x2000, 0x0500] {
        owners.insert(Span::new(start, 0x10), 0).unwrap();
    }
    let all = owners.overlapping(Span::new(0, 0x4000));
    let starts: Vec<u64> = all.iter().map(|id| owners.record(*id).span().start).collect();
    assert_eq!(starts, vec![0x0500, 0x1000, 0x2000, 0x3000]);
    for start in [0x3000u64, 0x1000, 0x2000, 0x0500] {
        assert!(owners.find(start).is_some());
    }
}

/// The whole index at once: a pseudo-random population spread over the three
/// runs, every query answered by a sweep and by a bisecting lookup, and the
/// two must never disagree.
#[test]
fn the_swept_index_agrees_with_the_bisecting_one_over_a_populated_map() {
    // Deterministic, dependency-free: a 64-bit xorshift.
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
        // Disjointness is the map's invariant, so the fixture keeps the
        // intervals inside their own 0x10 slot.
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

/// The index carries an INCLUSIVE last address so that an interval running to
/// the top of the address space is representable without the `u128` end.
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

/// SENTINEL. The index entry stores the DISTANCE to its inclusive last
/// address, so the saturation that `last_addr` performs happens once at insert
/// instead of on every comparison in the sweep. A span whose exclusive end is
/// past `2^64` must still be indexed as ending at `u64::MAX`: storing the
/// unclipped distance would wrap the comparison and make the interval
/// invisible to every query above its start.
#[test]
fn an_index_entry_clips_a_span_that_runs_off_the_top_of_the_address_space() {
    let mut owners = OwnershipMap::new();
    // Exclusive end is `u64::MAX + 5`, so the entry has to clip.
    owners.insert(Span::new(u64::MAX - 3, 8), 0).unwrap();
    assert_eq!(owners.find(u64::MAX).map(|id| id.0), Some(0));
    assert_eq!(owners.find(u64::MAX - 3).map(|id| id.0), Some(0));
    assert_eq!(owners.find(u64::MAX - 4), None);
    assert_eq!(owners.overlapping(Span::new(u64::MAX, 1)).len(), 1);
    let mut cursor = owners.sweep();
    let mut out = Vec::new();
    owners.sweep_overlapping(&mut cursor, Span::new(u64::MAX, 1), &mut out);
    assert_eq!(out.len(), 1, "a clipped entry is still reachable at the top");
}

/// SENTINEL. The dependency attach walks requests in the coalescer's address
/// order rather than in request order. Both orders must reach every request:
/// dropping the ones the coalescer merged away, or visiting only the planned
/// copies, silently narrows what a consumer may read.
#[test]
fn every_request_attaches_however_the_coalescer_ordered_them() {
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 0x400);
    // Windows declared in DESCENDING address order, so request order and
    // address order differ, and two of them merge into one transaction.
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

// ---------------------------------------------------------------------------
// The record model, and the bounds its narrowed fields rest on
// ---------------------------------------------------------------------------

/// SENTINEL. The record table is the largest structure an observation builds
/// and the index is what the sweep walks end to end once per wave, so both
/// widths are a cache-line budget rather than a memory one. Widening either
/// type back — restoring a `usize` offset, an `Option<usize>`, a `bool` that
/// does not fit in the length's spare bits, or a stored `last` alongside
/// `start` — puts the sweep back on half the entries per line.
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

/// SENTINEL. The length shares its word with the two flag bits. An interval as
/// long as custody permits must still report its own length with both flags
/// set — if the length ever reached the flag bits, a witnessed interval would
/// read back longer than it is and the view would hand out bytes past it.
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

    // The spill flag is the other bit, and the edge path is what sets it.
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

/// SENTINEL. `closing_offset` is a `u32` with `u32::MAX` standing for "no
/// closing copy", which is only sound while no real offset can reach it. The
/// custody ceiling is what bounds it; this is the check that fires if the
/// ceiling ever stops doing so.
#[test]
#[should_panic(expected = "cannot be told from")]
fn a_closing_offset_at_the_sentinel_is_refused() {
    let mut owners = OwnershipMap::new();
    let id = owners.insert(Span::new(0x1000, 0x10), 0).unwrap();
    owners.record_mut(id).set_closing_offset(u32::MAX as usize);
}

/// SENTINEL. The same bound on the way in. An offset past what the record can
/// hold would be truncated into some other interval's arena bytes, which is a
/// reader silently getting the wrong object rather than a refusal.
#[test]
#[should_panic(expected = "is past the")]
fn an_opening_offset_past_the_custody_ceiling_is_refused() {
    let mut owners = OwnershipMap::new();
    owners
        .insert(Span::new(0x1000, 0x10), u32::MAX as usize)
        .unwrap();
}

/// SENTINEL. And on the length. An interval longer than the record's thirty
/// length bits would read back truncated, so the view would refuse bytes the
/// observation actually copied.
#[test]
#[should_panic(expected = "it can only exist if custody admitted it")]
fn an_interval_longer_than_the_record_can_hold_is_refused() {
    let mut owners = OwnershipMap::new();
    owners
        .insert(Span::new(0x1000, u64::from(u32::MAX)), 0)
        .unwrap();
}

// ---------------------------------------------------------------------------
// The discovery visited set
// ---------------------------------------------------------------------------

/// SENTINEL. The visited set is two tables split by task kind. The split is
/// only behaviour-preserving because the two key spaces never overlapped: a
/// copy-op key carried a constant zero where an extent key carries its length,
/// and an enqueued extent's length is never zero. If the tables were ever
/// merged back onto one key, a copy op and an extent reaching the same address
/// would prune each other and one of them would never be copied.
#[test]
fn the_visited_sets_two_task_kinds_do_not_prune_each_other() {
    let mut visited = executor::VisitedSet::default();
    let tag = 7u64;
    assert!(visited.insert_op(0x1000, tag));
    assert!(!visited.insert_op(0x1000, tag), "a repeat is not fresh");
    // Same address, same tag, as an extent: a different task, so still fresh.
    assert!(visited.insert_extent(0x1000, 0x40, tag));
    assert!(!visited.insert_extent(0x1000, 0x40, tag));
    // A different decoded count at the same base is a different task. Pruning
    // it would copy a prefix of the array and report `Captured`.
    assert!(visited.insert_extent(0x1000, 0x80, tag));
    assert_eq!(visited.len(), 3);
}

/// Two windows `gap` bytes apart, reached either straight from a root or
/// through one pointer dereference.
///
/// The two shapes produce copy requests that are IDENTICAL in span, in policy
/// and in proof — the windows are the same widths at the same distance and
/// nothing about the surrounding allocation is proven either way. The single
/// difference is the anchor the task carries: a root address in one, the
/// dereferenced pointer value in the other.
fn windowed_report(through_pointer: bool, gap: u64) -> RecipeReport {
    const HOLDER: u64 = BASE;
    const TARGET: u64 = BASE + 0x8000;
    let window_len = 8u32;
    let far = (u64::from(window_len) + gap) as i64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(HOLDER, 16);
    memory.map_zeroed(TARGET, (far as u64 + u64::from(window_len)) as usize);
    memory.write_u64(HOLDER, TARGET);

    let windows = vec![RelWindow::new(0, window_len), RelWindow::new(far, window_len)];
    let recipe = if through_pointer {
        // CopyHeader at the holder, dereference its first field, and take the
        // windows at the VALUE. The window task's anchor is that value.
        Recipe::new(
            RecipeId(1),
            ConsumerId(1),
            vec![
                RecipeOp::CopyHeader {
                    size: 16,
                    next: vec![1],
                },
                RecipeOp::DecodePointer { offset: 0, guard: None, next: 2 },
                RecipeOp::CopyRanges {
                    windows,
                    next: vec![3],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: HOLDER }],
        )
    } else {
        // The same windows at the same address, named as a root.
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
            vec![Root { op: 0, addr: TARGET }],
        )
    };

    let build = observe(&[recipe], &memory, config()).expect("custody budget");
    *build.report_for(RecipeId(1)).expect("the recipe ran")
}

/// SENTINEL. A copy task carries its anchor as a KIND, and the address the
/// anchor rests on is rebuilt from the task's own address at request time.
///
/// That erasure is only sound because the two are the same at every site that
/// builds a task: a root anchors on the address it copies at, a dereferenced
/// pointer anchors on the value it copies at, and a bounded join anchors on the
/// element address it computed. If the rebuild ever names the wrong ADDRESS,
/// two windows under one pointer stop sharing an anchor and the generic tier
/// stops bridging between them. If it ever names the wrong KIND, a root's
/// windows start bridging — which is the proximity merge §4 of the gap-curve
/// verdict refuses, and it would over-copy across an unproven neighbourhood.
///
/// Both directions are asserted here, on window pairs that differ in nothing
/// else. A production change that made `TaskAnchor::at` return a constant
/// address, or that mapped `Root` onto `Pointer`, breaks one arm or the other.
#[test]
fn the_task_anchor_rebuilds_the_address_and_the_kind_it_erased() {
    // `planned_ranges` accumulates over the whole recipe, so the pointer shape
    // carries its holder's `CopyHeader` in every count below: one range for the
    // header, then whatever the two windows coalesce to.
    const HEADER: u32 = 1;

    // A 32 B hole is exactly the generic tier, so it bridges when — and only
    // when — both requests share one pointer anchor.
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

    // The same pair one byte past the tier. This is the control that says the
    // merge above was the TIER and not proximity, and it is also what fails if
    // the rebuilt anchor ever stops depending on the address at all.
    let too_far = windowed_report(true, u64::from(GENERIC_GAP_BYTES) + 1);
    assert_eq!(
        too_far.planned_ranges,
        HEADER + 2,
        "33 B is past the generic tier; the pair must separate again",
    );

    // The same two windows at the same address, at the same distance, anchored
    // on a ROOT instead. No header in this shape — the windows are the whole
    // recipe — and a root address proves nothing about its neighbours, so the
    // pair that merged above must not merge here.
    let unbridged = windowed_report(false, u64::from(GENERIC_GAP_BYTES));
    assert_eq!(unbridged.outcome, RecipeOutcome::Captured);
    assert_eq!(
        unbridged.planned_ranges, 2,
        "a ROOT anchor never satisfies the generic tier, so the identical pair stays two \
         transactions",
    );
}

/// SENTINEL, and the arm the test above cannot reach on its own.
///
/// Every assertion up there involves ONE window task, so a rebuild that ignored
/// the task's address and returned a CONSTANT one would satisfy all of them:
/// the windows of a single task share whatever anchor they are given. What a
/// constant breaks is the boundary BETWEEN tasks — two windows reached through
/// two different pointers would start claiming one anchor, and the generic tier
/// would bridge across two objects nothing tied together.
///
/// Two roots, two dereferences, two single-window copies placed exactly one
/// generic tier apart, all inside one recipe so they plan as one batch.
#[test]
fn two_window_tasks_under_different_pointers_do_not_share_an_anchor() {
    const HOLDER_A: u64 = BASE;
    const HOLDER_B: u64 = BASE + 0x1000;
    const TARGET_A: u64 = BASE + 0x8000;
    // Eight bytes of window, then exactly the generic tier of hole.
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
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 2 },
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
    // Two headers, far apart and each proving only its own 16 bytes, plus two
    // windows that must stay apart. A constant rebuilt anchor merges the
    // windows and this reads 3.
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

/// SENTINEL. The executor's own tables are charged by RECONCILIATION — they
/// grow inside `enqueue` and the decode dispatch, and `charge_own_tables`
/// brings the ledger up to their real capacity once per wave. A RESERVATION
/// cannot use that route unaided: `charged_reserve` charges the growth it
/// takes, and the next reconciliation would see the larger capacity and charge
/// the same bytes a second time.
///
/// `Observation::reserve_own` advances the reconciliation baseline by exactly
/// what it reserved, which keeps the two accountings disjoint. This test walks
/// the seeding path — reserve the root queue, enqueue every root, reconcile —
/// and asserts the ledger charged each byte once.
///
/// The production change that breaks it: drop the `*charged +=` from
/// `reserve_own`. The reservation is then charged by `charged_reserve` and
/// charged AGAIN by the reconciliation that follows it, and custody reports a
/// footprint the observation does not have — which at the ceiling refuses
/// observations that would have fitted.
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
    // charged nothing: every table byte on the ledger is one the executor's own
    // reconciliation is responsible for. Asserted rather than assumed, because
    // it is what makes the equality below exact.
    assert_eq!(
        probe.ledger_table_bytes(),
        probe.own_tables_charged(),
        "before the reservation the two accountings already agree",
    );

    // Measured BEFORE the reservation, so what follows is about the
    // reservation and not about the visited set's preallocation, which
    // `Observation::new` already charged.
    let before_reserve = probe.ledger_table_bytes();
    let capacity_before = probe.wave_capacity(0);

    probe.reserve_root_queue().expect("the default budget fits it");
    let after_reserve = probe.ledger_table_bytes();
    let reserved_capacity = probe.wave_capacity(0);
    // The reservation must have DONE something, or every equality below is a
    // tautology about a no-op. `>= ROOTS` rather than `== ROOTS`: the allocator
    // is allowed to hand back more room than it was asked for, and custody is
    // charged for whatever it handed back.
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

    // THE POINT. The ledger's table charge is the executor's own reconciled
    // footprint — not that footprint plus the reservation a second time.
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

/// SENTINEL, and the third `TaskBody::Op` constructor — the one the two tests
/// above do not reach.
///
/// `JoinBounded` computes an element address by bounded index arithmetic into a
/// registered extent and builds a task there. Its anchor is a POINTER: the
/// address it computed is a discovered object address, not a static one, so it
/// is a legitimate window anchor even though no pointer was dereferenced to
/// reach it. A rebuild that named `Root` for this task would silently stop its
/// windows merging; the root and pointer arms above would both still pass,
/// because neither of them runs a join.
///
/// The windows are placed WELL CLEAR of the table, and both parts of that
/// matter.
///
/// Clear of it by address, because the join's task and the extent's own copy
/// land in the same wave and therefore the same coalescing batch: a window
/// overlapping the element array merges into that copy, and the merged anchor
/// is `Mixed`, which decides everything afterwards without the join's anchor
/// ever being consulted. The first draft of this test made that mistake and
/// passed with the anchor mutated.
///
/// Clear of it by PROOF, because the join's task inherits `Proof::Extent` over
/// the table: a merged footprint inside the table bridges on the proven tier
/// (128 B) rather than the anchor tier. Outside it, only the anchor is left.
#[test]
fn a_bounded_joins_task_anchors_on_the_element_address_it_computed() {
    const TABLE: u64 = BASE + 0x1000;
    const STRIDE: u64 = 16;
    const ELEMENTS: u32 = 4;
    /// Element 2 — where the join lands, and what the windows are relative to.
    const KEY: u32 = 2;
    /// Far enough from the element that neither window can touch the 64-byte
    /// table, so nothing in this recipe merges except the two windows.
    const NEAR: i64 = 0x200;
    /// Eight bytes of window, then exactly the generic tier of hole.
    const FAR: i64 = NEAR + 8 + GENERIC_GAP_BYTES as i64;

    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 24);
    memory.write_u32(BASE, ELEMENTS);
    memory.write_u64(BASE + 8, TABLE);
    memory.write_u32(BASE + 16, KEY);
    // Room for the table and for both windows well past the end of it.
    memory.map_zeroed(TABLE, 0x400);
    // A distinct value under each window of the element the join is supposed
    // to land on, and a different one under the neighbouring elements'
    // windows. Without these the whole region is zeroes, and a join that
    // computed the WRONG element address would copy two equally mergeable
    // zero-filled windows: the range count and the byte count would be
    // identical and the test would prove only that some address was used.
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

    // The fixture's own preconditions. Each of these is a way the test could
    // pass without ever consulting the join's anchor, so they are checked
    // rather than intended.
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
    // The header, the element array, and the join's two windows merged into
    // one. A `Root` anchor on the join's task splits the last of those, and
    // this reads 4.
    assert_eq!(
        report.planned_ranges, 3,
        "the join's windows share the pointer anchor of the element address the join \
         computed, so the generic tier bridges the 32 B hole between them",
    );
    // And the merge is a real over-copy of the bridged hole, not a bookkeeping
    // artefact: 24 B header, 64 B element array, and 48 B spanning both
    // windows and the 32 B between them.
    assert_eq!(
        build.snapshot.custody_bytes(),
        24 + 64 + 48,
        "the bridged hole was copied, which is what a merge at the generic tier means",
    );

    // AND THE ADDRESS. Everything above is a count, and counts are the same
    // whichever element the join landed on: the neighbours' windows are just as
    // mergeable and just as wide. This reads the bytes back and pins the
    // element the join was supposed to compute — key 2 at stride 16 — so a
    // change to that arithmetic is a different value here rather than an
    // identical tally.
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

/// SENTINEL. The root-queue reservation is sized by what `enqueue` can ACCEPT,
/// never by what a recipe declared.
///
/// A root count arrives from the recipe with no limit applied to it:
/// `max_copy_tasks` is checked per task during discovery, so a recipe may
/// declare a hundred thousand roots against a task limit of sixteen and reach
/// exactly sixteen of them. Reserving the declared count would take — and
/// charge — memory the observation can never fill, and at the ceiling that
/// converts an observation which used to run into a global custody refusal.
/// Custody failure is not per-recipe: it aborts every recipe in the
/// observation, including the well-formed ones.
///
/// The production change that breaks this: size `reserve_root_queue` from
/// `recipe.roots.len()` instead of `min(roots, max_copy_tasks)`.
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
    probe.reserve_root_queue().expect("the default budget fits it");
    // Tight against the limit rather than merely "much less than the roots".
    // A loose bound — anything under a thousand, say — is passed by a rule that
    // reserves thirty-two times what the recipe can enqueue, which is the same
    // defect in a smaller coat. The allocator may round the request up, so this
    // allows a factor of two over the sixteen tasks the limit admits and
    // nothing like the six thousand the declared roots would take.
    assert!(
        probe.wave_capacity(0) <= 2 * TASKS as usize,
        "the queue was sized for the {TASKS} tasks the limit admits, not the {ROOTS} roots \
         the recipe declared; it has room for {}",
        probe.wave_capacity(0),
    );

    // And the recipe still behaves exactly as it did: the task limit refuses
    // it, per-recipe, rather than custody aborting the observation.
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

/// SENTINEL. A recipe that fails validation must not size the reservation.
///
/// A malformed recipe enqueues nothing: `run` passes over it and `enqueue`
/// refuses it again. Counting its declared roots therefore charges custody for
/// queue slots that cannot exist — and custody failure is the one GLOBAL
/// refusal, so at the ceiling a malformed recipe would abort the well-formed
/// recipes sharing its observation. Per-recipe refusals exist precisely so one
/// bad recipe cannot do that.
///
/// The production change that breaks it: reserve before the validation pass, or
/// drop the `refusal.is_none()` filter in `reserve_root_queue`.
#[test]
fn a_recipe_that_fails_validation_does_not_size_the_root_reservation() {
    use super::executor::DispatchProbe;

    const ROOTS: usize = 50_000;
    let mut memory = SyntheticMemory::new();
    memory.map_zeroed(BASE, 64);

    // Op 0 is a copy op whose `next` names op 0 — a copy op in a decode slot,
    // which `validate` rejects. Fifty thousand roots point at it.
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

    // The probe arm: the reservation itself, with the validation pass in front
    // of it. Tight against what the SOUND recipe can enqueue — one root, so the
    // growth schedule's floor — because a loose bound is passed by a rule that
    // still counts the malformed recipe, just less of it.
    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");
    probe.validate_recipes();
    probe.reserve_root_queue().expect("the default budget fits it");
    assert!(
        probe.wave_capacity(0) <= 32,
        "only the sound recipe's single root may be reserved for, not the malformed \
         recipe's {ROOTS}; the queue has room for {}",
        probe.wave_capacity(0),
    );

    // The `run` arm. The probe drives the validation pass itself, so it cannot
    // see `run` reordering the two; this measures what a whole observation
    // HOLDS, which the reservation's charge is part of and which no amount of
    // reordering hides.
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

    // And the observation still behaves as it always did: one recipe refuses
    // for being malformed, the other captures.
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

/// SENTINEL. The per-wave reconciliation must see the wave that is IN FLIGHT.
///
/// `run` takes a wave's tasks out of `self.waves` before running it, so for the
/// whole of that wave the queue the reconciliation measures is empty while the
/// memory is still held in a local. The baseline it compares against still
/// contains that capacity, so the `bytes <= charged` exit fires and growth
/// elsewhere in the wave — the next wave's queue filling up as the decode
/// dispatch discovers — is charged to nobody up to the size of the invisible
/// queue. The ledger then sits BELOW what the observation holds, which is the
/// one direction custody must never take: the ceiling stops being a ceiling.
///
/// `run` therefore hands the held queue's size to `charge_own_tables`. This
/// test reproduces the exact sequence — seed, reconcile, take the wave, copy,
/// decode into the next wave, reconcile again — and asserts the ledger covers
/// the footprint including the wave in hand.
///
/// The production change that breaks it: pass `0` instead of
/// `vec_bytes(&tasks)` at the end of `run`'s wave loop.
#[test]
fn the_reconciliation_counts_the_wave_that_is_in_flight() {
    use super::executor::DispatchProbe;

    // A chain, so the decode dispatch discovers into the next wave and that
    // queue grows through the RECONCILED path rather than being charged
    // eagerly the way a reservation is.
    let memory = chain_memory(64, 0);
    let recipes = [chain_recipe(1, 1)];
    let mut probe =
        DispatchProbe::new(&recipes, &memory, config()).expect("the default budget fits it");
    probe.reserve_root_queue().expect("custody");
    probe.seed_roots();
    probe.charge_tables().expect("custody");

    // The wave leaves `self.waves` and stays alive here, exactly as it does
    // inside `run`.
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

    // Through the same method `run`'s wave loop calls, so this exercises the
    // production path rather than arithmetic the test invented.
    //
    // WHAT THIS TEST STILL DOES NOT COVER, stated rather than implied: it
    // cannot catch the wave loop calling `charge_own_tables(0)` instead, since
    // it drives the reconciliation itself. That call site is guarded by
    // construction rather than by this test — `charge_own_tables_holding` takes
    // the queue, so there is no `in_flight` number at that site to get wrong.
    probe.charge_tables_holding(&tasks).expect("custody");

    // THE POINT. What the executor has been charged for covers everything it
    // holds, including the queue that is not in `self.waves` at this instant.
    //
    // Two things about how this is written, and both were mistakes first.
    //
    // The floor is built as `own_tables_bytes(0) + in_flight` — the production
    // sum of everything the observation still owns, plus the held queue added
    // HERE — rather than by asking `own_tables_bytes(in_flight)`. The latter
    // could not fail: a reconciliation that ignored `in_flight` would have made
    // the expectation ignore it too, and the assertion would have compared a
    // mutated number against itself.
    //
    // And it reads the EXECUTOR's reconciled total rather than the ledger's.
    // The ledger also carries the ownership map's records and link arenas,
    // which this fixture grows by far more than one wave queue is worth; a
    // ledger-level comparison is satisfied by the index's charges no matter
    // what the executor did with its own.
    let floor = probe.own_tables_bytes(0) + in_flight;
    assert!(
        probe.own_tables_charged() >= floor,
        "custody must never sit below the footprint: the executor is charged for {} against \
         {floor} held ({} in tables it still owns, {in_flight} in the wave in hand)",
        probe.own_tables_charged(),
        probe.own_tables_bytes(0),
    );
    // And the ledger is at least that, since the executor's share is part of
    // it.
    assert!(probe.ledger_table_bytes() >= probe.own_tables_charged());
}

/// The reservation covers DEPTH ZERO and nothing else, and every wave under it
/// still grows the way it always did.
///
/// A behaviour-preservation test rather than a sentinel, and labelled as one:
/// it passes with the reservation removed entirely, which is exactly what a
/// test of "the optimisation changed no result" should do. What it watches for
/// is the opposite mistake — a reservation that became a BOUND, or a sizing
/// rule that leaked into the discovered waves. Six chained nodes are reached
/// from two entry points, so waves one through five run on queues nothing sized
/// in advance, and the second entry point lands inside the chain the first one
/// already walked, so deduplication has to prune it rather than copy it twice.
#[test]
fn discovery_past_depth_zero_runs_on_queues_that_were_not_reserved() {
    let memory = chain_memory(6, 0);
    // Two roots into the same chain: the second is pruned as a duplicate of
    // what the first discovered, so the queue's final occupancy and the
    // reservation's size are not the same number.
    let recipe = Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: 16,
                next: vec![1, 2],
            },
            RecipeOp::DecodePointer { offset: 0, guard: None, next: 0 },
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

/// SENTINEL. The key packs the recipe index and the op into one word. The key
/// this replaced cast the recipe index to `u32`, so two recipes `2^32` apart
/// shared every visited entry and pruned each other's copy tasks. The packing
/// must keep them distinct, and `MAX_RECIPES` is what bounds it.
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
    // And the op is distinct from the recipe: op 1 of recipe 0 is not recipe 1
    // at op 0.
    assert_ne!(executor::visit_tag(0, 1), executor::visit_tag(1, 0));
}


/// RULING: a `Gap` request wins whatever order the requests arrive in.
///
/// `CloseAction::Record` lets a converted reader take the closing image and
/// render its own verdict instead of being gapped by the substrate's. It is a
/// NARROWING of what the substrate certifies, so it is per (op, consumer) and
/// the strictest request must win — and "strictest wins" is only expressible if
/// every request is recorded, in both orders.
///
/// THE DEFECT THIS PINS (Codex 019feb7d, finding 5). A first version skipped
/// storing a `Gap` request on an interval nobody had asked about, on the grounds
/// that `Gap` is the answer `close_gaps` already gives for an interval with no
/// asks. That is true until one arrives: a consumer whose `Gap` request reached
/// a coalesced interval FIRST and whose `Record` request reached it second was
/// left with only the `Record`, and opted out of a gap it had asked for.
///
/// MUTATION: put the `if gap_on_change && !self.closes.contains_key(&interval)
/// { return; }` short-circuit back into `Annotations::ask_close` and the first
/// arm below fails.
#[test]
fn a_gap_request_wins_whatever_order_the_requests_arrive_in() {
    use super::annot::Annotations;
    use super::{ConsumerId, IntervalId};

    let interval = IntervalId(7);
    let consumer = ConsumerId(81);

    // GAP FIRST, then RECORD.
    let mut gap_first = Annotations::new();
    gap_first.arm();
    gap_first.ask_close(interval, consumer, true);
    gap_first.ask_close(interval, consumer, false);
    assert!(
        gap_first.close_gaps(interval, consumer),
        "a consumer that asked to be gapped must be gapped, whatever it asked \
         for afterwards",
    );

    // RECORD FIRST, then GAP. The same two requests, and the same answer.
    let mut record_first = Annotations::new();
    record_first.arm();
    record_first.ask_close(interval, consumer, false);
    record_first.ask_close(interval, consumer, true);
    assert!(
        record_first.close_gaps(interval, consumer),
        "the order requests arrive in must not decide the policy",
    );

    // RECORD ONLY: the opt-out this whole mechanism exists for.
    let mut record_only = Annotations::new();
    record_only.arm();
    record_only.ask_close(interval, consumer, false);
    assert!(
        !record_only.close_gaps(interval, consumer),
        "a consumer whose every request said `Record` renders its own verdict",
    );

    // AND A NEIGHBOUR IS UNTOUCHED. One consumer's opt-out may not reach
    // another's hour, which is the whole reason the policy is per request.
    let neighbour = ConsumerId(47);
    assert!(
        record_only.close_gaps(interval, neighbour),
        "a consumer that never asked takes the substrate's verdict",
    );

    // AND AN INTERVAL NOBODY ASKED ABOUT IS GAPPED, which is the default the
    // opt-out is measured against.
    assert!(record_only.close_gaps(IntervalId(9), consumer));
}
