//! d81's observation phase as a COPY PLAN — stage S4 of the capture floor.
//!
//! # What this is
//!
//! `rail_replay_state_live` owns the capture's critical lane, and what is left
//! inside its clock bracket after lane EK moved the compute out is OBSERVATION:
//! guarded reads and their witnesses. Lane FX priced converting them and named
//! nine things the substrate could not say; lane FY closed all nine and proved
//! the phase EXPRESSIBLE as one recipe program, row-for-row against a
//! direct-read walk, refusals included
//! ([`super::tests::d81_recipe_expressibility`]). This module is that program
//! at PRODUCTION offsets and production roots — the shipped builder, so there
//! is one plan and not a test's copy of one.
//!
//! # Every offset here is the live reader's own
//!
//! Not one literal in this file is new. `D81_GS_RAIL_MANAGER_OFF`,
//! `D81_ACTIVE_ENTRY_STRIDE`, `RAIL_INFO_PAIRS_PTR_OFF`,
//! `D81_SELECTOR_*`, `D81_MAP_PROVINCE_RECORD_OFF`, `D81_ANCHOR_*` and the rest
//! are the constants `read_compact_rail_replay_state` forms its own addresses
//! from, and they moved to module scope for this file rather than being copied
//! into it. That is lane FZ's rule, applied after its own review found a test
//! carrying a private copy of the selector offsets and passing while a
//! production offset drifted: a plan that copies a superset of the WRONG ranges
//! is not a superset of anything.
//!
//! # The four province-indexed tables, and where each one's count comes from
//!
//! Four of d81's reads index into contiguous engine tables — the rail table,
//! the active-province table, the gamestate province array and the map
//! container's province array — and those four are the whole of the batching
//! win. `DecodeExtent` decodes a count and a base out of ONE copied object, and
//! the engine puts the two together for two of them:
//!
//! | table | base | count |
//! |---|---|---|
//! | gamestate province array | `gamestate + 0x2B0` | `u32[gamestate + 0x2BC]`, the engine's own |
//! | map container's province array | `container + 0x68` | `u32[container + 0x74]`, the engine's own |
//! | rail table | `rail_manager + 0x08` | nowhere beside it — a `CountExpr` constant |
//! | active table | `active_root + 0x10` | nowhere beside it — a `CountExpr` constant |
//!
//! The two constants come from the CALLER, which enumerates them inside its own
//! bracket exactly as `d81_rail_slot_extent` does in production today.
//! Enumerating them before the bracket would be last hour's tables, which is
//! the hour-attribution rule and not a style preference. They are still bounded
//! by each op's `max_count` and `max_bytes`, so a caller cannot widen a copy
//! past the recipe's `Limits`.
//!
//! # The scattered half is SELECTIVE, and that is where the win is
//!
//! The live reader touches a map province, a record, a source `CProvince` and a
//! rail-info object only for provinces whose rail slot was non-null — at t26,
//! 3,510 of 13,413. A plan that hung those families off the ARRAYS' own
//! elements would do all four for every province, which is more transactions
//! than the live reader issues rather than fewer, and would still publish
//! identical rows.
//!
//! So the two arrays that carry pointers are REGISTERED
//! ([`TABLE_SOURCE`], [`TABLE_MAP`]) and reached by `JoinBounded` from inside
//! the rail branch, on the province id the rail table's element index carries.
//! Only railed provinces enter that branch. The joined slot copies are
//! deduplicated into the array's own interval, so they cost no bytes and buy
//! the stage tag and the dependency edge.
//!
//! Measured at the gate's own shape (512 provinces, 171 railed): 1,035
//! intervals and 2,240 guarded copies, which is nine fixed plus SIX intervals
//! per railed province and nothing per unrailed one. It is pinned by
//! `the_shipped_plans_province_tables_are_a_fixed_number_of_transactions`,
//! whose ceiling is per RAILED province and says why.
//!
//! # The closing program, and why it needs mid-walk closure
//!
//! d81 closes its rail header INSIDE the province's own iteration: the window
//! between the two images is that province's handful of reads. A program that
//! closed at the end of the observation would compare the header against a
//! version 13,412 provinces later and report `ClosureChanged` for every write
//! the engine made to a province the walk had already passed. That is
//! [`CloseWhen::AfterWave`], lane GC's executor topology change, and it is the
//! reason the conversion needed one.
//!
//! # What is NOT here, stated so it is a boundary and not an omission
//!
//! 1. **The reader's own reads are not yet served from the view.** That is the
//!    step that changes what a capture publishes, and d81 has no end-to-end
//!    fixture — staging a `CGameState` complete enough to drive
//!    `read_compact_rail_replay_state` past its `RailRoots` stage is a fixture
//!    nobody has built (see the head of `super::tests::d81_frame_cost`). So the
//!    only instrument that can compare the converted reader against the live
//!    one is a capture, and [`D81_COPY_BRACKET_ARMED`] is OFF.
//!
//! 2. **The hub.** `read_compact_rail_hub_simulation` and its companion tree —
//!    country and country-system windows, the foreign-source vectors, the route
//!    map, the controller fields, the fallback-gate chain and the hash tables —
//!    are not in this plan. That is lane FX's own scoping and its second
//!    finding: the hub's expensive half is ALREADY copy-only, because
//!    `d81_hash_table_rows` computes a whole-table extent from the engine's own
//!    header and takes it as one adjacent pair through `d81_read_stable_bytes`
//!    before anything is decoded from it. A copy plan has nothing to win where
//!    the win is already taken. Converting the REST of the hub is a second
//!    lane's worth of work and is recorded as a residual rather than implied by
//!    silence.

use crate::snapshot_recipes::executor::{
    Admissibility, CarrySource, CloseStage, CloseWhen, ClosingProgram, CompareMask, CountExpr,
    CountTerm,
    DisagreeAction, OpMod, RejectAction, RelWindow, Recipe, RecipeOp, Root, SampleSpec, Tolerance,
};
use crate::snapshot_recipes::{ConsumerId, Limits, MemoryRefusal, RecipeId, SnapshotMemory, Span};

use super::{
    D81_ACTIVE_ENTRY_STRIDE, D81_ACTIVE_ROOT_TABLE_OFF, D81_ACTIVE_WINDOW_LEN,
    D81_ACTIVE_WINDOW_OFFSET, D81_ANCHOR_PRIMARY_OFF, D81_ANCHOR_SECONDARY_OFF,
    D81_CONTAINER_PROVINCE_ARRAY_OFF, D81_CONTAINER_PROVINCE_COUNT_OFF,
    D81_GS_ACTIVE_PROVINCE_ROOT_OFF, D81_GS_ANCHOR_SELECTOR_MAP_OFF, D81_GS_RAIL_MANAGER_OFF,
    D81_IDLER_OWNER_OFF, D81_MAP_PROVINCE_RECORD_OFF, D81_OWNER_CONTAINER_OFF,
    D81_RAIL_MANAGER_TABLE_OFF, D81_RECORD_SELECTOR_KEY_OFF, D81_SELECTOR_ENTRY_STRIDE,
    D81_SELECTOR_MAP_MASK_OFF, D81_SELECTOR_MAP_MAX_DISTANCE_OFF, D81_SELECTOR_MAP_TABLE_OFF,
    RAIL_HEADER_WINDOW_LEN, RAIL_INFO_PAIRS_PTR_OFF,
};

