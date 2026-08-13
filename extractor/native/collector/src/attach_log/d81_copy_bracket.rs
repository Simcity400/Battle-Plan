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
//! # The scattered half is SELECTIVE, and it is the VECTOR PIPELINE's now
//!
//! The live reader touches a map province, a record, a source `CProvince` and a
//! rail-info object only for provinces whose rail slot was non-null — at t26,
//! 3,510 of 13,413. This recipe used to reach them through a per-province
//! branch: six intervals and four images each, 83,000 of the plan phase's
//! 84,305 guarded transactions at t50, each paying the substrate's
//! per-interval bookkeeping. Lane HS (2026-08-13, owner mandate) moved those
//! observations into [`super::d81_vector_store`]: flat per-family descriptor
//! lists, copied by an interleaved-pair loop through the same guarded
//! primitive, with demand-driven images (the walk's own consumption table, in
//! `runs/hs_d81_cheap/COST_MODEL.txt`) and per-family bookkeeping. The walk
//! is unchanged — [`D81ReadSource`] serves family windows from the vector
//! capture and everything else from this recipe's view.
//!
//! What remains HERE is the bulk half: the fixed objects and the five engine
//! tables, copied whole with the adjacent images the walk's `_stable` pairs
//! consume. The vector pipeline decodes its descriptors out of these bulk
//! OPENING images, so a scattered address is derived from the same bytes the
//! walk reads and never from a second live read.
//!
//! # Where the closing discipline lives
//!
//! d81 closes its rail header INSIDE the province's own iteration: the window
//! between the two images is that province's handful of reads. The vector
//! pipeline preserves that instant — the header closing pair is taken after
//! the pair-blob descriptors are decoded and before the blobs are copied,
//! which is where the executor's `AfterWave(3)` put it — and takes the slot
//! and active closings LAST, after every family reached through them, so
//! every closing observation lands after the reads it vouches for. The
//! substrate's closing wave no longer runs for this recipe at all; see
//! [`domain81_copy_plan`].
//!
//! # What is NOT here, stated so it is a boundary and not an omission
//!
//! 1. **The plan's own preconditions are still live reads, and they are ~15 of
//!    them.** `roots()`, `selector_header()` and `map_container()` run inside
//!    the bracket exactly as they did, because the plan is rooted on what they
//!    resolve — the container address is a `Root` — and because the four
//!    whole-snapshot closing gates compare the live engine at bracket close
//!    against what they opened with. Fifteen guarded calls against the ~105,000
//!    the walk used to issue is not the number this conversion is about, and
//!    routing them through the view as well would cost the closing gates their
//!    meaning. See [`D81ReadSource`] for what IS served from the copy.
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
    Admissibility, CountExpr, CountTerm, DisagreeAction, OpMod, RejectAction, Recipe, RecipeOp,
    Root, SampleSpec, Tolerance,
};
use crate::snapshot_recipes::{ConsumerId, Limits, MemoryRefusal, RecipeId, SnapshotMemory, Span};

use super::{
    D81_ACTIVE_ENTRY_STRIDE, D81_ACTIVE_ROOT_TABLE_OFF, D81_CONTAINER_PROVINCE_ARRAY_OFF,
    D81_CONTAINER_PROVINCE_COUNT_OFF, D81_GS_ACTIVE_PROVINCE_ROOT_OFF,
    D81_GS_ANCHOR_SELECTOR_MAP_OFF, D81_GS_RAIL_MANAGER_OFF, D81_IDLER_OWNER_OFF,
    D81_MAP_PROVINCE_RECORD_OFF, D81_OWNER_CONTAINER_OFF, D81_RAIL_MANAGER_TABLE_OFF,
    D81_SELECTOR_ENTRY_STRIDE, D81_SELECTOR_MAP_MASK_OFF, D81_SELECTOR_MAP_MAX_DISTANCE_OFF,
    D81_SELECTOR_MAP_TABLE_OFF, RAIL_HEADER_WINDOW_LEN, RAIL_INFO_PAIRS_PTR_OFF,
};

/// THE CONVERSION SWITCH, and it is ON (lane GI, 2026-08-10, owner-funded on
/// t46's equality evidence).
///
/// `true` means the recipe program IS d81's reader: the copy plan runs inside
/// the clock bracket and every byte the province walk decodes comes out of that
/// copy, at the same address, in the compute half. The live guarded walk is off
/// the critical sweep path — it survives only as [`D81_COPY_BRACKET_SHADOW_ENV`]
/// arms it, so a capture can still prove authoritative-equals-live on the wire
/// before the walk's code is deleted.
///
/// WHAT CHANGED THE ANSWER. Lane GC shipped this plan with the switch off and
/// named the reason: no offline instrument could compare a converted reader
/// against the live one. Two things closed that. t46 ran the plan beside the
/// live walk on a real game and compared 7,024 rows with 0 divergent and every
/// refusal-class counter zero. And the walk itself is now parameterised over
/// [`D81ReadSource`], so the SAME walk body runs against a staged world twice —
/// once reading it directly, once through this plan's view — which is the
/// end-to-end equivalence gate that did not exist
/// (`the_authoritative_walk_publishes_what_the_live_walk_publishes`).
///
/// A `const` and not an environment variable, deliberately: an env-gated read
/// path is a second production configuration that no gate runs, and the
/// measurement law here is that what ships is what was measured.
pub const D81_COPY_BRACKET_AUTHORITATIVE: bool = true;

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

