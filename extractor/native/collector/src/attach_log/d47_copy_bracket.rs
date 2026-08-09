//! d47's prewarm as a COPY PLAN — stage S4 of the capture floor.
//!
//! # What this is
//!
//! The owner-adopted floor plan says a bracket contains bulk memory copies and
//! nothing else: every classification, identity decode, string read and
//! admission decision runs AFTER it, on the hour-labelled copy. This module is
//! the copy half for domain 47 (characters / leaders / cabinet), expressed as a
//! [`crate::snapshot_recipes`] recipe.
//!
//! The recipe is the same walk `prewarm_compact_characters_leaders_cabinet_roots`
//! performs, with every DECISION removed:
//!
//! | live prewarm | copy plan |
//! |---|---|
//! | read the manager's two vectors, validate size/capacity, gate every entry with `is_map_control_heap_pointer` | `DecodeExtent` over each vector; the gate moves post-bracket |
//! | read each `CCharacter` body | `CopyHeader` at [`CHARACTER_BODY_SPAN`] |
//! | walk `+0x98` / `+0xB0`, testing `_Isnil`, the node count, duplicate nodes and the closing header | copy the sentinel and every node reachable through `_Left`/`_Parent`/`_Right`; every one of those tests moves post-bracket and runs over the copies |
//! | double-read each `CCountryLeader` / `CAdvisor`, chase its back-pointer, decode its strings | copy the role body, chase the back-pointer into the SAME character op (deduplicated), copy the string headers and their heap payloads |
//! | `read_msvc_string` on six fields per character | a guarded, NUL-trailing byte extent per field |
//!
//! # The three things that make it a superset and not a re-implementation
//!
//! 1. **It copies more than the live path reads.** The live walk stops at
//!    `_Mysize` nodes and refuses beyond; the recipe follows links until the
//!    tree is exhausted. That is deliberate: a bracket cannot decide, so it
//!    takes everything the decisions might need and the decisions are made
//!    afterwards on what it took. `visited != size`, the duplicate-node check
//!    and the closing-header comparison all still run — over the copy.
//!
//! 2. **It cannot run away.** Nothing here is bounded by a check the recipe
//!    makes; it is bounded by the substrate's limits, which refuse rather than
//!    truncate: `max_depth` ([`D47_MAX_DEPTH`]), `max_copy_tasks`, `max_ranges`,
//!    `max_bytes`, `max_single_range_bytes`, and the discovery visited set,
//!    which prunes a link that returns to an object already expanded. A cycle
//!    therefore ends in a pruned duplicate or in a named refusal, never in a
//!    short result reported as success.
//!
//! 3. **It never dereferences a union's wrong arm.** Two guards do that work
//!    and both are the substrate's, not this module's: `_Isnil == 0` before a
//!    tree node's value is chased, and `_Myres >= 16` before a `std::string`'s
//!    first eight bytes are treated as a pointer. Their absence is not a
//!    slower plan, it is a refused one — see the substrate's own tests.
//!
//! # What is NOT here, and it is one thing
//!
//! The post-bracket DECODE. d47's decode side (`domain47_probe_advisor`,
//! `read_msvc_string_with_snapshot`, `ObjectSnapshot::read`, `read_u64`) is
//! written against free functions that read the LIVE process, not against an
//! injected memory, so it cannot yet be pointed at a [`SnapshotView`].
//! [`Domain47ViewMemory`] is the adapter that will point it there — it
//! implements the same `LaneMemory` the lane's readers already take — and the
//! remaining work is threading that trait through the collector-side readers
//! that currently call the free functions. That is a migration across a shared
//! read layer, not a d47 change, and it is named in this lane's report rather
//! than started here.

use characters_leaders_cabinet_live_lane as domain47;

use crate::snapshot_recipes::executor::{ExtentGuard, Recipe, RecipeOp, Root};
use crate::snapshot_recipes::view::{SnapshotView, ViewRefusal};
use crate::snapshot_recipes::{
    ConsumerId, Limits, MemoryRefusal, RecipeId, SnapshotMemory, Span,
};