/// THE CONVERSION SWITCH, and it is OFF.
///
/// `false` is not a placeholder for "not finished wiring". It is the statement
/// that no instrument exists offline to compare the converted reader against
/// the live one — d81 has no end-to-end fixture — so the first evidence either
/// way comes from a capture, and arming it is an owner decision taken with that
/// capture's numbers in hand rather than a lane's.
///
/// A `const` and not an environment variable, deliberately: an env-gated read
/// path is a second production configuration that no gate runs, and the
/// measurement law here is that what ships is what was measured.
pub const D81_COPY_BRACKET_ARMED: bool = false;

/// The consumer every d81 post-bracket reader would take its view as.
pub const D81_CONSUMER: ConsumerId = ConsumerId(81);
pub const D81_RECIPE: RecipeId = RecipeId(81);

/// The engine's own ceiling on the province tables, from
/// `compact_contract::rail_replay`. A plan that asked for more than this has
/// decoded a torn count, and the substrate refuses rather than copying it.
const D81_MAX_PROVINCES: u32 = super::MAX_MAP_CONTROL_PROVINCE_COUNT;

/// Widest single copy the plan may ask for.
///
/// The four province-indexed tables at the engine ceiling are the largest:
/// 65,536 entries at the active table's 0x20 stride is 2 MiB. Lane FZ's
/// `D81_BATCH_MAX_EXTENT_BYTES` bounds its two hand-built pairs at 4 MiB for
/// the same reason and this is the same number, so the two batching mechanisms
/// cannot disagree about what is too wide.
const D81_MAX_SINGLE_RANGE_BYTES: u64 = super::D81_BATCH_MAX_EXTENT_BYTES as u64;

/// Stage tags. The order here IS the closing order; see [`closing_program`].
pub const STAGE_RAIL_HEADER: u16 = 1;
pub const STAGE_SELECTOR: u16 = 2;
pub const STAGE_SOURCE: u16 = 3;
pub const STAGE_MAP: u16 = 4;

/// The wave the rail header's stage closes at.
///
/// COUNTED DOWN THE BRANCH, not chosen: the gamestate root is wave 0, the rail
/// manager lands at wave 1, the rail-table extent at wave 2, and each rail
/// header at wave 3. The pair blob is an extent off the header and copies the
/// BLOB, not the header, so wave 3 is the last wave that writes into the
/// header's interval.
///
/// A wave too early is not a wrong witness, it is a
/// `Refusal::ClosedIntervalReopened` — see [`CloseWhen`] — so this number is
/// checked by the substrate rather than trusted.
const RAIL_HEADER_CLOSE_WAVE: u32 = 3;

/// Op indices, named. A recipe is a flat list whose branch targets are indices
/// into it, so every index is written once here and nowhere else.
mod op {
    pub const GAMESTATE: u16 = 0;
    pub const RAIL_MANAGER_PTR: u16 = 1;
    pub const RAIL_MANAGER: u16 = 2;
    pub const ACTIVE_ROOT_PTR: u16 = 3;
    pub const ACTIVE_ROOT: u16 = 4;
    pub const SELECTOR_MAP_PTR: u16 = 5;
    pub const SELECTOR_MAP: u16 = 6;
    pub const SELECTOR_TABLE: u16 = 7;
    pub const RAIL_TABLE: u16 = 8;
    pub const RAIL_SLOT_PTR: u16 = 9;
    pub const RAIL_HEADER: u16 = 10;
    /// Unused: the pair blob is an extent off the header, not a followed
    /// pointer. Held so the indices below do not shift.
    pub const _RESERVED_PAIR_PTR: u16 = 11;
    pub const PAIR_BLOB: u16 = 12;
    pub const ACTIVE_TABLE: u16 = 13;
    pub const ACTIVE_ENTRY: u16 = 14;
    pub const SOURCE_ARRAY: u16 = 15;
    pub const SOURCE_SLOT: u16 = 16;
    pub const MAP_ARRAY: u16 = 17;
    pub const MAP_SLOT: u16 = 18;
    pub const MAP_PROVINCE_PTR: u16 = 19;
    pub const MAP_WINDOWS: u16 = 20;
    pub const RECORD_PTR: u16 = 21;
    pub const RECORD_WINDOW: u16 = 22;
    pub const IDLER_SLOT: u16 = 23;
    pub const IDLER_PTR: u16 = 24;
    pub const IDLER: u16 = 25;
    pub const OWNER_PTR: u16 = 26;
    pub const OWNER: u16 = 27;
    pub const CONTAINER_PTR: u16 = 28;
    pub const CONTAINER: u16 = 29;
    pub const WITNESS: u16 = 30;
    pub const FREEZE: u16 = 31;
    /// THE SECOND ROUTE TO THE GAMESTATE PROVINCE ARRAY.
    ///
    /// d81's anchor-selector resolver serves the province slot out of the
    /// walk's own opening read whenever the engine's selector key is the
    /// province's own id: two logical reads at one address. Deduplication gives
    /// them ONE interval carrying both stages, which is exactly the shape
    /// ruling B's elision exists for — and it is why this op exists as a
    /// separate index rather than as a second `stage` on `SOURCE_ARRAY`, which
    /// could carry only one.
    pub const SELECTOR_ROUTE: u16 = 32;
    /// The join that makes the scattered half SELECTIVE.
    ///
    /// `container[id] -> map province -> record` is work the live reader does
    /// only for provinces whose rail slot was non-null. Hanging it off the map
    /// array's own elements would do it for every province the container
    /// declares — at t26 that is 13,413 instead of 3,510, and it is more
    /// transactions than the live reader issues rather than fewer. So the map
    /// slot is reached by JOINING into the registered array on the province id
    /// the rail table's element index carries, from inside the rail branch,
    /// which only railed provinces enter.
    pub const JOIN_MAP: u16 = 33;
    pub const MAP_SLOT_PTR: u16 = 34;
    /// The SECOND clock singleton slot. `map_container` reads both and refuses
    /// the frame unless they hold the same pointer, so a plan that copied only
    /// one would leave the reader's own equality check reading a byte nothing
    /// captured.
    pub const IDLER_SLOT_B: u16 = 35;
    /// The gamestate `CProvince` behind a province-array slot.
    ///
    /// `d81_resolve_anchor_selector_key` reads its vtable at `+0` and its id at
    /// `CPROVINCE_OFF_ID`, and both are inside the window this op copies.
    pub const SOURCE_PROVINCE_PTR: u16 = 36;
    pub const SOURCE_PROVINCE_WINDOW: u16 = 37;
    /// The join that reaches the gamestate province slot, from the rail branch,
    /// on the province id the rail table's element index carries.
    pub const JOIN_SOURCE: u16 = 38;
    pub const SOURCE_SLOT_PTR: u16 = 39;
    /// The selector resolver's SECOND join into the same array. Two logical
    /// reads at one address; deduplication gives them one interval carrying
    /// both stages, which is the shape ruling B's elision exists for.
    pub const JOIN_SELECTOR: u16 = 40;
    pub const COUNT: usize = 41;
}