/// Op indices, named. A recipe is a flat list whose branch targets are indices
/// into it, so every index is written once here and nowhere else.
///
/// # The per-province branch is GONE (lane HS, 2026-08-13)
///
/// This recipe used to grow a branch per railed province — rail header, pair
/// blob, three joins, the map-province windows, the record window, the
/// `CProvince` window — six intervals and twenty-four guarded transactions
/// per province, 83,000 of the plan phase's 84,305 at t50. Those observations
/// now run as vector families in [`super::d81_vector_store`], which copies
/// the same windows through the same guarded primitive with per-family
/// bookkeeping instead of per-interval bookkeeping. What remains here is the
/// BULK half: the fixed objects and the five engine tables, copied whole,
/// which the vector pipeline and the walk both read their scattered
/// addresses out of.
///
/// The closing program went with the branch: every closing image this recipe
/// used to take was either consumed by the walk through windows the vector
/// store now observes at its own (identically ordered) closing points, or
/// consumed by nobody — the demand table in
/// `runs/hs_d81_cheap/COST_MODEL.txt`, confirmed by the Codex architecture
/// consult (thread 019ff96e-dd04). The substrate's closing wave and its
/// blanket comparison therefore no longer run for d81 at all.
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
    pub const ACTIVE_TABLE: u16 = 9;
    pub const ACTIVE_ENTRY: u16 = 10;
    pub const SOURCE_ARRAY: u16 = 11;
    pub const MAP_ARRAY: u16 = 12;
    pub const IDLER_SLOT: u16 = 13;
    pub const IDLER_PTR: u16 = 14;
    pub const IDLER: u16 = 15;
    pub const OWNER_PTR: u16 = 16;
    pub const OWNER: u16 = 17;
    pub const CONTAINER_PTR: u16 = 18;
    pub const CONTAINER: u16 = 19;
    /// The SECOND clock singleton slot. `map_container` reads both and refuses
    /// the frame unless they hold the same pointer, so a plan that copied only
    /// one would leave the reader's own equality check reading a byte nothing
    /// captured.
    pub const IDLER_SLOT_B: u16 = 20;
    pub const FREEZE: u16 = 21;
    pub const COUNT: usize = 22;
}

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

/// Build d81's copy plan — the BULK half only; see the `op` module header for
/// where the per-province branch went.
///
/// # Where ruling B's closing sequence lives now
///
/// The 2026-08-11 closing-sequence ruling's structure — the rail header
/// closing mid-walk, the selector close elided into the source close, the
/// source and map closes at the end — is preserved at its points of MEANING
/// by the vector pipeline (`d81_vector_observe`): the header closing pair is
/// taken after the pair-blob descriptors are decoded and before the blobs
/// are copied (the executor's `AfterWave(3)` instant); the record-pointer
/// closing pair after the key windows it vouches for; the slot closings and
/// the active closing triples last, after every family reached through them.
/// The owner's 2026-08-13 mandate explicitly authorized re-architecting the
/// closing-image program; what was kept is the discipline (every closing
/// observation lands AFTER the reads it vouches for), and what was dropped
/// is the substrate's stage/elision machinery, whose consumers went with the
/// branch ops.
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
        // The elements start NO branch: the vector pipeline decodes the rail
        // slots out of this table's OPENING image and follows them itself.
        element: vec![op::FREEZE],
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
        register: None,
        element: vec![op::FREEZE],
    };
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
        register: None,
        element: vec![op::FREEZE],
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

    // RULING A. An ENGINE TABLE the plan copies wholesale is `Scoped`: the
    // reader only ever touches the entries it walks, so an unreadable page in
    // the part nobody reads must cost nothing.
    for scoped in [
        op::RAIL_TABLE,
        op::ACTIVE_TABLE,
        op::SOURCE_ARRAY,
        op::MAP_ARRAY,
        op::SELECTOR_TABLE,
    ] {
        mods[scoped as usize].tolerance = Tolerance::Scoped;
    }

    // The ROOT pointers: the live reader refuses the whole snapshot at
    // `RailRoots` when one of them fails `is_map_control_heap_pointer`, so a
    // rejection here is fatal to the recipe and carries the window it failed.
    // (The per-province `StopBranch` admissions moved to the vector pipeline,
    // which applies the same predicate to the same pointers.)
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

    // THE COPIER TAKES THE IMAGES; THE READER RENDERS THE VERDICT (lane GI,
    // 2026-08-10, after t46 measured 78 of 80 frames gapped; unchanged by the
    // vector conversion). Every bulk copy op takes the adjacent images the
    // walk's `_stable` pairs consume and declares `Record`, so the substrate
    // judges nothing and the walk's refusals keep their stage, subject, index
    // and witness. What is GONE is the closing side: no bulk closing image is
    // consumed by the walk (the demand table in
    // `runs/hs_d81_cheap/COST_MODEL.txt`) — the slot and active closings the
    // walk DOES consume are the vector pipeline's closing-only families,
    // observed after every family that was reached through them.
    for op in [
        op::GAMESTATE,
        op::RAIL_MANAGER,
        op::ACTIVE_ROOT,
        op::SELECTOR_MAP,
        op::SELECTOR_TABLE,
        op::RAIL_TABLE,
        op::ACTIVE_TABLE,
        op::SOURCE_ARRAY,
        op::MAP_ARRAY,
        op::IDLER_SLOT,
        op::IDLER_SLOT_B,
        op::IDLER,
        op::OWNER,
        op::CONTAINER,
    ] {
        mods[op as usize].samples = SampleSpec {
            count: D81_CLOSING_IMAGES_HELD as u8,
            on_disagree: DisagreeAction::Record,
        };
    }
    // THE ACTIVE TABLE TAKES THREE, because its reader takes three (Codex
    // 019feb7d, finding 3). `d81_read_coherent_active_entry` observes the
    // seven-byte window up to `D81_ACTIVE_COHERENCE_READS` times and publishes
    // the first CONSECUTIVE PAIR that agrees, so a live sequence of A, B, B
    // publishes B. With two images the third attempt has nothing to read and the
    // walk reports `Torn` for a window the live reader accepted.
    mods[op::ACTIVE_TABLE as usize].samples = SampleSpec {
        count: super::D81_ACTIVE_COHERENCE_READS as u8,
        on_disagree: DisagreeAction::Record,
    };

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
    // NO CLOSING PROGRAM AND NO WITNESS WAVE: every closing image the walk
    // consumes is the vector pipeline's, taken at the points the function
    // header names. The substrate's closing wave would re-copy megabytes of
    // table nobody compares (all of d81's asks declare `Record`, so its
    // blanket comparison gapped nothing even when it ran).
    recipe.with_mods(mods)
}

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
#[derive(Default)]
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
/// GENERIC OVER THE INNER MEMORY so the gate can count a staged world's copies
/// on the same terms production counts the process's. A counter that only
/// existed on the production path would leave the offline gate unable to assert
/// the transaction ceiling the plan's whole shape is argued from.
pub struct CountingSnapshotMemory<M = Domain81SnapshotMemory> {
    inner: M,
    calls: std::cell::Cell<u64>,
}

impl<M: Default> Default for CountingSnapshotMemory<M> {
    fn default() -> Self {
        Self::new(M::default())
    }
}