/// Bytes of a `CCharacter` the plan copies.
///
/// WIDER than `CCHARACTER_RAW_FIELDS_SPAN` (which ends at the unit-leader
/// pointer at `+0xA8`) because the advisor map's header lives at `+0xB0`/`+0xB8`
/// and the walk reads both. It is the same span
/// `domain47_shadow_walk_headers` already reaches — `+0x98` through `+0xC0` —
/// folded into the body copy, which is what makes the map headers cost no
/// transaction of their own.
pub const CHARACTER_BODY_SPAN: u32 = 0xC0;

/// Bytes of a `CCountryLeader` the plan copies: through its character pointer
/// at `+0x08`, exactly as `CCOUNTRY_LEADER_RAW_FIELDS_SPAN` says.
pub const COUNTRY_LEADER_BODY_SPAN: u32 = 0x10;

/// Bytes of a `CAdvisor` the plan copies: through the END of the slot string's
/// header at `+0x80`, because `domain47_probe_advisor` reads the character
/// pointer at `+0x10`, the idea token at `+0x30` and the slot at `+0x80`.
pub const ADVISOR_BODY_SPAN: u32 =
    domain47::extractor::CADVISOR_SLOT_OFF as u32 + MSVC_STRING_HEADER_LEN;

/// An MSVC `std::basic_string` object: `{ union { char[16]; char* }, _Mysize,
/// _Myres }`.
pub const MSVC_STRING_HEADER_LEN: u32 = 0x20;
const MSVC_STRING_SIZE_OFF: u32 = 0x10;
const MSVC_STRING_CAPACITY_OFF: u32 = 0x18;
/// At or above this capacity the first eight bytes are a heap pointer; below
/// it they are the characters themselves. The one number the string guard is.
const MSVC_STRING_HEAP_CAPACITY: u64 = 16;

/// `_Tree` node layout, shared by both maps.
const TREE_LEFT_OFF: u32 = 0x00;
const TREE_PARENT_OFF: u32 = 0x08;
const TREE_RIGHT_OFF: u32 = 0x10;
const TREE_ISNIL_OFF: u32 = 0x19;

/// `CCharacter+0x98`: `std::map<int ideology, CCountryLeader*>`, node 0x30 B.
const LEADER_MAP_HEAD_OFF: u32 = 0x98;
const LEADER_MAP_SIZE_OFF: u32 = 0xA0;
const LEADER_NODE_SPAN: u32 = 0x30;
const LEADER_NODE_VALUE_OFF: u32 = 0x28;

/// `CCharacter+0xB0`: `std::map<std::string, CAdvisor*>`, node 0x50 B.
const ADVISOR_MAP_HEAD_OFF: u32 = 0xB0;
const ADVISOR_MAP_SIZE_OFF: u32 = 0xB8;
const ADVISOR_NODE_SPAN: u32 = 0x50;
const ADVISOR_NODE_VALUE_OFF: u32 = 0x48;

/// The deepest wave this plan can need.
///
/// character body (0 when character-rooted) -> map sentinel (+1) -> tree root
/// (+2) -> the tree's height `h` -> role body -> its heap string payload. With
/// the manager prefix every number gains two. The substrate's ceiling
/// (`MAX_RECIPE_DEPTH`) is set from the same arithmetic and states it.
pub const D47_MAX_DEPTH: u32 = crate::snapshot_recipes::MAX_RECIPE_DEPTH;

/// Widest single copy this plan ever asks for. A `std::string` payload is
/// bounded by the same `DOMAIN47_CCHARACTER_STRING_MAX_LEN` the decoder
/// refuses past, so nothing here can ask for a megabyte because a torn
/// capacity word said so.
pub const D47_MAX_STRING_BYTES: u64 = super::DOMAIN47_CCHARACTER_STRING_MAX_LEN as u64 + 1;

/// The consumer every d47 post-bracket reader takes its view as.
pub const D47_CONSUMER: ConsumerId = ConsumerId(47);
pub const D47_RECIPE: RecipeId = RecipeId(47);