/// The registered join table. Only one is needed: the gamestate province array
/// and the active table are read by ADDRESS post-bracket (they are whole
/// copies and their elements start no branch), and only the map array's
/// elements are pointers the plan has to follow.
const TABLE_MAP: crate::snapshot_recipes::TableId = crate::snapshot_recipes::TableId(1);
/// The gamestate province array, registered so the rail branch can join into
/// it on the carried province id.
const TABLE_SOURCE: crate::snapshot_recipes::TableId = crate::snapshot_recipes::TableId(2);

/// Where the plan starts.
///
/// Every address here is one the live reader forms inside its own bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain81PlanRoots {
    /// `state.date_ptr`, the `CGameState` singleton.
    pub game_state: u64,
    /// `state.base + DOMAIN57_CLOCK_SINGLETON_A_RVA`: the static slot the map
    /// container chain hangs off.
    pub idler_slot: u64,
    /// `state.base + DOMAIN57_CLOCK_SINGLETON_B_RVA`. The reader reads BOTH
    /// slots and refuses unless they hold the same pointer, so the plan copies
    /// both or that check reads a byte nothing captured.
    pub idler_slot_b: u64,
    /// The map container object, 's own
    /// result.
    ///
    /// A ROOT, even though the plan also walks the idler chain that reaches it,
    /// and the reason is the WAVE it registers its province array at. The join
    /// that reaches a map province runs in the rail branch at wave 3; the
    /// container is four copies down the idler chain and would register its
    /// array in that same wave, so which of the two ran first would depend on
    /// intra-wave task order. Rooting the container makes the array register at
    /// wave 0 and the ordering a property of the plan rather than of the
    /// queue. Deduplication means the idler chain still costs what it costs and
    /// the container object is copied once.
    pub container: u64,
    /// How many province slots the rail and active tables hold.
    ///
    /// The caller's own in-bracket enumeration, exactly the way
    /// `d81_rail_slot_extent` sizes its extent today: the engine keeps no count
    /// beside either of those two bases. It rides in as a `CountExpr` CONSTANT,
    /// so the substrate's `max_count` and `max_bytes` still bound it and a
    /// caller cannot widen a copy past the recipe's `Limits`.
    pub province_slots: u32,
}

/// The heap window every pointer the plan follows must lie in.
///
/// `is_map_control_heap_pointer` is `[HEAP_LO, HEAP_HI)` plus 8-byte alignment,
/// and it is the predicate the live reader rejects a rail pointer with. The
/// plan admits on the SAME predicate before it follows one, so a pointer the
/// reader would refuse is a recorded `PointerRejection` here rather than a copy
/// of unrelated bytes.
fn heap_window(on_reject: RejectAction) -> Admissibility {
    Admissibility {
        low: super::HEAP_LO as u64,
        high: super::HEAP_HI as u64,
        align: 8,
        singletons: [0, 0],
        on_reject,
    }
}

/// RULING B. d81's closing sequence, with the rail header closing MID-WALK.
///
/// The order is the reader's own: the rail header first (it is closed inside
/// the province's iteration), then the selector's route to the gamestate slot
/// ELIDED into the source-position read that covers it — d81's own
/// coincident-slot elision, kept because it is what carries the selector
/// stage's attribution — then the source position, then the map binding.
///
/// The `when`s are non-decreasing down the list, which `validate` requires: the
/// listed sequence and the wave numbers have to be one claim.
pub fn closing_program() -> ClosingProgram {
    ClosingProgram {
        stages: vec![
            CloseStage {
                stage: STAGE_RAIL_HEADER,
                elide_into: None,
                when: CloseWhen::AfterWave(RAIL_HEADER_CLOSE_WAVE),
            },
            CloseStage {
                stage: STAGE_SELECTOR,
                elide_into: Some(STAGE_SOURCE),
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: STAGE_SOURCE,
                elide_into: None,
                when: CloseWhen::Final,
            },
            CloseStage {
                stage: STAGE_MAP,
                elide_into: None,
                when: CloseWhen::Final,
            },
        ],
    }
}