impl<M> CountingSnapshotMemory<M> {
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            calls: std::cell::Cell::new(0),
        }
    }

    pub fn calls(&self) -> u64 {
        self.calls.get()
    }
}

impl<M: SnapshotMemory> SnapshotMemory for CountingSnapshotMemory<M> {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.inner.copy_into(addr, out)
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
// THE READ SOURCE: one walk body, two places its bytes can come from
// ===========================================================================

/// Which of the copier's images a read takes its bytes from.
///
/// The live walk reads the same address more than once on purpose — a stable
/// pair, an adjacent A/B sample, a closing re-read — and each of those is a
/// DIFFERENT instant. A frozen copy has no instants, it has IMAGES, and the
/// substrate takes exactly the three the walk distinguishes. Naming the image
/// at the call site is what keeps the walk's own comparisons meaningful instead
/// of tautological: an opening read compared against a closing read is still
/// comparing two observations, taken by the copier rather than by the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum D81Image {
    /// The copy the observation's forward wave took.
    Opening,
    /// The `k`-th ADJACENT sample (`k >= 1`), taken back to back with the
    /// opening copy.
    Sample(usize),
    /// The FIRST image the closing wave took, after every forward wave — for a
    /// mid-walk stage, at the end of the wave it declared.
    Closing,
    /// The `k`-th ADJACENT closing observation (`k >= 1`), taken back to back
    /// with [`Self::Closing`].
    ///
    /// The closing side of [`Self::Sample`], and the instrument the walk renders
    /// its own closing torn verdict from. A closing read is an observation like
    /// any other, and the live walk took two of them at every closing site.
    ClosingSample(usize),
}

/// Adjacent observations this plan asks for at EACH instant.
///
/// It is one number because it is one decision: the opening wave takes this
/// many and, since the owner ruling of 2026-08-11, so does the closing wave.
/// The plan's `SampleSpec` is declared FROM it, so changing it changes what the
/// copier does rather than only what this file says about it -- an earlier
/// version was a restatement that no copy count depended on, which is a
/// constant a test can assert and a mutation cannot reach.
///
/// AS MANY AS THE OPENING SIDE TAKES (owner ruling 2026-08-11): two for a stable
/// pair and [`super::D81_ACTIVE_COHERENCE_READS`] for the active window, because
/// a closing read is an observation like any other and the live walk took two of
/// them at every closing site. With one image the walk's closing A/B compared
/// that image with itself and could never tear, which published where the live
/// walk refused.
///
/// It is a named constant rather than an assumption so that the mapping in
/// [`D81ReadSource::paired_image`] and the number the substrate actually takes
/// cannot drift apart without a gate failing.
pub const D81_CLOSING_IMAGES_HELD: usize = 2;

/// Ordinals for [`D81ViewMisses::first_class`]. A WIRE vocabulary: a new class
/// appends and an existing number never moves.
pub const D81_MISS_UNREADABLE: u32 = 1;
pub const D81_MISS_NOT_CAPTURED: u32 = 2;
pub const D81_MISS_NOT_OWNED: u32 = 3;
pub const D81_MISS_NOT_CONTIGUOUS: u32 = 4;
pub const D81_MISS_NO_CLOSING_IMAGE: u32 = 5;

/// Why reads of the hour's copy could not answer, in the classes that mean
/// DIFFERENT things to a capture.
///
/// The split is the whole point. [`Self::unreadable`] is ruling A working: the
/// plan asked for the span, the engine refused it, the void is recorded and
/// only the reads that touch it fail — so the reader raises exactly the typed
/// failure an unreadable LIVE read raised, at the same stage, subject and
/// index. Every other class is OUR defect: bytes the reader consumes that the
/// plan did not ask for, did not own, or could not hand over in one piece. Those
/// refuse the frame too, but they are a conversion bug and not an engine
/// condition, and a capture that could not tell them apart would read a plan
/// that under-copies as a game that went unreadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81ViewMisses {
    /// RULING A. The plan asked; the engine refused; the void names it.
    pub unreadable: u32,
    /// OUR DEFECT: the reader consumed bytes the plan never asked for.
    pub not_captured: u32,
    /// OUR DEFECT: copied, but by transactions this consumer does not own.
    pub not_owned: u32,
    /// OUR DEFECT: copied, but split across transactions a single borrow cannot
    /// span. [`SnapshotView::read_into`] stitches, so this can only reach a call
    /// site that needs one contiguous slice — the sampled and closing images.
    pub not_contiguous: u32,
    /// OUR DEFECT: the reader closed over a span whose closing image the
    /// observation never took.
    pub no_closing_image: u32,
    /// A requested adjacent image the observation does not hold.
    ///
    /// AN ENGINE CONDITION, NOT A SUBSTITUTION (Codex 019feb7d, finding 1). An
    /// earlier version served the opening image in its place, which made every
    /// walk-side A/B pair a comparison of one image with itself and published
    /// exactly the transients the live second read refused on. The plan asks for
    /// two adjacent images of everything (three of the active table), so a
    /// missing one means the engine refused the adjacent copy — which is what
    /// the live walk's second read would have refused on — and the read fails
    /// here as it failed there.
    ///
    /// It is counted apart from [`Self::unreadable`] because the SPAN was
    /// readable and only its second observation was not, which is a different
    /// thing for a capture to see.
    pub samples_absent: u32,
    /// The FIRST miss, so a capture can name one rather than count many.
    pub first_class: u32,
    pub first_addr: u64,
    pub first_len: u32,
    /// The LAST miss, which is the one that ended the walk.
    ///
    /// NOT THE SAME WITNESS AS THE FIRST, and the difference decides a refusal's
    /// name. A read that misses returns `None` at its own site and the walk
    /// refuses there, so the miss recorded LAST is the one the published refusal
    /// belongs to; the first is the capture's earliest evidence and may belong to
    /// a read that recovered. `d81_restamp_plan_defect` renames from this one —
    /// renaming from the first would let an earlier, harmless plan defect
    /// relabel a later genuine void, and vice versa.
    pub last_class: u32,
    pub last_addr: u64,
}

impl D81ViewMisses {
    /// Misses that mean the plan is wrong, as opposed to the engine being
    /// unreadable.
    pub fn plan_defects(&self) -> u32 {
        self.not_captured
            .saturating_add(self.not_owned)
            .saturating_add(self.not_contiguous)
            .saturating_add(self.no_closing_image)
    }