/// Where a plan starts.
#[derive(Clone, Debug)]
pub enum Domain47PlanRoots {
    /// PRODUCTION. The `CCharacterManager`, whose two `{data, capacity, size}`
    /// vectors the plan decodes in-bracket. The roster enumeration is then
    /// inside the bracket, which is what the hour-attribution rule requires: a
    /// roster read before the bracket would be last hour's roster.
    Manager {
        addr: u64,
        historical_vector_off: u32,
        dynamic_vector_off: u32,
        max_characters: u32,
    },
    /// A roster already in hand. This is the FIXTURE form — the staged worlds
    /// carry characters without a manager object — and it is also what a caller
    /// with an externally enumerated roster would use. It is not a production
    /// shape: see the `Manager` variant.
    Characters(Vec<u64>),
}

/// Vector header layout shared by the manager's two rosters:
/// `{ data u64 @+0x00, capacity u32 @+0x08, size u32 @+0x0C }`.
const VECTOR_DATA_OFF: u32 = 0x00;
const VECTOR_SIZE_OFF: u32 = 0x0C;

/// Op indices, named. A recipe is a flat list and the branch targets are
/// indices into it, so every index is written once here and nowhere else.
mod op {
    pub const CHARACTER: u16 = 0;
    pub const CHARACTER_FREEZE: u16 = 1;
    pub const LEADER_MAP: u16 = 2;
    pub const ADVISOR_MAP: u16 = 3;
    pub const NAME_STRING: u16 = 4;
    pub const TOKEN_STRING: u16 = 5;
    pub const LEADER_NODE: u16 = 6;
    pub const LEADER_NODE_LEFT: u16 = 7;
    pub const LEADER_NODE_PARENT: u16 = 8;
    pub const LEADER_NODE_RIGHT: u16 = 9;
    pub const LEADER_NODE_VALUE: u16 = 10;
    pub const COUNTRY_LEADER: u16 = 11;
    pub const COUNTRY_LEADER_CHARACTER: u16 = 12;
    pub const ADVISOR_NODE: u16 = 13;
    pub const ADVISOR_NODE_LEFT: u16 = 14;
    pub const ADVISOR_NODE_PARENT: u16 = 15;
    pub const ADVISOR_NODE_RIGHT: u16 = 16;
    pub const ADVISOR_NODE_VALUE: u16 = 17;
    pub const ADVISOR: u16 = 18;
    pub const ADVISOR_CHARACTER: u16 = 19;
    pub const ADVISOR_TOKEN_STRING: u16 = 20;
    pub const ADVISOR_SLOT_STRING: u16 = 21;
    pub const STRING_PAYLOAD_FREEZE: u16 = 22;
    /// Only present on a manager-rooted plan, appended after the fixed block.
    pub const MANAGER: u16 = 23;
    pub const MANAGER_HISTORICAL: u16 = 24;
    pub const MANAGER_DYNAMIC: u16 = 25;
    pub const MANAGER_ENTRY: u16 = 26;
}

/// One MSVC `std::string` field, as a guarded NUL-trailing byte extent.
///
/// The guard is the capacity word; below [`MSVC_STRING_HEAP_CAPACITY`] the
/// characters are inside the header the caller already copied and the branch
/// ends without a dereference. The trailer is the NUL the decoder tests for.
/// `element` is empty: the payload is bytes, and running a decode op per byte
/// is the cost the terminal form exists to avoid.
fn string_extent(header_off: u32) -> RecipeOp {
    RecipeOp::DecodeExtent {
        count_offset: header_off + MSVC_STRING_SIZE_OFF,
        count_width: 8,
        ptr_offset: header_off,
        stride: 1,
        // The decoder refuses past `DOMAIN47_CCHARACTER_STRING_MAX_LEN`, so a
        // longer container is a refusal here rather than a copy nothing reads.
        max_count: D47_MAX_STRING_BYTES as u32,
        max_bytes: D47_MAX_STRING_BYTES,
        trailer_elements: 1,
        guard: Some(ExtentGuard::at_least(
            header_off + MSVC_STRING_CAPACITY_OFF,
            8,
            MSVC_STRING_HEAP_CAPACITY,
        )),
        register: None,
        element: Vec::new(),
    }
}