/// Build d81's copy plan.
pub fn domain81_copy_plan(roots: &Domain81PlanRoots) -> Recipe {
    let mut ops = vec![RecipeOp::Freeze; op::COUNT];

    // ---- the gamestate chain: three roots, none of them a table -----------
    ops[op::GAMESTATE as usize] = RecipeOp::CopyHeader {
        // Through the END of the last field the reader takes off the
        // gamestate, and no further. Three pointers at 0x3D8, 0x3E0 and 0x698.
        size: (D81_GS_ANCHOR_SELECTOR_MAP_OFF + 8) as u32,
        next: vec![
            op::RAIL_MANAGER_PTR,
            op::ACTIVE_ROOT_PTR,
            op::SELECTOR_MAP_PTR,
            op::SOURCE_ARRAY,
            op::WITNESS,
        ],
    };
    ops[op::RAIL_MANAGER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_RAIL_MANAGER_OFF as u32,
        guard: None,
        next: op::RAIL_MANAGER,
    };
    ops[op::RAIL_MANAGER as usize] = RecipeOp::CopyHeader {
        size: (D81_RAIL_MANAGER_TABLE_OFF + 8) as u32,
        next: vec![op::RAIL_TABLE],
    };
    ops[op::ACTIVE_ROOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_ACTIVE_PROVINCE_ROOT_OFF as u32,
        guard: None,
        next: op::ACTIVE_ROOT,
    };
    ops[op::ACTIVE_ROOT as usize] = RecipeOp::CopyHeader {
        size: (D81_ACTIVE_ROOT_TABLE_OFF + 8) as u32,
        next: vec![op::ACTIVE_TABLE],
    };

    // ---- the anchor-selector map and its derived extent --------------------
    ops[op::SELECTOR_MAP_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_GS_ANCHOR_SELECTOR_MAP_OFF as u32,
        guard: None,
        next: op::SELECTOR_MAP,
    };
    ops[op::SELECTOR_MAP as usize] = RecipeOp::CopyHeader {
        size: (D81_SELECTOR_MAP_MAX_DISTANCE_OFF + 1) as u32,
        next: vec![op::SELECTOR_TABLE],
    };
    // THE DERIVED EXTENT (FY's M1). `select_secondary` starts at the masked
    // slot and advances WITHOUT wrapping, up to `max_distance + 1` steps, and
    // reads the terminating marker one slot beyond that — so a copy of exactly
    // `mask + 1` entries does not cover a probe that starts near the top. The
    // count is `mask + max_distance + 2`, which is the formula d81's own
    // `d81_hash_table_rows` already uses, expressed as a `CountExpr` because no
    // single engine field holds it.
    ops[op::SELECTOR_TABLE as usize] = RecipeOp::DecodeExtent {
        // Unused: the count comes from the expression on this op. Declared
        // anyway because the op's shape requires it, and pointed at the mask so
        // a reader of this file is not left wondering which field was meant.
        count_offset: D81_SELECTOR_MAP_MASK_OFF as u32,
        count_width: 4,
        ptr_offset: D81_SELECTOR_MAP_TABLE_OFF as u32,
        stride: D81_SELECTOR_ENTRY_STRIDE as u32,
        max_count: D81_MAX_SELECTOR_SLOTS,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };

    // ---- the four province-indexed tables ----------------------------------
    //
    // Three are decoded off the object that holds their base; the fourth is a
    // span root because nothing the plan copies holds its base — see
    // [`Domain81PlanRoots::source_array`].
    //
    // All four are `Tolerance::Scoped`, because a copy plan asks for the WHOLE
    // table where the reader only ever touched the entries it walked: an
    // unreadable page in the part nobody reads must cost nothing, which is
    // ruling A and is what lane FZ's page-wise retry buys by hand today.
    ops[op::RAIL_TABLE as usize] = RecipeOp::DecodeExtent {
        // The count is the expression on this op — the engine keeps none
        // beside this base. Pointed at the manager's own table pointer so a
        // reader of this file is not left guessing which field was meant.
        count_offset: D81_RAIL_MANAGER_TABLE_OFF as u32,
        count_width: 4,
        ptr_offset: D81_RAIL_MANAGER_TABLE_OFF as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::RAIL_SLOT_PTR],
    };
    ops[op::ACTIVE_TABLE as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_ACTIVE_ROOT_TABLE_OFF as u32,
        count_width: 4,
        ptr_offset: D81_ACTIVE_ROOT_TABLE_OFF as u32,
        stride: D81_ACTIVE_ENTRY_STRIDE as u32,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::ACTIVE_ENTRY],
    };
    ops[op::ACTIVE_ENTRY as usize] = RecipeOp::Freeze;
    // THE ENGINE'S OWN BASE AND COUNT, both on the gamestate:
    // `u64[gamestate + 0x2B0]` and `u32[gamestate + 0x2BC]`. They are the pair
    // `cprovince_array_still_current` compares the topology's own array
    // against, so decoding them here is reading the same two fields the
    // collector already treats as authoritative for this array.
    ops[op::SOURCE_ARRAY as usize] = RecipeOp::DecodeExtent {
        count_offset: super::CGAMESTATE_CPROVINCE_COUNT as u32,
        count_width: 4,
        ptr_offset: super::CGAMESTATE_CPROVINCE_ARRAY as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        register: Some(TABLE_SOURCE),
        element: vec![op::FREEZE],
    };
    ops[op::JOIN_SOURCE as usize] = RecipeOp::JoinBounded {
        table: TABLE_SOURCE,
        key_offset: 0,
        key_width: 4,
        next: op::SOURCE_SLOT,
    };
    ops[op::SOURCE_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::SOURCE_SLOT_PTR],
    };
    ops[op::SOURCE_SLOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: op::SOURCE_PROVINCE_PTR,
    };
    // The selector resolver's route to the SAME slot, as its own join, so the
    // interval carries both stages and the elision has something to elide.
    ops[op::JOIN_SELECTOR as usize] = RecipeOp::JoinBounded {
        table: TABLE_SOURCE,
        key_offset: 0,
        key_width: 4,
        next: op::SELECTOR_ROUTE,
    };
    ops[op::SELECTOR_ROUTE as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::FREEZE],
    };
    // The slot's `CProvince`, whose vtable and id
    // `d81_resolve_anchor_selector_key` reads and compares.
    ops[op::SOURCE_PROVINCE_PTR as usize] = RecipeOp::CopyRanges {
        // ONE window covering both fields. They are 164 bytes apart in the same
        // object, and a guarded copy costs ~85 ns almost independently of
        // width, so two windows would be two transactions for nothing.
        windows: vec![RelWindow::new(0, super::CPROVINCE_OFF_ID as u32 + 4)],
        next: vec![op::SOURCE_PROVINCE_WINDOW],
    };
    ops[op::SOURCE_PROVINCE_WINDOW as usize] = RecipeOp::Freeze;
    // THE ENGINE'S OWN COUNT. `u32[container + 0x74]` is the bound the live
    // reader refuses every `container[id]` against and it sits in the same
    // object as the array pointer at `+0x68`, so this is the one table whose
    // extent the substrate can size without being told.
    ops[op::MAP_ARRAY as usize] = RecipeOp::DecodeExtent {
        count_offset: D81_CONTAINER_PROVINCE_COUNT_OFF as u32,
        count_width: 4,
        ptr_offset: D81_CONTAINER_PROVINCE_ARRAY_OFF as u32,
        stride: 8,
        max_count: D81_MAX_PROVINCES,
        max_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        trailer_elements: 0,
        guard: None,
        // REGISTERED, and its elements start NO branch. The array lands as one
        // copy; which of its slots is followed is decided by the join below,
        // from inside the rail branch. See `op::JOIN_MAP`.
        register: Some(TABLE_MAP),
        element: vec![op::FREEZE],
    };
    ops[op::JOIN_MAP as usize] = RecipeOp::JoinBounded {
        table: TABLE_MAP,
        key_offset: 0,
        key_width: 4,
        next: op::MAP_SLOT,
    };
    // Deduplicated into the array's own interval: zero extra bytes, and the
    // stage tag and dependency edge the join needs.
    ops[op::MAP_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::MAP_SLOT_PTR],
    };
    ops[op::MAP_SLOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: op::MAP_PROVINCE_PTR,
    };

    // ---- per railed province: the scattered half ---------------------------
    ops[op::RAIL_SLOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: op::RAIL_HEADER,
    };
    // The rail header window, `[+0x38, +0x48)`, taken as an ADJACENT PAIR and
    // compared over the two consumed fields only. The four bytes between them
    // are DEAD and a write to them must not gap the reader — which is exactly
    // what `d81_decode_rail_header_window` exists to state, and what a
    // whole-interval comparison would get wrong.
    // FROM `+0`, THROUGH THE END OF THE COUNT FIELD, and the first 0x38 bytes
    // are an over-read this op takes on purpose.
    //
    // The reader consumes only `[+0x38, +0x48)`. A `CopyRanges` of exactly
    // that window would copy 56 fewer bytes per railed province — which is
    // nothing, because a guarded copy costs ~85 ns almost independently of
    // width — and would leave the object's BASE unowned. `JoinBounded` anchors
    // its dependency edge on the interval containing the SITE's base
    // (`executor.rs`, the `key_from_carry` arm), so a branch rooted on an
    // object whose base is not owned cannot join at all: it refuses
    // `FieldNotCaptured` at the base. Taking the object from `+0` is the
    // cheaper half of that trade by a wide margin.
    ops[op::RAIL_HEADER as usize] = RecipeOp::CopyHeader {
        size: (super::RAIL_INFO_PAIR_COUNT_OFF + 4) as u32,
        next: vec![
            op::PAIR_BLOB,
            op::JOIN_MAP,
            op::JOIN_SOURCE,
            op::JOIN_SELECTOR,
        ],
    };
    // `pair_count * 8` bytes behind the header's pointer.
    //
    // AN EXTENT OFF THE HEADER, NOT A POINTER FOLLOWED BY A FIXED COPY, and
    // the difference is the over-read. The count is variable — 0 to
    // `MAX_NEIGHBORS_PER_PROVINCE` — so a fixed copy would have to take the
    // ceiling, 512 bytes, off allocations that are frequently smaller than
    // that; an all-or-nothing guarded read of 512 bytes past the end of a
    // 32-byte blob fails, and the neighbours are then lost for a province
    // whose bytes were all there. The extent reads BOTH the pointer at window
    // offset 0 and the count at offset 12 out of the header the previous op
    // already copied, so it asks for exactly what the engine says is there and
    // the engine's own ceiling bounds it.
    ops[op::PAIR_BLOB as usize] = RecipeOp::DecodeExtent {
        count_offset: super::RAIL_INFO_PAIR_COUNT_OFF as u32,
        count_width: 4,
        ptr_offset: RAIL_INFO_PAIRS_PTR_OFF as u32,
        stride: 8,
        max_count: compact_contract::rail_replay::MAX_NEIGHBORS_PER_PROVINCE as u32,
        max_bytes: (compact_contract::rail_replay::MAX_NEIGHBORS_PER_PROVINCE * 8) as u64,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![op::FREEZE],
    };

    // ---- the map province, reached through the container's array -----------
    //
    // The map province is over 5 KiB and the reader consumes 32 bytes of it,
    // in three places: the two anchor triplets and the record pointer. Three
    // windows, not one object — the whole-object copy would be 160x the bytes
    // for no call saving at all.
    ops[op::MAP_PROVINCE_PTR as usize] = RecipeOp::CopyRanges {
        windows: vec![
            RelWindow::new(D81_ANCHOR_PRIMARY_OFF as i64, 12),
            RelWindow::new(D81_ANCHOR_SECONDARY_OFF as i64, 12),
            RelWindow::new(D81_MAP_PROVINCE_RECORD_OFF as i64, 8),
        ],
        next: vec![op::RECORD_PTR],
    };
    ops[op::RECORD_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_MAP_PROVINCE_RECORD_OFF as u32,
        guard: None,
        next: op::RECORD_WINDOW,
    };
    ops[op::RECORD_WINDOW as usize] = RecipeOp::CopyRanges {
        windows: vec![RelWindow::new(D81_RECORD_SELECTOR_KEY_OFF as i64, 4)],
        next: vec![op::FREEZE],
    };

    // ---- the map container chain, off the static idler slot ----------------
    ops[op::IDLER_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::IDLER_PTR],
    };
    ops[op::IDLER_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: op::IDLER,
    };
    ops[op::IDLER as usize] = RecipeOp::CopyHeader {
        // Through the end of the owner pointer, plus the vtable at +0 the
        // reader compares against `DOMAIN57_CLOCK_VTABLE_RVA`.
        size: (D81_IDLER_OWNER_OFF + 8) as u32,
        next: vec![op::OWNER_PTR],
    };
    ops[op::OWNER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_IDLER_OWNER_OFF as u32,
        guard: None,
        next: op::OWNER,
    };
    ops[op::OWNER as usize] = RecipeOp::CopyHeader {
        size: (D81_OWNER_CONTAINER_OFF + 8) as u32,
        next: vec![op::CONTAINER_PTR],
    };
    ops[op::CONTAINER_PTR as usize] = RecipeOp::DecodePointer {
        offset: D81_OWNER_CONTAINER_OFF as u32,
        guard: None,
        next: op::CONTAINER,
    };
    ops[op::CONTAINER as usize] = RecipeOp::CopyHeader {
        // Through the end of the province COUNT at +0x74, which the reader
        // bounds every `container[id]` against and which the map array's
        // extent decodes its own count from.
        size: (D81_CONTAINER_PROVINCE_COUNT_OFF + 4) as u32,
        next: vec![op::MAP_ARRAY],
    };

    // The second clock singleton, which `map_container` compares against the
    // first before it will follow either.
    ops[op::IDLER_SLOT_B as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![op::FREEZE],
    };

    ops[op::WITNESS as usize] = RecipeOp::WitnessClose {
        next: vec![op::FREEZE],
    };

    // ---- the qualifications ------------------------------------------------
    let mut mods = vec![OpMod::default(); op::COUNT];

    // M1: the selector extent's count is the engine's own formula.
    mods[op::SELECTOR_TABLE as usize].count = Some(CountExpr {
        terms: vec![
            CountTerm {
                offset: D81_SELECTOR_MAP_MASK_OFF as u32,
                width: 4,
            },
            CountTerm {
                offset: D81_SELECTOR_MAP_MAX_DISTANCE_OFF as u32,
                width: 1,
            },
        ],
        constant: 2,
    });

    // RULING A. Anything derived from an ENGINE TABLE the plan copies
    // wholesale is `Scoped`; anything reached through a pointer the plan
    // proved admissible is `Required`, because a copy plan that cannot read an
    // object it was handed the address of has found a real defect and must say
    // so. The four tables and the entries taken out of them are the first set.
    for scoped in [
        op::RAIL_TABLE,
        op::ACTIVE_TABLE,
        op::SOURCE_ARRAY,
        op::SOURCE_SLOT,
        op::SELECTOR_ROUTE,
        op::MAP_ARRAY,
        op::MAP_SLOT,
        op::SELECTOR_TABLE,
        // AND THE SCATTERED HALF, for a reason the wholesale tables do not
        // have (Codex 019fe963, finding C1). The topology walk SKIPS provinces
        // whose gamestate slot is null, and the reader iterates only the roots
        // that survived it — but a skipped index can still carry a non-null
        // rail slot, and the plan cannot tell the two apart because that is a
        // decision. So the plan follows objects the reader never visits, and
        // under  one unreadable byte in any of them would refuse the
        // whole frame for a province nobody reads.
        //
        // Scoped is not weaker for the provinces that ARE visited: the void
        // makes the post-bracket read refuse, and the reader turns that into
        // the same typed failure an unreadable live read produces. It is only
        // weaker for the ones that are not, which is the point.
        op::RAIL_HEADER,
        op::PAIR_BLOB,
        op::MAP_PROVINCE_PTR,
        op::RECORD_WINDOW,
        op::SOURCE_PROVINCE_PTR,
    ] {
        mods[scoped as usize].tolerance = Tolerance::Scoped;
    }

    // M3: every pointer the live reader gates with
    // `is_map_control_heap_pointer` is admitted on the same predicate here,
    // and a rejection STOPS THE BRANCH rather than refusing the recipe — the
    // reader turns the recorded rejection into its own typed `InvalidPointer`
    // for that province and keeps the frame.
    for admitted in [
        op::RAIL_SLOT_PTR,
        op::PAIR_BLOB,
        op::MAP_SLOT_PTR,
        op::SOURCE_SLOT_PTR,
        op::RECORD_PTR,
    ] {
        mods[admitted as usize].admit = Some(heap_window(RejectAction::StopBranch));
    }
    // The four ROOT pointers are the other direction: the live reader refuses
    // the whole snapshot at `RailRoots` when one of them fails the same
    // predicate, so a rejection here is fatal to the recipe and carries the
    // window it failed.
    for fatal in [
        op::RAIL_MANAGER_PTR,
        op::ACTIVE_ROOT_PTR,
        op::SELECTOR_MAP_PTR,
        op::IDLER_PTR,
        op::OWNER_PTR,
        op::CONTAINER_PTR,
    ] {
        mods[fatal as usize].admit = Some(heap_window(RejectAction::RefuseRecipe));
    }

    // M4 + RULING B: the rail header is taken as an adjacent pair and compared
    // ONLY over the two fields the reader consumes.
    mods[op::RAIL_HEADER as usize].samples = SampleSpec {
        count: 2,
        on_disagree: DisagreeAction::Gap,
    };
    mods[op::RAIL_HEADER as usize].compare = CompareMask {
        windows: vec![
            RelWindow::new(RAIL_INFO_PAIRS_PTR_OFF as i64, 8),
            RelWindow::new(super::RAIL_INFO_PAIR_COUNT_OFF as i64, 4),
        ],
    };

    // The active entry's coherence gate consumes seven bytes at +0x12 of the
    // entry and nothing else; a write anywhere else in the 0x20-byte entry is
    // not a torn read of it. Per ELEMENT, which is what makes the mask one
    // entry rather than one per province.
    mods[op::ACTIVE_TABLE as usize].compare = CompareMask {
        windows: vec![RelWindow::new(D81_ACTIVE_WINDOW_OFFSET as i64, D81_ACTIVE_WINDOW_LEN as u32)],
    };

    // RULING B's stages, and every one of them sits on an op that COPIES —
    // a stage tag on a  or a  names no interval and
    // would silently order nothing.
    mods[op::RAIL_HEADER as usize].stage = STAGE_RAIL_HEADER;
    mods[op::SOURCE_SLOT as usize].stage = STAGE_SOURCE;
    mods[op::SELECTOR_ROUTE as usize].stage = STAGE_SELECTOR;
    mods[op::MAP_PROVINCE_PTR as usize].stage = STAGE_MAP;
    // M2: the province id is the rail table element index, carried into the
    // join that reaches this province and only this province.
    mods[op::RAIL_SLOT_PTR as usize].carry = Some(CarrySource::ElementIndex);
    mods[op::JOIN_MAP as usize].key_from_carry = true;
    mods[op::JOIN_SOURCE as usize].key_from_carry = true;
    mods[op::JOIN_SELECTOR as usize].key_from_carry = true;

    // THE TWO COUNTS THE ENGINE DOES NOT KEEP. The rail and active tables have
    // a base and no count beside it, so the caller's own in-bracket
    // enumeration rides in as a `CountExpr` constant. It is still bounded by
    // `max_count` and `max_bytes` on the op, so this is a number the plan is
    // TOLD and not a number it trusts.
    let enumerated = CountExpr {
        terms: Vec::new(),
        constant: roots.province_slots.max(1),
    };
    mods[op::RAIL_TABLE as usize].count = Some(enumerated.clone());
    mods[op::ACTIVE_TABLE as usize].count = Some(enumerated);

    let mut recipe = Recipe::new(
        D81_RECIPE,
        D81_CONSUMER,
        ops,
        vec![
            Root {
                op: op::GAMESTATE,
                addr: roots.game_state,
            },
            Root {
                op: op::IDLER_SLOT,
                addr: roots.idler_slot,
            },
            Root {
                op: op::IDLER_SLOT_B,
                addr: roots.idler_slot_b,
            },
            Root {
                op: op::CONTAINER,
                addr: roots.container,
            },
        ],
    );
    recipe.limits = Limits {
        max_single_range_bytes: D81_MAX_SINGLE_RANGE_BYTES,
        ..Limits::default()
    };
    let recipe = recipe.with_mods(mods);
    recipe.with_closing(closing_program())
}