    pub fn total(&self) -> u32 {
        self.unreadable.saturating_add(self.plan_defects())
    }
}

/// Where d81's province walk takes its bytes from.
///
/// # The two arms are the same walk
///
/// `read_compact_rail_replay_state`'s per-province loop moved into
/// `d81_walk_provinces`, which reads through this and nothing else. The LIVE
/// arm calls the identical guarded primitives the loop called before, so the
/// retired path is byte-for-byte what it was and the equality lever can still
/// run it. The SNAPSHOT arm serves the same addresses out of the hour's copy.
/// One body, so a change to the walk cannot apply to one arm and not the other
/// — which is the drift the shadow's private field list could not rule out.
///
/// # There is no fallback, and that is deliberate
///
/// A snapshot read that cannot be served returns `None` at exactly the site a
/// live read would have, and the walk raises its own typed failure there. It
/// does NOT re-read the engine. A hidden live re-read would put bytes from
/// outside the labelled bracket into a published row — the hour-misattribution
/// rule — and it would make the plan's coverage unfalsifiable, because a plan
/// that copied nothing would still publish perfect rows.
pub enum D81ReadSource<'a> {
    /// The live process, through the same guarded primitives as before.
    Live,
    /// The hour's copy. Nothing reachable from here can read the game.
    Snapshot(D81SnapshotSource<'a>),
    /// A STAGED WORLD, and it exists so the conversion has an offline gate.
    ///
    /// [`Self::Live`] can only read the game, so the walk it drives cannot be
    /// run against a fixture — which is exactly why lane GC could not compare a
    /// converted reader against the live one and left the switch off. This arm
    /// reads a staged world DIRECTLY, byte for byte, with no plan between it and
    /// the memory, so `d81_walk_provinces` can be run twice over one world — once
    /// through this and once through the copy the plan took of it — and the rows
    /// compared. That is `the_authoritative_walk_publishes_what_the_live_walk_
    /// publishes`, and it is a claim about the SHIPPED walk because it is the
    /// shipped walk on both arms.
    ///
    /// Test-only, and it has to be: a production build that could read anything
    /// but the game and the hour's copy would have a third read path no capture
    /// ever runs.
    #[cfg(test)]
    Staged(&'a dyn crate::snapshot_recipes::SnapshotMemory),
}

/// The snapshot arm's state: the vector capture, the bulk view, and what the
/// two could not answer.
pub struct D81SnapshotSource<'a> {
    view: crate::snapshot_recipes::view::SnapshotView<'a>,
    /// The scattered families (lane HS). Consulted FIRST: an address inside a
    /// family window is that family's to answer, and only addresses no family
    /// covers fall through to the bulk view. `None` only when the observation
    /// refused before the pipeline ran, and then the walk refuses at its first
    /// scattered read exactly as it refused when the recipe's branch was
    /// gapped.
    vector: Option<&'a super::d81_vector_store::D81VectorCapture>,
    misses: std::cell::Cell<D81ViewMisses>,
}

impl<'a> D81ReadSource<'a> {
    pub fn snapshot(
        view: crate::snapshot_recipes::view::SnapshotView<'a>,
        vector: Option<&'a super::d81_vector_store::D81VectorCapture>,
    ) -> Self {
        D81ReadSource::Snapshot(D81SnapshotSource {
            view,
            vector,
            misses: std::cell::Cell::new(D81ViewMisses::default()),
        })
    }

    /// True when this source is the hour's copy rather than the live process.
    ///
    /// THE WALK ASKS THIS ONCE, at the rail header's fallback, and the site says
    /// why. Everywhere else the walk cannot tell which source it is on, which is
    /// what keeps the two arms one body.
    pub fn is_snapshot(&self) -> bool {
        matches!(self, D81ReadSource::Snapshot(_))
    }

    pub fn misses(&self) -> D81ViewMisses {
        match self {
            D81ReadSource::Snapshot(source) => source.misses.get(),
            // A direct read has no copy to miss: every refusal it produces is
            // the memory itself refusing, and the walk has already named it.
            _ => D81ViewMisses::default(),
        }
    }

    /// Fill `out` from `addr` out of `image`, or say it could not.
    ///
    /// The one primitive: every accessor below is this plus a decode, which is
    /// what keeps the two arms from acquiring different coverage.
    pub fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {
        match self {
            // The image is a live re-read, which is what the walk did at each of
            // these sites before: three reads of one address at three instants.
            D81ReadSource::Live => super::read_into(addr, out),
            D81ReadSource::Snapshot(source) => source.read_into(addr, out, image),
            // Same rule as the live arm, for the same reason: a direct read has
            // no images, only later reads, and a staged world does not change
            // between them.
            #[cfg(test)]
            D81ReadSource::Staged(memory) => memory.copy_into(addr as u64, out).is_ok(),
        }
    }