/// One `_Tree` axis: the four ops that copy a sentinel, every node reachable
/// from it, and each real node's value object.
///
/// `_Left`, `_Parent` and `_Right` are chased UNGUARDED because every one of
/// them is a valid link on every node including the sentinel — MSVC points nil
/// links at the head rather than at null, so there is no arm to get wrong. The
/// VALUE is guarded on `_Isnil`, because the head node's value slot is
/// allocated and never constructed.
fn tree_ops(node_span: u32, value_off: u32, node_op: u16, value_target: u16) -> [RecipeOp; 5] {
    [
        RecipeOp::CopyHeader {
            size: node_span,
            next: vec![node_op + 1, node_op + 2, node_op + 3, node_op + 4],
        },
        RecipeOp::DecodePointer {
            offset: TREE_LEFT_OFF,
            guard: None,
            next: node_op,
        },
        RecipeOp::DecodePointer {
            offset: TREE_PARENT_OFF,
            guard: None,
            next: node_op,
        },
        RecipeOp::DecodePointer {
            offset: TREE_RIGHT_OFF,
            guard: None,
            next: node_op,
        },
        RecipeOp::DecodePointer {
            offset: value_off,
            guard: Some(ExtentGuard::exactly(TREE_ISNIL_OFF, 1, 0)),
            next: value_target,
        },
    ]
}