/// A span's width as an op size, saturated at the plan's own single-range cap
/// so a caller cannot smuggle a wider request past `Limits`.
fn span_width(span: Span) -> u32 {
    span.len.min(D81_MAX_SINGLE_RANGE_BYTES).min(u32::MAX as u64) as u32
}

/// Offset of the pair COUNT inside the rail-header window. Derived from the two
/// production field offsets, exactly as `RAIL_HEADER_WINDOW_LEN` is.
const RAIL_INFO_PAIR_COUNT_IN_WINDOW: usize = super::RAIL_HEADER_COUNT_IN_WINDOW;

/// The largest selector table the derived count may ask for, in slots. The
/// engine's mask is a power of two minus one and the probe tail adds
/// `max_distance + 2`; `u8::MAX` bounds the distance, so this is the ceiling
/// past which a torn mask refuses rather than being copied.
const D81_MAX_SELECTOR_SLOTS: u32 = 1 << 20;

/// The live process, as the substrate's bulk-copy primitive.
///
/// One guarded, all-or-nothing range copy per call — the same
/// `range_confirmed_good` gate and the same `copy_selected` every other read in
/// this file goes through. Nothing else about the process is exposed: the
/// substrate can move bytes through this and cannot compute over them.
pub struct Domain81SnapshotMemory;