    pub fn scalar<const N: usize>(&self, addr: usize, image: D81Image) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        self.read_into(addr, &mut out, image).then_some(out)
    }

    pub fn u64_at(&self, addr: usize, image: D81Image) -> Option<u64> {
        self.scalar::<8>(addr, image).map(u64::from_le_bytes)
    }

    pub fn u32_at(&self, addr: usize, image: D81Image) -> Option<u32> {
        self.scalar::<4>(addr, image).map(u32::from_le_bytes)
    }

    pub fn u8_at(&self, addr: usize, image: D81Image) -> Option<u8> {
        self.scalar::<1>(addr, image).map(|byte| byte[0])
    }

    /// The second observation a stable pair takes.
    ///
    /// An OPENING pair is the opening image and the copier's first adjacent
    /// sample: the same instrument, taken by the copier instead of by the walk.
    ///
    /// # A CLOSING pair is a REAL pair
    ///
    /// The copier takes [`D81_CLOSING_IMAGES_HELD`] closing observations of every
    /// interval — and [`super::D81_ACTIVE_COHERENCE_READS`] of the active window
    /// — so a closing pair compares two adjacent observations exactly as an
    /// opening pair does. That is tear-detection PARITY with the live walk at
    /// every closing site, and it is what the owner funded on 2026-08-11.
    ///
    /// WHAT THAT BUYS, and each of the three is a scenario the single-image
    /// version published where the live walk refused:
    ///
    ///  * A WRITE BETWEEN THE TWO ADJACENT CLOSING READS is now seen. The two
    ///    closing images differ, the walk's own pair check refuses `Torn`, and
    ///    the live walk refuses `Torn` for the same write.
    ///  * A SECOND CLOSING READ THAT WENT UNREADABLE now refuses with its honest
    ///    reason. The copier asked for that observation and the engine declined
    ///    it, so [`crate::snapshot_recipes::view::SnapshotView::closing_sample_bytes`]
    ///    answers `Unreadable` rather than `NotCaptured`, and the walk raises the
    ///    same `Unreadable` the live second read raised — not a plan defect,
    ///    because the plan asked.
    ///  * THE ACTIVE CLOSING LOOP coheres over its own three observations. A
    ///    closing sequence `A, B, B` coheres on `B` and the closure against
    ///    opening `A` refuses, which is the live verdict for the same sequence.
    ///
    /// WHAT REMAINS, and it is not a closing-side gap: the copier takes its
    /// images at ITS instants and the live walk took its during the walk, so a
    /// field the engine rewrites inside the bracket can be published with
    /// different CONTENT by the two arms. That is the sampling-window difference
    /// lane FY named, it is inherent to copy-only brackets, and it is what the
    /// equality mode exists to MEASURE rather than something any image count
    /// rules out.
    pub fn paired_image(image: D81Image) -> D81Image {
        match image {
            D81Image::Opening => D81Image::Sample(1),
            D81Image::Closing => D81Image::ClosingSample(1),
            // A pair OF a sample is that sample: the walk never nests one, and
            // returning the next index would silently reach past what the
            // sequence holds.
            other => other,
        }
    }

    /// `read_u64_stable`, expressed once for both arms.
    ///
    /// Live, this is a read and a second read of the same address, compared —
    /// verbatim what `read_u64_stable` does, at both images, because a live
    /// image IS just a later read.
    pub fn u64_stable_at(&self, addr: usize, image: D81Image) -> Option<u64> {
        let first = self.u64_at(addr, image)?;
        let second = self.u64_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u32_stable_at(&self, addr: usize, image: D81Image) -> Option<u32> {
        let first = self.u32_at(addr, image)?;
        let second = self.u32_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u8_stable_at(&self, addr: usize, image: D81Image) -> Option<u8> {
        let first = self.u8_at(addr, image)?;
        let second = self.u8_at(addr, Self::paired_image(image))?;
        (first == second).then_some(first)
    }

    pub fn u64_stable(&self, addr: usize) -> Option<u64> {
        self.u64_stable_at(addr, D81Image::Opening)
    }

    pub fn u32_stable(&self, addr: usize) -> Option<u32> {
        self.u32_stable_at(addr, D81Image::Opening)
    }

    pub fn u8_stable(&self, addr: usize) -> Option<u8> {
        self.u8_stable_at(addr, D81Image::Opening)
    }
}

impl D81SnapshotSource<'_> {
    fn note(&self, class: u32, span: crate::snapshot_recipes::Span) {
        let mut misses = self.misses.get();
        match class {
            D81_MISS_UNREADABLE => misses.unreadable = misses.unreadable.saturating_add(1),
            D81_MISS_NOT_CAPTURED => misses.not_captured = misses.not_captured.saturating_add(1),
            D81_MISS_NOT_OWNED => misses.not_owned = misses.not_owned.saturating_add(1),
            D81_MISS_NOT_CONTIGUOUS => {
                misses.not_contiguous = misses.not_contiguous.saturating_add(1)
            }
            _ => misses.no_closing_image = misses.no_closing_image.saturating_add(1),
        }
        if misses.first_class == 0 {
            misses.first_class = class;
            misses.first_addr = span.start;
            misses.first_len = span.len.min(u64::from(u32::MAX)) as u32;
        }
        misses.last_class = class;
        misses.last_addr = span.start;
        self.misses.set(misses);
    }

    fn note_absent_sample(&self) {
        let mut misses = self.misses.get();
        misses.samples_absent = misses.samples_absent.saturating_add(1);
        self.misses.set(misses);
    }


    /// Fold a view refusal into a miss class. `ForeignHour` and `ConsumerGapped`
    /// cannot reach here — a view that exists is this hour's and is not gapped —
    /// and are folded to `NotCaptured` rather than given a class of their own,
    /// because a class nothing can produce is a wire word nobody can interpret.
    fn class_of(refusal: crate::snapshot_recipes::view::ViewRefusal) -> u32 {
        use crate::snapshot_recipes::view::ViewRefusal as R;
        match refusal {
            R::Unreadable { .. } => D81_MISS_UNREADABLE,
            R::NotOwned { .. } => D81_MISS_NOT_OWNED,
            R::NotContiguous { .. } => D81_MISS_NOT_CONTIGUOUS,
            R::NotCaptured { .. } | R::ForeignHour { .. } | R::ConsumerGapped(_) => {
                D81_MISS_NOT_CAPTURED
            }
        }
    }

    fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {
        let span = crate::snapshot_recipes::Span::new(addr as u64, out.len() as u64);
        // THE VECTOR FAMILIES ANSWER FIRST (lane HS). Their windows are the
        // scattered objects the recipe's branch used to copy; the bulk view
        // has never owned those bytes on this tip, so serving them here is a
        // routing fact and not a preference. `NotCovered` — the only answer
        // that falls through — means no family window contains the span, and
        // then the view's own verdict stands for the bulk tables it copies.
        if let Some(vector) = self.vector {
            use super::d81_vector_store::{VectorAnswer, VectorImage};
            let asked = match image {
                D81Image::Opening => VectorImage::Opening,
                D81Image::Sample(index) => VectorImage::Sample(index),
                D81Image::Closing => VectorImage::Closing,
                D81Image::ClosingSample(index) => VectorImage::ClosingSample(index),
            };
            match vector.read(addr as u64, out.len() as u32, asked) {
                VectorAnswer::Served(bytes) => {
                    out.copy_from_slice(bytes);
                    return true;
                }
                VectorAnswer::NotCovered => {}
                VectorAnswer::Unreadable => {
                    self.note(D81_MISS_UNREADABLE, span);
                    return false;
                }
                VectorAnswer::SampleAbsent => {
                    // An OPENING adjacent sample the engine refused keeps its
                    // own class, exactly as `sample_bytes` gave it. A refused
                    // CLOSING observation was `Unreadable` under
                    // `closing_sample_bytes` — the engine declined an
                    // observation the plan asked for — and it keeps that
                    // class here so the wire counters do not move.
                    match image {
                        D81Image::Closing | D81Image::ClosingSample(_) => {
                            self.note(D81_MISS_UNREADABLE, span);
                        }
                        _ => self.note_absent_sample(),
                    }
                    return false;
                }
                VectorAnswer::Aliased => {
                    // Two scattered families' windows contain the span: a
                    // malformed engine pointer aliased two objects, and no
                    // byte is served from it. A PLAN-coverage refusal, so it
                    // publishes in the defect class and the frame refuses
                    // fail-closed.
                    self.note(D81_MISS_NOT_CAPTURED, span);
                    return false;
                }
                VectorAnswer::NeverObserved => {
                    // The plan never asked for this image of these bytes: a
                    // PLAN DEFECT, in the class that names it — the closing
                    // class for a closing image, the coverage class otherwise
                    // — never a silent substitution.
                    let class = match image {
                        D81Image::Closing | D81Image::ClosingSample(_) => {
                            D81_MISS_NO_CLOSING_IMAGE
                        }
                        _ => D81_MISS_NOT_CAPTURED,
                    };
                    self.note(class, span);
                    return false;
                }
            }
        }
        match image {
            D81Image::Opening => match self.view.read_into(addr as u64, out) {
                Ok(()) => true,
                Err(refusal) => {
                    self.note(Self::class_of(refusal), span);
                    false
                }
            },
            D81Image::Sample(index) => {
                match self.view.sample_bytes(addr as u64, out.len(), index) {
                    Ok(bytes) => {
                        out.copy_from_slice(bytes);
                        true
                    }
                    // A REQUESTED IMAGE THE OBSERVATION DOES NOT HOLD, and it
                    // FAILS rather than being substituted (Codex 019feb7d,
                    // finding 1). Every copy op of this plan asks for its
                    // adjacent images, so reaching here means the engine refused
                    // the adjacent copy — the same condition the live walk's
                    // second read refuses on, at the same site, with the same
                    // typed failure. Serving the opening image instead would
                    // make the walk's pair check a comparison of one image with
                    // itself and publish the transients it exists to catch.
                    Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {
                        self.note_absent_sample();
                        false
                    }
                    Err(refusal) => {
                        self.note(Self::class_of(refusal), span);
                        false
                    }
                }
            }
            // THE CLOSING SEQUENCE, image by image. Image 0 is the first
            // closing observation and `ClosingSample(k)` the k-th adjacent one,
            // which is what gives the walk's closing A/B two observations to
            // compare instead of one image against itself.
            D81Image::Closing | D81Image::ClosingSample(_) => {
                let index = match image {
                    D81Image::ClosingSample(index) => index,
                    _ => 0,
                };
                match self.view.closing_sample_bytes(addr as u64, out.len(), index) {
                    Ok(bytes) => {
                        out.copy_from_slice(bytes);
                        true
                    }
                    // THE ENGINE DECLINED AN OBSERVATION THE PLAN ASKED FOR.
                    // `closing_sample_bytes` answers `Unreadable` for exactly
                    // that, and `NotCaptured` only when nobody asked — so this
                    // falls through to the class mapping below and is counted as
                    // the engine condition it is, never as a plan defect. It is
                    // the live walk's own second closing read going unreadable,
                    // refusing at the same site with the same reason.
                    //
                    // AN ABSENT CLOSING IMAGE NOBODY ASKED FOR is still a plan
                    // defect: the reader closed over a span outside the plan's
                    // own closing set.
                    Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {
                        self.note(D81_MISS_NO_CLOSING_IMAGE, span);
                        false
                    }
                    Err(refusal) => {
                        self.note(Self::class_of(refusal), span);
                        false
                    }
                }
            }
        }
    }
}