/// Build d47's copy plan.
pub fn domain47_copy_plan(roots: &Domain47PlanRoots) -> Recipe {
    let mut ops = vec![
        // 0 CHARACTER
        RecipeOp::CopyHeader {
            size: CHARACTER_BODY_SPAN,
            next: vec![
                op::CHARACTER_FREEZE,
                op::LEADER_MAP,
                op::ADVISOR_MAP,
                op::NAME_STRING,
                op::TOKEN_STRING,
            ],
        },
        // 1 CHARACTER_FREEZE — the body itself is an end state: a character
        // with neither role and two short strings is a complete branch.
        RecipeOp::Freeze,
        // 2 LEADER_MAP, 3 ADVISOR_MAP.
        //
        // GUARDED ON `_Mysize`, and this is the single largest thing in the
        // plan. An empty MSVC `_Tree` has `_Myhead->_Parent == _Myhead`, so
        // there is nothing behind the sentinel — and most characters hold
        // neither role, which is why `domain47_shadow_walk_map`'s `if size == 0
        // { return true }` is most of that instrument's saving. Without the
        // same test here the plan copies two sentinels for every character on
        // the roster: at t32's width that is 26,806 copy transactions, about
        // half of everything the observation does, for containers the live walk
        // never touches.
        //
        // `_Mysize` is at `+0xA0` / `+0xB8`, inside the body copy above, so the
        // guard costs no fetch.
        //
        // THE TORN-ZERO DIRECTION IS A DIVERGENCE, and it is not caused by this
        // guard (Codex 019fe3eb, finding 1). A size word that tears to zero over
        // a non-empty map ends the branch, and the post-bracket walk then reads
        // the SAME torn zero out of the copy and concludes "empty map, observed
        // clean" — publishing the family a role short. Today's walk reads the
        // same zero and returns the same true; what catches it is the CLOSING
        // header re-read in `domain47_shadow_walk_character`, which refuses the
        // axis when its own `_Myhead`/`_Mysize` pair moved.
        //
        // Removing this guard does NOT close it. It would copy the sentinel, so
        // the DATA for a `_Myhead->_Parent != _Myhead` test would be present —
        // but that test does not exist today either (the live walk deliberately
        // never reads the sentinel: "the sentinel need not be read, and that is
        // most of this instrument's saving"), so adding it would be its own
        // behaviour change. The 26,806 transactions buy nothing on their own.
        //
        // What actually closes it is a closing image the CONSUMER may compare
        // FIELD BY FIELD. `WitnessClose` is the substrate's only closing
        // mechanism and it compares every byte of each interval and gaps the
        // consumer before issuing a view — which is wrong here twice over:
        // `CCHARACTER_UNIT_LEADER_PTR_OFF` at `+0xA8` sits between the two map
        // headers, so a byte-wise comparison reads a legal unit-leader
        // assignment as a role-container tear, and a whole-consumer gap is far
        // coarser than the per-axis refusal today produces. That is lane FX's
        // finding 7 and it is an owner question, not a lane's to take.
        //
        // The other direction IS equivalent: a size that tears NONZERO over an
        // empty map copies the sentinel, and the post-bracket `visited != size`
        // check refuses that axis exactly as it does today.
        RecipeOp::DecodePointer {
            offset: LEADER_MAP_HEAD_OFF,
            guard: Some(ExtentGuard::at_least(LEADER_MAP_SIZE_OFF, 8, 1)),
            next: op::LEADER_NODE,
        },
        RecipeOp::DecodePointer {
            offset: ADVISOR_MAP_HEAD_OFF,
            guard: Some(ExtentGuard::at_least(ADVISOR_MAP_SIZE_OFF, 8, 1)),
            next: op::ADVISOR_NODE,
        },
        // 4 NAME_STRING
        string_extent(domain47::extractor::CCHARACTER_NAME_OFF as u32),
        // 5 TOKEN_STRING
        string_extent(domain47::extractor::CCHARACTER_TOKEN_OFF as u32),
    ];

    // 6..=10 the leader axis
    ops.extend(tree_ops(
        LEADER_NODE_SPAN,
        LEADER_NODE_VALUE_OFF,
        op::LEADER_NODE,
        op::COUNTRY_LEADER,
    ));
    // 11 COUNTRY_LEADER
    ops.push(RecipeOp::CopyHeader {
        size: COUNTRY_LEADER_BODY_SPAN,
        next: vec![op::COUNTRY_LEADER_CHARACTER],
    });
    // 12 COUNTRY_LEADER_CHARACTER — back into the character op. Deduplicated:
    // the body was already expanded under `(recipe, CHARACTER, addr)`, so this
    // attaches the dependency edge and copies nothing. A role whose owner is
    // NOT on the roster is copied here and only here, which is exactly the
    // population the live path reaches through the same back-pointer.
    ops.push(RecipeOp::DecodePointer {
        offset: domain47::extractor::CCOUNTRY_LEADER_CHARACTER_PTR_OFF as u32,
        guard: None,
        next: op::CHARACTER,
    });

    // 13..=17 the advisor axis
    ops.extend(tree_ops(
        ADVISOR_NODE_SPAN,
        ADVISOR_NODE_VALUE_OFF,
        op::ADVISOR_NODE,
        op::ADVISOR,
    ));
    // 18 ADVISOR
    ops.push(RecipeOp::CopyHeader {
        size: ADVISOR_BODY_SPAN,
        next: vec![
            op::ADVISOR_CHARACTER,
            op::ADVISOR_TOKEN_STRING,
            op::ADVISOR_SLOT_STRING,
        ],
    });
    // 19 ADVISOR_CHARACTER
    ops.push(RecipeOp::DecodePointer {
        offset: domain47::extractor::CADVISOR_CHARACTER_PTR_OFF as u32,
        guard: None,
        next: op::CHARACTER,
    });
    // 20 ADVISOR_TOKEN_STRING
    ops.push(string_extent(
        domain47::extractor::CADVISOR_IDEA_TOKEN_OFF as u32,
    ));
    // 21 ADVISOR_SLOT_STRING
    ops.push(string_extent(domain47::extractor::CADVISOR_SLOT_OFF as u32));
    // 22 STRING_PAYLOAD_FREEZE — unreferenced by the fixed block (the string
    // extents are terminal), and kept as the branch end a manager-rooted plan's
    // entry op needs. See `MANAGER_ENTRY`.
    ops.push(RecipeOp::Freeze);

    let entry_roots = match roots {
        Domain47PlanRoots::Characters(addrs) => addrs
            .iter()
            .map(|addr| Root {
                op: op::CHARACTER,
                addr: *addr,
            })
            .collect(),
        Domain47PlanRoots::Manager {
            addr,
            historical_vector_off,
            dynamic_vector_off,
            max_characters,
        } => {
            // 23 MANAGER
            ops.push(RecipeOp::CopyHeader {
                // Through the end of the later vector's header.
                size: (*dynamic_vector_off).max(*historical_vector_off) + 0x10,
                next: vec![op::MANAGER_HISTORICAL, op::MANAGER_DYNAMIC],
            });
            for vector_off in [*historical_vector_off, *dynamic_vector_off] {
                ops.push(RecipeOp::DecodeExtent {
                    count_offset: vector_off + VECTOR_SIZE_OFF,
                    count_width: 4,
                    ptr_offset: vector_off + VECTOR_DATA_OFF,
                    stride: 8,
                    max_count: *max_characters,
                    max_bytes: u64::from(*max_characters) * 8,
                    trailer_elements: 0,
                    guard: None,
                    register: None,
                    element: vec![op::MANAGER_ENTRY],
                });
            }
            // 26 MANAGER_ENTRY — one per vector slot.
            ops.push(RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: op::CHARACTER,
            });
            vec![Root {
                op: op::MANAGER,
                addr: *addr,
            }]
        }
    };

    let mut recipe = Recipe::new(D47_RECIPE, D47_CONSUMER, ops, entry_roots);
    recipe.limits = Limits {
        max_depth: D47_MAX_DEPTH,
        ..Limits::default()
    };
    recipe
}