/// [`Domain81SnapshotMemory`] with a call counter.
///
/// The shadow reports `d81_shadow_copies_issued`, and the only honest source
/// for that number is the substrate's own calls — including the ones that
/// FAILED, because a bisected scoped copy's retries and an adjacent sample are
/// both work the engine did. Deriving it from the interval count omits both
/// (Codex 019fe9b6, finding F).
///
/// A `Cell`, not an atomic: one observation runs on one thread, and
/// `SnapshotMemory::copy_into` takes `&self`.
#[derive(Default)]
pub struct CountingSnapshotMemory {
    calls: std::cell::Cell<u64>,
}

impl CountingSnapshotMemory {
    pub fn calls(&self) -> u64 {
        self.calls.get()
    }
}

impl SnapshotMemory for CountingSnapshotMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        self.calls.set(self.calls.get().saturating_add(1));
        Domain81SnapshotMemory.copy_into(addr, out)
    }
}

impl SnapshotMemory for Domain81SnapshotMemory {
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

// ===========================================================================
// THE SHADOW: the plan run beside the live reader, and the equality it claims
// ===========================================================================

/// The environment spelling of the shadow's on switch.
///
/// AN ENVIRONMENT VARIABLE AND A CONTROL KEY, both, neither required — exactly
/// `BATTLE_PLAN_D47_SWEEP_SHADOW`'s shape and for its reason: the orchestrator's
/// transition captures are started by the WMI/env path, where there is no
/// control block to write, while a recorder-hosted capture has no environment
/// to set. Either arms it.
///
/// NOT a cargo feature, following `custody_shadow::CUSTODY_SHADOW_ENV`: a
/// feature forks the binary, and then the only collector that can answer the
/// question is a different collector from the one that ships.
///
/// # WHAT ARMING COSTS THE FRAME, and it is not nothing
///
/// The plan itself runs in the COMPUTE half, after d81's clock bracket has
/// closed and its verdict is decided, so it cannot change d81's own
/// publication. Two smaller routes remain, both ARMED-ONLY, and they are
/// written here rather than discovered in a capture (Codex 019fea08):
///
/// 1. **The witness collection is inside d81's bracket.** One struct move per
///    published province, into capacity reserved once before the walk. It is
///    irreducible if the witness is to exist at all — the values it holds are
///    this hour's and exist nowhere else — and it is time the guard's closing
///    observation is taken after, so an armed frame's bracket is very slightly
///    wider than a disarmed one's.
///
/// 2. **The reader-gate permit is held across the compute half.** The shadow
///    therefore delays other readers queued behind d81 in the same frame, and a
///    reader delayed far enough can open in a later game hour.
///
/// Both are the same shape as every other shadow in this collector — the d47
/// sweep shadow costs two needles in the shared scan and a second admission
/// pass over ~3,400 role objects, inside the frame — and both are why an armed
/// capture's DELIVERY is not comparable to a disarmed one's. Its EQUALITY
/// numbers are what it is for, and those are unaffected by either.
pub const D81_COPY_BRACKET_SHADOW_ENV: &str = "BATTLE_PLAN_D81_COPY_BRACKET_SHADOW";

/// The fields the shadow compares, in ordinal order. Ordinal 0 is "no
/// divergence"; the witness on the wire is an index into this table, so a
/// capture names the field rather than a number nobody can decode.
pub const D81_SHADOW_FIELDS: [&str; 11] = [
    "none",
    "rail_slot",
    "rail_header_pair_ptr",
    "rail_header_pair_count",
    "neighbour_pair_bytes",
    "active_window",
    "source_province_slot",
    "map_province_slot",
    "map_record_ptr",
    "record_selector_key",
    "anchor_triplet",
];

/// One province's LIVE reading, kept so the shadow can ask the COPY for the
/// same fields at the same addresses.
///
/// Built only when the shadow is armed. Every field is a value the walk had
/// already decoded and an address it had already formed, so capturing it costs
/// the move and nothing else — the shadow performs no engine read of its own
/// outside the copy plan.
pub struct D81ShadowWitness {
    pub province_id: u32,
    pub rail_slot_addr: usize,
    pub rail_info: usize,
    pub rail_header_addr: usize,
    pub pair_ptr: usize,
    pub pair_count: u32,
    pub pair_bytes: Vec<u8>,
    pub active_window_addr: usize,
    /// The DECODED coherence state, not the raw bytes: the live walk decodes
    /// this window through  and the shadow runs
    /// the same decoder over the copy, so the comparison is of the values the
    /// reader acts on rather than of a byte image it never compares.
    pub active: super::D81ActiveEntryState,
    pub source_slot_addr: usize,
    pub source_province_ptr: usize,
    pub map_slot_addr: usize,
    pub map_province: usize,
    pub record_addr: usize,
    pub record: usize,
    pub selector_key_addr: usize,
    pub selector_key: u32,
    pub anchor_addr: usize,
    pub anchor_bits: [u32; 3],
}

/// What one shadow frame observed. Every field is per-frame and lands on the
/// wire as its own diagnostic unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81ShadowCounters {
    /// The switch was on for this frame.
    pub armed: bool,
    /// The plan ran to a view. False with `armed` true means the observation
    /// refused or custody declined it, and the reason words say which.
    pub ran: bool,
    pub rows_compared: u32,
    pub rows_equal: u32,
    pub rows_divergent: u32,
    /// The FIRST divergence's province and field ordinal. Zero when there was
    /// none; the field ordinal indexes [`D81_SHADOW_FIELDS`].
    pub first_divergence_province: u32,
    pub first_divergence_field: u32,
    pub copies_issued: u64,
    pub intervals_closed: u32,
    pub scoped_voids: u32,
    pub pointers_rejected: u32,
    /// Custody declined the observation before it began.
    pub custody_refused: bool,
    /// An allocation for the shadow's own per-province bookkeeping was
    /// declined, so the collection is INCOMPLETE and its equality figures
    /// cover only the provinces it reached. Recorded rather than fatal: a
    /// diagnostic may not abort a frame the live reader had already read.
    pub witnesses_declined: bool,
    /// The observation ran but the substrate would not issue a view for the
    /// consumer — a torn image, an unmet sample, or the refusal below. Its own
    /// word, because "armed, not ran, no refusal reason" is an inference from
    /// absences rather than a statement.
    pub consumer_gapped: bool,
    pub refused: bool,
    /// [`d81_shadow_refusal_ordinal`], zero when the recipe captured.
    pub refusal_ordinal: u32,
    pub custody_held_bytes: u64,
    pub plan_us: u64,
}