// ===========================================================================
// THE PLAN'S OWN ACCOUNTING — its BLOCK is on every frame, and which frames
// MEASURED the block they publish is its own word on the wire
// ===========================================================================

/// What one frame's AUTHORITATIVE observation cost and produced.
///
/// The block is emitted on every outcome, armed or not, because this is now the
/// read path and not an experiment: `plan_us` is the whole of what the
/// conversion moved off the critical sweep, and a capture that could not see it
/// could not attribute the sweep-length change to anything.
///
/// PUBLICATION IS NOT PRESERVATION, and the difference used to be most of a
/// capture. A rejected bracket discards the observation that filled these in — a
/// TORN one drops the whole observed outcome in the worker, a LABEL MISMATCH
/// hands it over and a deferred one is dropped — and the block those frames
/// published was then default-constructed. In t49 that was 317 of 334 frames
/// carrying zeros they did not measure.
///
/// LANE GM CARRIES THEM ACROSS BOTH DISCARDS (2026-08-11): the counters are
/// copied off the outcome before it dies and published beside the refusal, which
/// is a change to PROVENANCE and not to the refusal — no row from a refused
/// bracket reaches the wire, and none of the words below decides anything.
/// Which of the two a record carries is
/// `D81DiagnosticsProvenance`, published as `d81_diagnostics_measured_frames` /
/// `_preserved_frames` / `_substituted_frames`. It is NOT `ran`: that means a
/// view was ISSUED, so a custody refusal that measured `plan_us` perfectly well
/// also reports false. See `runs/gj_crit_doubling/FINDINGS.txt`, residual R3.
///
/// Every field is per-frame and lands on the wire as its own diagnostic unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81PlanCounters {
    /// The plan ran to a view. False means the observation refused, custody
    /// declined it, or the consumer was gapped — and the words below say which.
    pub ran: bool,
    /// Guarded copy transactions the substrate issued, INCLUDING the ones that
    /// failed: a bisected scoped copy's retries and an adjacent sample are both
    /// work the engine did.
    pub copies_issued: u64,
    pub intervals_closed: u32,
    pub scoped_voids: u32,
    pub pointers_rejected: u32,
    /// Custody declined the observation before it began.
    pub custody_refused: bool,
    /// The observation ran but the substrate would not issue a view — a changed
    /// witness, an unmet sample, or the refusal below. Its own word, because
    /// "not ran, no refusal reason" is an inference from absences rather than a
    /// statement.
    pub consumer_gapped: bool,
    pub refused: bool,
    /// [`d81_plan_refusal_ordinal`], zero when the recipe captured.
    pub refusal_ordinal: u32,
    /// [`d81_plan_gap_ordinal`], zero when the consumer was not gapped. WHICH
    /// gap it was is the difference between "the engine moved under the copier"
    /// and "the recipe refused", and a capture that could only see
    /// `consumer_gapped` could not tell them apart.
    pub gap_ordinal: u32,
    pub custody_held_bytes: u64,
    /// THE NUMBER THE CONVERSION IS ABOUT: wall spent inside the bracket
    /// building the copy — root formation excluded, because those fifteen reads
    /// are charged to `roots_us` exactly as they were before.
    pub plan_us: u64,
    /// What the compute half's reads could not get out of the copy, by class.
    /// See [`D81ViewMisses`]: `unreadable` is the engine, everything else is us.
    pub misses: D81ViewMisses,
}