/// The live process, as the substrate's bulk-copy primitive.
///
/// One guarded, all-or-nothing range copy per call — the same
/// `range_confirmed_good` gate and the same `copy_selected` every other reader
/// in this file goes through. Nothing else about the process is exposed: the
/// substrate can move bytes through this and it cannot compute over them.
pub struct Domain47SnapshotMemory;

impl SnapshotMemory for Domain47SnapshotMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        let span = Span::new(addr, out.len() as u64);
        let Ok(start) = usize::try_from(addr) else {
            return Err(MemoryRefusal { span });
        };
        if super::read_into(start, out) {
            Ok(())
        } else {
            Err(MemoryRefusal { span })
        }
    }
}

/// A [`SnapshotView`] as the lane's own `LaneMemory`.
///
/// This is the seam the post-bracket decode runs on. It takes ORIGINAL
/// addresses — the same addresses, the same offset constants, the same reader
/// code — and answers out of the frozen arena. There is no path from here back
/// to the observed process: [`SnapshotView`] holds no memory handle.
///
/// A byte the observation did not copy reads as `None`, which is what every
/// `LaneMemory` caller already treats as "this field could not be read". So a
/// copy plan that under-asks produces the SAME outcome an unreadable engine
/// range produces — a named refusal — and never a zero standing in for data.
#[derive(Clone, Copy)]
pub struct Domain47ViewMemory<'a> {
    view: SnapshotView<'a>,
}

impl<'a> Domain47ViewMemory<'a> {
    pub fn new(view: SnapshotView<'a>) -> Self {
        Self { view }
    }

    pub fn view(&self) -> &SnapshotView<'a> {
        &self.view
    }

    /// Whether the snapshot holds `len` bytes at `addr` for this consumer.
    /// The coverage tests ask this; production readers go through the trait.
    pub fn covers(&self, addr: u64, len: usize) -> Result<(), ViewRefusal> {
        let mut sink = vec![0u8; len];
        self.view.read_into(addr, &mut sink)
    }
}

impl domain47::extractor::LaneMemory for Domain47ViewMemory<'_> {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        self.view.u8_at(addr).ok()
    }

    fn read_array<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        self.view.read_into(addr, &mut out).ok().map(|()| out)
    }

    fn read_object_window<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        // The whole-range form, exactly as the live implementation defines it:
        // `Some` only when ONE request covered the range. `read_into` stitches
        // across intervals, and stitching is the copy-plan analogue of a range
        // that straddles two committed regions — which the live
        // `read_object_window` refuses. `bytes` is the non-stitching form.
        self.view.bytes(addr, N).ok().map(|slice| {
            out.copy_from_slice(slice);
            out
        })
    }
}