/// A refusal as a stable small ordinal, so the wire can carry WHICH refusal
/// without carrying a string.
///
/// The order is the `Refusal` enum's own declaration order and it is a WIRE
/// vocabulary: a new variant appends, and an existing number never moves.
/// Zero is reserved for "no refusal".
pub fn d81_shadow_refusal_ordinal(refusal: &crate::snapshot_recipes::Refusal) -> u32 {
    use crate::snapshot_recipes::Refusal as R;
    match refusal {
        R::DepthExceeded { .. } => 1,
        R::CopyTaskLimitExceeded { .. } => 2,
        R::RangeLimitExceeded { .. } => 3,
        R::ByteLimitExceeded { .. } => 4,
        R::RangeTooWide { .. } => 5,
        R::ExtentCountExceeded { .. } => 6,
        R::ExtentBytesExceeded { .. } => 7,
        R::JoinOutOfRange { .. } => 8,
        R::JoinTableMissing { .. } => 9,
        R::FieldNotCaptured { .. } => 10,
        R::MemoryUnreadable { .. } => 11,
        R::AddressNotRepresentable { .. } => 12,
        R::PointerNotAdmissible { .. } => 13,
        R::CarryNotSet { .. } => 14,
        R::ConditionalUnresolved { .. } => 15,
        R::ClosedIntervalReopened { .. } => 16,
        R::Malformed { .. } => 17,
    }
}