/// A refusal as a stable small ordinal, so the wire can carry WHICH refusal
/// without carrying a string.
///
/// The order is the `Refusal` enum's own declaration order and it is a WIRE
/// vocabulary: a new variant appends, and an existing number never moves.
/// Zero is reserved for "no refusal".
pub fn d81_plan_refusal_ordinal(refusal: &crate::snapshot_recipes::Refusal) -> u32 {
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
pub fn d81_plan_refusal_unit_name(ordinal: u32) -> &'static str {
    match ordinal {
        1 => "d81_plan_refusal_01_depth_exceeded",
        2 => "d81_plan_refusal_02_copy_task_limit",
        3 => "d81_plan_refusal_03_range_limit",
        4 => "d81_plan_refusal_04_byte_limit",
        5 => "d81_plan_refusal_05_range_too_wide",
        6 => "d81_plan_refusal_06_extent_count",
        7 => "d81_plan_refusal_07_extent_bytes",
        8 => "d81_plan_refusal_08_join_out_of_range",
        9 => "d81_plan_refusal_09_join_table_missing",
        10 => "d81_plan_refusal_10_field_not_captured",
        11 => "d81_plan_refusal_11_memory_unreadable",
        12 => "d81_plan_refusal_12_address_not_representable",
        13 => "d81_plan_refusal_13_pointer_not_admissible",
        14 => "d81_plan_refusal_14_carry_not_set",
        15 => "d81_plan_refusal_15_conditional_unresolved",
        16 => "d81_plan_refusal_16_closed_interval_reopened",
        17 => "d81_plan_refusal_17_malformed",
        _ => "d81_plan_refusal_00_unknown",
    }
}

/// The gap a view refusal names, as a small ordinal. Zero is "no gap".
///
/// A WIRE vocabulary on the same terms as the refusal ordinals above.
pub fn d81_plan_gap_ordinal(refusal: &crate::snapshot_recipes::view::ViewRefusal) -> u32 {
    use crate::snapshot_recipes::view::ViewRefusal as V;
    use crate::snapshot_recipes::GapReason as G;
    match refusal {
        V::ForeignHour { .. } => 1,
        V::ConsumerGapped(G::Refused(_)) => 2,
        V::ConsumerGapped(G::WitnessChanged { .. }) => 3,
        V::ConsumerGapped(G::WitnessDependency { .. }) => 4,
        V::ConsumerGapped(G::SamplesDisagreed { .. }) => 5,
        V::NotCaptured { .. } => 6,
        V::NotContiguous { .. } => 7,
        V::NotOwned { .. } => 8,
        V::Unreadable { .. } => 9,
    }
}

pub fn d81_plan_gap_unit_name(ordinal: u32) -> &'static str {
    match ordinal {
        1 => "d81_plan_gap_01_foreign_hour",
        2 => "d81_plan_gap_02_recipe_refused",
        3 => "d81_plan_gap_03_witness_changed",
        4 => "d81_plan_gap_04_witness_dependency",
        5 => "d81_plan_gap_05_samples_disagreed",
        6 => "d81_plan_gap_06_not_captured",
        7 => "d81_plan_gap_07_not_contiguous",
        8 => "d81_plan_gap_08_not_owned",
        9 => "d81_plan_gap_09_unreadable",
        _ => "d81_plan_gap_00_unknown",
    }
}

// ===========================================================================
// THE REVERSE SHADOW: the retired live walk, run beside the authoritative one
// ===========================================================================

/// The environment spelling of the equality lever's on switch.
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
/// # THE LEVER TURNED AROUND (lane GI, 2026-08-10)
///
/// It used to mean "run the copy plan beside the live reader". The plan IS the
/// reader now, so it means the other thing: run the RETIRED live walk beside the
/// authoritative one and compare the rows they publish. The authoritative rows
/// are what the frame publishes either way — the live arm's rows are never
/// published, only compared — so an armed capture cannot deliver a row a
/// disarmed one would not.
///
/// # WHAT ARMING COSTS THE FRAME, and it is the whole retired walk
///
/// The live arm reads the engine, so it runs INSIDE the clock bracket, and it is
/// the ~105,000 guarded calls this conversion removed. An armed capture's
/// bracket is therefore about as long as a pre-conversion one's and its DELIVERY
/// is not comparable to a disarmed capture's. Its EQUALITY numbers are what it
/// is for, and those are unaffected: both arms observe the same hour, the live
/// arm inside the bracket and the copy taken inside the same bracket.
///
/// The disarmed path pays one relaxed atomic load per frame — the environment
/// lever is folded into the same atomic at session install — and nothing else:
/// no second walk, no allocation, no engine read, and
/// [`D81EqualityCounters::armed`] false publishes no equality unit at all.
pub const D81_COPY_BRACKET_SHADOW_ENV: &str = "BATTLE_PLAN_D81_COPY_BRACKET_SHADOW";

/// The row fields the equality mode compares, in ordinal order, AS WIRE UNIT
/// NAMES.
///
/// Ordinal 0 is "no divergence". One table and not two — a vocabulary and a
/// separate list of unit names for it drift, which is how a capture ends up
/// with a field ordinal nobody can decode. The frame publishes the NAME, so
/// "which field disagreed" is a count against a word rather than a number a
/// reader has to look up.
pub const D81_SHADOW_FIELD_UNITS: [&str; 7] = [
    "d81_shadow_divergence_00_none",
    "d81_shadow_divergence_01_row_present",
    "d81_shadow_divergence_02_province_id",
    "d81_shadow_divergence_03_active_terminal",
    "d81_shadow_divergence_04_anchor_variant",
    "d81_shadow_divergence_05_anchor_bits",
    "d81_shadow_divergence_06_neighbors",
];

/// The unit name a divergence ordinal publishes under, saturating at "unknown"
/// so a future field cannot publish under another field's name.
pub fn d81_shadow_divergence_unit_name(ordinal: u32) -> &'static str {
    D81_SHADOW_FIELD_UNITS
        .get(ordinal as usize)
        .copied()
        .unwrap_or("d81_shadow_divergence_99_unknown")
}

/// What one equality frame observed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81EqualityCounters {
    /// The lever was on for this frame.
    pub armed: bool,
    /// Both arms produced a verdict, so the comparison below happened. False
    /// with `armed` true means the authoritative arm had no rows to compare
    /// against — the plan refused or the consumer was gapped — and
    /// [`D81PlanCounters`] says which.
    pub ran: bool,
    /// Rows the retired live walk published.
    pub rows_live: u32,
    /// Rows the authoritative walk published.
    pub rows_authoritative: u32,
    /// Rows compared: the longer of the two, so a row present on one side and
    /// absent on the other is a DIVERGENCE and never a skipped comparison.
    pub rows_compared: u32,
    pub rows_equal: u32,
    pub rows_divergent: u32,
    /// The FIRST divergence's province and field ordinal. Zero when there was
    /// none; the field ordinal indexes [`D81_SHADOW_FIELD_UNITS`].
    pub first_divergence_province: u32,
    pub first_divergence_field: u32,
    /// The two arms agreed about WHETHER the frame refused, and about the
    /// refusing stage and reason when it did.
    pub failures_agree: bool,
    /// `(stage << 16) | reason` for each arm, zero when that arm did not refuse.
    /// Published so a disagreement names both sides rather than only its own.
    pub live_failure: u32,
    pub authoritative_failure: u32,
    /// Neighbour totals, which the row comparison would otherwise only reach
    /// through the per-province neighbour lists.
    pub live_neighbors: u32,
    pub authoritative_neighbors: u32,
    /// Wall the retired live walk cost inside the bracket. This is the number
    /// the conversion removed, measured on the same frame that measures what
    /// replaced it.
    pub live_walk_us: u64,
}

/// One arm's walk result, reduced to what the equality mode compares.
///
/// Borrowed rather than owned: the authoritative arm's rows are the ones the
/// frame PUBLISHES, and a comparison that took them by value would be a
/// comparison that could consume the publication.
pub struct D81WalkVerdict<'a, F> {
    pub provinces: &'a [compact_contract::rail_replay::RailProvinceState],
    pub observed_neighbors: usize,
    /// THE WHOLE REFUSAL, not a summary of it.
    ///
    /// `(stage << 16) | reason` is what the WIRE carries, because a diagnostic
    /// unit is one integer. It is not what the comparison may run on: two arms
    /// refusing at different provinces, indices, raw witnesses or read addresses
    /// are not in agreement, and a comparison over the packed word would call
    /// them equal. The type is generic so this module needs no knowledge of the
    /// reader's failure type beyond it being comparable.
    pub failure: Option<F>,
    /// The packed `(stage << 16) | reason` of `failure`, for the wire.
    pub failure_word: u32,
}

/// Compare the retired live walk's rows against the authoritative ones.
///
/// # What equality means here, exactly
///
/// Row `i` of one arm against row `i` of the other, over every field the wire
/// carries, with the LONGER list setting the count — so a row one arm published
/// and the other did not is `row_present`, ordinal 1, and never a comparison
/// that quietly did not happen. Then the two refusals: a frame both arms refused
/// on the same stage and reason agrees; anything else does not.
///
/// # It compares VALUES, not timing, and the difference is the measurement
///
/// The live arm read its fields during its own walk; the plan copied them at the
/// top of the same bracket. A field the engine rewrote in between diverges
/// legitimately. That is not noise to be tuned away — it is the sampling-window
/// difference between a walk and a copy, and it is the number a capture is run
/// to see.
pub fn d81_equality_compare<F: PartialEq>(
    live: &D81WalkVerdict<'_, F>,
    authoritative: &D81WalkVerdict<'_, F>,
    counters: &mut D81EqualityCounters,
) {
    counters.ran = true;
    counters.rows_live = live.provinces.len().min(u32::MAX as usize) as u32;
    counters.rows_authoritative = authoritative.provinces.len().min(u32::MAX as usize) as u32;
    counters.live_neighbors = live.observed_neighbors.min(u32::MAX as usize) as u32;
    counters.authoritative_neighbors =
        authoritative.observed_neighbors.min(u32::MAX as usize) as u32;
    counters.live_failure = live.failure_word;
    counters.authoritative_failure = authoritative.failure_word;
    // THE WHOLE VALUE. Same stage and reason at different provinces, indices or
    // witnesses is a divergence the conversion has to be told about, and the
    // packed word cannot see it.
    counters.failures_agree = live.failure == authoritative.failure;

    let rows = live.provinces.len().max(authoritative.provinces.len());
    for index in 0..rows {
        counters.rows_compared = counters.rows_compared.saturating_add(1);
        let (Some(left), Some(right)) = (
            live.provinces.get(index),
            authoritative.provinces.get(index),
        ) else {
            counters.rows_divergent = counters.rows_divergent.saturating_add(1);
            if counters.first_divergence_field == 0 {
                counters.first_divergence_field = 1;
                counters.first_divergence_province = live
                    .provinces
                    .get(index)
                    .or_else(|| authoritative.provinces.get(index))
                    .map(|row| row.province_id)
                    .unwrap_or(0);
            }
            continue;
        };
        // The EARLIEST field of this row that disagreed, not whichever the loop
        // noticed: the checks run in ordinal order and only the first sticks.
        let diverged = if left.province_id != right.province_id {
            2
        } else if left.active_terminal != right.active_terminal {
            3
        } else if left.anchor_variant != right.anchor_variant {
            4
        } else if left.anchor_bits != right.anchor_bits {
            5
        } else if left.neighbors != right.neighbors {
            6
        } else {
            0
        };
        if diverged == 0 {
            counters.rows_equal = counters.rows_equal.saturating_add(1);
        } else {
            counters.rows_divergent = counters.rows_divergent.saturating_add(1);
            if counters.first_divergence_field == 0 {
                counters.first_divergence_province = left.province_id;
                counters.first_divergence_field = diverged;
            }
        }
    }
}