/// The name a refusal ordinal publishes under, so "refusals by reason" is a set
/// of NAMED units rather than one number a reader has to decode.
pub fn d81_shadow_refusal_unit_name(ordinal: u32) -> &'static str {
    match ordinal {
        1 => "d81_shadow_refusal_01_depth_exceeded",
        2 => "d81_shadow_refusal_02_copy_task_limit",
        3 => "d81_shadow_refusal_03_range_limit",
        4 => "d81_shadow_refusal_04_byte_limit",
        5 => "d81_shadow_refusal_05_range_too_wide",
        6 => "d81_shadow_refusal_06_extent_count",
        7 => "d81_shadow_refusal_07_extent_bytes",
        8 => "d81_shadow_refusal_08_join_out_of_range",
        9 => "d81_shadow_refusal_09_join_table_missing",
        10 => "d81_shadow_refusal_10_field_not_captured",
        11 => "d81_shadow_refusal_11_memory_unreadable",
        12 => "d81_shadow_refusal_12_address_not_representable",
        13 => "d81_shadow_refusal_13_pointer_not_admissible",
        14 => "d81_shadow_refusal_14_carry_not_set",
        15 => "d81_shadow_refusal_15_conditional_unresolved",
        16 => "d81_shadow_refusal_16_closed_interval_reopened",
        17 => "d81_shadow_refusal_17_malformed",
        _ => "d81_shadow_refusal_00_unknown",
    }
}

/// Compare every witnessed province's LIVE reading against the COPY.
///
/// # What equality means here, exactly
///
/// For each field the live walk consumed, the same bytes are read out of the
/// view AT THE SAME ADDRESS and compared to the value the walk decoded. A field
/// the view cannot serve — because the plan did not copy it, or copied it into
/// a void — counts as a DIVERGENCE and not as a skip: "the copy could not
/// answer" is precisely the outcome a conversion has to be told about, and
/// calling it equal would let a plan that copies nothing report perfect
/// agreement.
///
/// # It compares VALUES, not timing, and the difference is the measurement
///
/// The live walk read its fields during the walk; the plan copied them at the
/// top of the bracket. A field the engine rewrote in between diverges
/// legitimately. That is not noise to be tuned away — it is the
/// sampling-window widening lane FX named and lane FZ documented, finally
/// measured against a running game, and it is the number the arming decision
/// turns on.
pub fn d81_shadow_compare(
    view: &crate::snapshot_recipes::view::SnapshotView<'_>,
    witnesses: &[D81ShadowWitness],
    counters: &mut D81ShadowCounters,
) {
    let u32_at = |addr: usize| -> Option<u32> {
        view.bytes(addr as u64, 4)
            .ok()
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    };
    let u64_at = |addr: usize| -> Option<u64> {
        view.bytes(addr as u64, 8)
            .ok()
            .map(|bytes| u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
    };

    for witness in witnesses {
        counters.rows_compared = counters.rows_compared.saturating_add(1);
        // The EARLIEST field of this row that disagreed, not whichever the
        // loop noticed: the checks run in ordinal order and only the first
        // sticks.
        let mut diverged = 0u32;
        {
            let mut check = |ordinal: u32, equal: bool| {
                if diverged == 0 && !equal {
                    diverged = ordinal;
                }
            };
            check(
                1,
                u64_at(witness.rail_slot_addr) == Some(witness.rail_info as u64),
            );
            check(
                2,
                u64_at(witness.rail_header_addr) == Some(witness.pair_ptr as u64),
            );
            check(
                3,
                u32_at(witness.rail_header_addr + super::RAIL_HEADER_COUNT_IN_WINDOW)
                    == Some(witness.pair_count),
            );
            check(
                4,
                witness.pair_bytes.is_empty()
                    || view
                        .bytes(witness.pair_ptr as u64, witness.pair_bytes.len())
                        .map(|bytes| bytes == witness.pair_bytes.as_slice())
                        .unwrap_or(false),
            );
            check(
                5,
                view.bytes(
                    witness.active_window_addr as u64,
                    super::D81_ACTIVE_WINDOW_LEN,
                )
                .ok()
                .and_then(|bytes| <[u8; super::D81_ACTIVE_WINDOW_LEN]>::try_from(bytes).ok())
                .map(|window| super::d81_active_state_from_window(window) == witness.active)
                .unwrap_or(false),
            );
            check(
                6,
                u64_at(witness.source_slot_addr) == Some(witness.source_province_ptr as u64),
            );
            check(
                7,
                u64_at(witness.map_slot_addr) == Some(witness.map_province as u64),
            );
            check(8, u64_at(witness.record_addr) == Some(witness.record as u64));
            check(
                9,
                u32_at(witness.selector_key_addr) == Some(witness.selector_key),
            );
            let anchor_equal = (0..3).all(|axis| {
                u32_at(witness.anchor_addr + axis * 4) == Some(witness.anchor_bits[axis])
            });
            check(10, anchor_equal);
        }

        if diverged == 0 {
            counters.rows_equal = counters.rows_equal.saturating_add(1);
        } else {
            counters.rows_divergent = counters.rows_divergent.saturating_add(1);
            if counters.first_divergence_field == 0 {
                counters.first_divergence_province = witness.province_id;
                counters.first_divergence_field = diverged;
            }
        }
    }
}
