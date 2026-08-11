//! THE S4 GATE: can the snapshot-recipe substrate express d81's observation
//! phase, and does it produce the same rows?
//!
//! # What this file is, and what it deliberately is not
//!
//! It is NOT a conversion of `read_compact_rail_replay_state`. Lane FX priced
//! the conversion and proved nine things the substrate could not say; this file
//! is the acceptance test for the vocabulary that closes them. The question it
//! answers is expressibility: can the shapes FX staged be written as a recipe
//! PROGRAM, and does executing that program against a staged world produce the
//! same row-level outcomes — including the refusal paths — as a direct-read
//! walk of the same world.
//!
//! # The staged world
//!
//! Every stride, offset and formula comes from
//! [`super::d81_copy_bracket::shape`], which is FX's staging: the rail slot
//! width, the active-table stride, the pointer stride, the selector entry
//! stride, and the `mask + max_probe + 2` extent formula with its probe slack.
//! The POPULATION is scaled down and only the population: FX's census runs at
//! 13,413 provinces because it is pricing a frame, and this file is checking a
//! program, so it runs the same shapes over [`PROVINCES`] of them. Nothing about
//! which mechanism is exercised depends on the count, and the scaled count is
//! what keeps `SyntheticMemory`'s linear region scan out of the way of the
//! thing under test.
//!
//! # Why the two walkers are not the same function twice
//!
//! [`reference_walk`] reads the fixture's raw bytes directly, the way the live
//! reader reads the game. [`replay_walk`] runs the same logic over a
//! [`SnapshotView`] and can see NOTHING the recipe did not copy. So the two
//! agree only if the copy plan reaches every byte the walk consumes, keys every
//! join the same way, admits the same pointers, and scopes its failures the
//! same way. A recipe that under-copies does not produce a wrong number here —
//! it produces a `ViewRefusal`, and the row differs.
//!
//! # The nine mechanisms, and where each one is load-bearing below
//!
//! | mechanism | where |
//! |---|---|
//! | derived extent (`mask + max_probe + 2`) | the selector table, [`OP_SELECTOR_EXTENT`] |
//! | carried key across waves | the province id, carried as an ELEMENT INDEX from the rail table into three joins |
//! | per-instance table bindings | not needed by d81's walk; exercised in `vocab_tests` |
//! | pointer admissibility | the rail-info pointer, whose out-of-window value is a recorded rejection |
//! | adjacent samples | the rail header pair, `SampleSpec { count: 2 }` |
//! | rich refusal witnesses | the rejection's stage/subject/raw |
//! | RULING A over-read scoping | the unreadable page staged inside the active table |
//! | RULING B masks/order/elision | the rail header's dead bytes, the two-stage closing program, the elided source stage |
//! | RULING C conditional selection | the rail header's inline/out-of-line pair layout |

use super::*;

use crate::snapshot_recipes::executor::{
    observe, Admissibility, CarrySource, CloseStage, CloseWhen, ClosingProgram, CompareMask,
    CountExpr,
    CountTerm, DisagreeAction, ObservationConfig, OpMod, Probe, Recipe, RecipeOp, RecipeOutcome,
    RejectAction, RelWindow, Root, SampleSpec, Tolerance,
};
use crate::snapshot_recipes::synthetic::SyntheticMemory;
use crate::snapshot_recipes::view::{SnapshotView, ViewRefusal};
use crate::snapshot_recipes::{ConsumerId, HourStamp, RecipeId, Span, TableId};

use super::d81_copy_bracket::shape;

// ---------------------------------------------------------------------------
// The staged world
// ---------------------------------------------------------------------------

/// Provinces in the staged world. FX's 13,413 is a frame's population and this
/// is a program's; see the module header.
///
/// It is 512 and not 48 for one reason: ruling A's scoping is a PAGE-level
/// statement, and the active table has to span several pages before "one page
/// of it is unreadable" is a thing the fixture can say at all.
const PROVINCES: u32 = 512;
/// Every third province is railed, which is close to FX's 3,510-of-13,413 and
/// keeps the railless path — the `rail_slot == 0` skip — in the walk.
const RAIL_EVERY: u32 = 3;
/// The page granularity a scoped copy's failure narrows to.
const PAGE: u64 = crate::snapshot_recipes::SCOPED_VOID_GRANULARITY_BYTES;

const ROOT: u64 = 0x0010_0000;
const RAIL_TABLE: u64 = 0x0020_0000;
const ACTIVE_TABLE: u64 = 0x0030_0000;
const GS_ARRAY: u64 = 0x0040_0000;
const CONTAINER_ARRAY: u64 = 0x0050_0000;
const SELECTOR_MAP: u64 = 0x0060_0000;
const SELECTOR_TABLE: u64 = 0x0061_0000;
// The scattered per-province objects. Each family gets 0x0400_0000 of space and
// uses 512 * 0x0001_0000 = 0x0200_0000 of it, so no two families can overlap
// however the province count is raised.
const RAIL_INFO: u64 = 0x0400_0000;
const MAP_PROVINCE: u64 = 0x0800_0000;
const RECORD: u64 = 0x0C00_0000;
const PAIR_BLOB: u64 = 0x1000_0000;
const PROVINCE_OBJECT: u64 = 0x1400_0000;
const SCATTER_STRIDE: u64 = 0x0001_0000;

/// The heap window admissible pointers lie in. d81 admits a heap range plus
/// 8-byte alignment; this is that predicate at fixture scale.
const HEAP_LOW: u64 = 0x0020_0000;
const HEAP_HIGH: u64 = 0x2000_0000;
/// A readable, correctly aligned address OUTSIDE the heap window. This is the
/// case that matters: a value the substrate would happily follow and the reader
/// rejects.
const OUTSIDE_HEAP: u64 = 0x8000_0000;

/// The selector map's mask and probe depth, from which the derived extent's
/// count follows as `mask + max_probe + 2`.
///
/// The mask is `PROVINCES - 1` and every key sits at `(key & mask) + 2`, so the
/// TOP railed province's entry lands at slot `mask + 1` — past the masked
/// table. That is not decoration: it is the only thing that makes the derived
/// count observable, because a recipe that reads the mask alone copies
/// `mask + 1` slots and stops one short of it.
const SELECTOR_MASK: u32 = PROVINCES - 1;
const SELECTOR_MAX_PROBE: u8 = 2;
const SELECTOR_SLOTS: u32 = SELECTOR_MASK + 1 + SELECTOR_MAX_PROBE as u32 + 1;
/// Where every key sits relative to its masked slot.
const SELECTOR_PROBE_DISTANCE: u32 = 2;

/// Rail-header layout discriminant. 1 = the neighbour pair blob is INLINE in
/// the header; 0 = it is behind the pointer at `RAIL_PAIR_PTR`.
const RAIL_KIND_OFF: u64 = 0x00;
const RAIL_PAIR_COUNT_OFF: u64 = 0x04;
/// Four bytes of engine padding between the two fields the reader consumes.
/// Writing here must not gap the reader — this is the mask's whole job.
const RAIL_DEAD_OFF: u64 = 0x08;
const RAIL_PAIR_PTR_OFF: u64 = 0x10;
const RAIL_INLINE_PAIRS_OFF: u64 = 0x18;
const RAIL_HEADER_LEN: u32 = 0x28;

const MAP_RECORD_PTR_OFF: u64 = 0x11B8;
const MAP_ANCHOR_OFF: u64 = 0x0400;
const RECORD_SELECTOR_KEY_OFF: u64 = 0xC4;

/// Which province the fixture gives a rail pointer the reader must reject.
const REJECTED_PROVINCE: u32 = 9;
/// The page of the active table the fixture makes unreadable, as an offset from
/// its base. One page in, so there is a readable page either side of it.
const VOID_PAGE_START: u64 = PAGE;

/// True when this province's active-table entry lands in the voided page.
/// Ruling A says exactly these provinces refuse and every other one publishes.
fn voided(province: u32) -> bool {
    let at = u64::from(province) * shape::ACTIVE_STRIDE as u64;
    (VOID_PAGE_START..VOID_PAGE_START + PAGE).contains(&at)
}

fn railed(province: u32) -> bool {
    province % RAIL_EVERY == 0
}

fn rail_info_of(province: u32) -> u64 {
    RAIL_INFO + u64::from(province) * SCATTER_STRIDE
}

fn map_province_of(province: u32) -> u64 {
    MAP_PROVINCE + u64::from(province) * SCATTER_STRIDE
}

fn record_of(province: u32) -> u64 {
    RECORD + u64::from(province) * SCATTER_STRIDE
}

fn pair_blob_of(province: u32) -> u64 {
    PAIR_BLOB + u64::from(province) * SCATTER_STRIDE
}

fn province_object_of(province: u32) -> u64 {
    PROVINCE_OBJECT + u64::from(province) * SCATTER_STRIDE
}

/// Odd railed provinces carry their pairs out of line, even ones inline. Both
/// shapes exist in every run so the conditional has something to choose.
fn inline_pairs(province: u32) -> bool {
    province % 2 == 0
}

/// A key whose masked bucket is the province's own id, with bits above the
/// mask so the key itself is never mistakable for the id.
fn selector_key_of(province: u32) -> u32 {
    0x0001_0000 + province
}

/// The engine's own bucket for a key: `key & mask`, then a linear probe that
/// does NOT wrap. The fixture places every key at its masked slot plus a fixed
/// probe distance, so the tail past `mask` is genuinely used — which is why the
/// extent has to be `mask + max_probe + 2` long.
fn selector_slot_of(province: u32) -> u32 {
    (selector_key_of(province) & SELECTOR_MASK) + SELECTOR_PROBE_DISTANCE
}

fn selector_value_of(province: u32) -> u64 {
    0xA0000 + u64::from(province)
}

fn anchor_of(province: u32) -> [u32; 3] {
    [province, province + 100, province + 200]
}

/// The address space the two walkers share.
fn stage_world() -> SyntheticMemory {
    let mut memory = SyntheticMemory::new();

    memory.map_zeroed(ROOT, 0x40);
    memory.map_zeroed(RAIL_TABLE, PROVINCES as usize * shape::RAIL_SLOT);
    // The active table is mapped as THREE regions, of which the middle one is a
    // single page and is unreadable. That is the whole point of ruling A: a
    // fixture whose table could only refuse WHOLE would make "gap only what is
    // consumed" a statement about a set that is always everything.
    let active_bytes = PROVINCES as u64 * shape::ACTIVE_STRIDE as u64;
    assert!(
        active_bytes >= 3 * PAGE,
        "the fixture needs a readable page either side of the voided one",
    );
    memory.map_zeroed(ACTIVE_TABLE, VOID_PAGE_START as usize);
    memory.map_zeroed(ACTIVE_TABLE + VOID_PAGE_START, PAGE as usize);
    memory.map_zeroed(
        ACTIVE_TABLE + VOID_PAGE_START + PAGE,
        (active_bytes - VOID_PAGE_START - PAGE) as usize,
    );
    memory.map_zeroed(GS_ARRAY, PROVINCES as usize * shape::POINTER);
    memory.map_zeroed(CONTAINER_ARRAY, PROVINCES as usize * shape::POINTER);
    memory.map_zeroed(SELECTOR_MAP, 0x40);
    memory.map_zeroed(
        SELECTOR_TABLE,
        (SELECTOR_SLOTS as usize + 8) * shape::SELECTOR_STRIDE,
    );

    memory.write_u32(ROOT, PROVINCES);
    memory.write_u64(ROOT + 0x08, RAIL_TABLE);
    memory.write_u64(ROOT + 0x10, ACTIVE_TABLE);
    memory.write_u64(ROOT + 0x18, GS_ARRAY);
    memory.write_u64(ROOT + 0x20, CONTAINER_ARRAY);
    memory.write_u64(ROOT + 0x28, SELECTOR_MAP);

    memory.write_u64(SELECTOR_MAP + 0x08, SELECTOR_TABLE);
    memory.write_u32(SELECTOR_MAP + 0x34, SELECTOR_MASK);
    memory.write(SELECTOR_MAP + 0x38, &[SELECTOR_MAX_PROBE]);

    // Every slot is OCCUPIED (marker 1) and holds a key no province owns, so a
    // probe walks its full distance instead of stopping at the first empty
    // slot. That is the engine's own shape: `select_secondary` stops on the
    // terminating marker, and a table with holes in it would never exercise the
    // probe tail this fixture exists to reach.
    for slot in 0..SELECTOR_SLOTS + 8 {
        let entry = SELECTOR_TABLE + u64::from(slot) * shape::SELECTOR_STRIDE as u64;
        memory.write(entry + 4, &[1]);
        memory.write_u32(entry + 8, u32::MAX);
    }

    for province in 0..PROVINCES {
        let index = u64::from(province);
        // Active state, gamestate slot and container slot exist for every
        // province, railed or not: a copy plan takes the whole table.
        memory.write_u32(
            ACTIVE_TABLE + index * shape::ACTIVE_STRIDE as u64,
            0x100 + province,
        );
        if !railed(province) {
            // No rail slot: the walk skips this province entirely, so nothing
            // below it is mapped either.
            continue;
        }
        let object = province_object_of(province);
        memory.map_zeroed(object, 0x20);
        memory.write_u64(object, 0xB0000 + index);
        memory.write_u64(GS_ARRAY + index * shape::POINTER as u64, object);

        let map = map_province_of(province);
        let record = record_of(province);
        memory.map_zeroed(map, MAP_RECORD_PTR_OFF as usize + 0x20);
        memory.map_zeroed(record, RECORD_SELECTOR_KEY_OFF as usize + 0x10);
        memory.write_u64(map + MAP_RECORD_PTR_OFF, record);
        let anchor = anchor_of(province);
        for (axis, value) in anchor.iter().enumerate() {
            memory.write_u32(map + MAP_ANCHOR_OFF + axis as u64 * 4, *value);
        }
        memory.write_u32(record + RECORD_SELECTOR_KEY_OFF, selector_key_of(province));
        memory.write_u64(CONTAINER_ARRAY + index * shape::POINTER as u64, map);

        let slot = selector_slot_of(province);
        let entry = SELECTOR_TABLE + u64::from(slot) * shape::SELECTOR_STRIDE as u64;
        memory.write_u32(entry + 8, selector_key_of(province));
        memory.write_u64(entry + 0x10, selector_value_of(province));

        let info = rail_info_of(province);
        memory.map_zeroed(info, RAIL_HEADER_LEN as usize + 0x40);
        memory.write_u32(info + RAIL_PAIR_COUNT_OFF, 2);
        // The dead bytes carry a value, so a whole-interval comparison would
        // have something to see when the fixture rewrites them.
        memory.write_u32(info + RAIL_DEAD_OFF, 0xDEAD_0000 + province);
        if inline_pairs(province) {
            memory.write_u32(info + RAIL_KIND_OFF, 1);
            memory.write_u64(info + RAIL_INLINE_PAIRS_OFF, 0x1000 + index);
            memory.write_u64(info + RAIL_INLINE_PAIRS_OFF + 8, 0x2000 + index);
        } else {
            memory.write_u32(info + RAIL_KIND_OFF, 0);
            let blob = pair_blob_of(province);
            memory.map_zeroed(blob, 0x40);
            memory.write_u64(blob, 0x1000 + index);
            memory.write_u64(blob + 8, 0x2000 + index);
            memory.write_u64(info + RAIL_PAIR_PTR_OFF, blob);
        }

        let pointer = if province == REJECTED_PROVINCE {
            OUTSIDE_HEAP
        } else {
            info
        };
        memory.write_u64(RAIL_TABLE + index * shape::RAIL_SLOT as u64, pointer);
    }

    // RULING A. One page of the active table is unreadable. A copy plan takes
    // the whole table, so it asks for these bytes; a selective reader would
    // only have touched the entries it walks.
    memory.set_readable(ACTIVE_TABLE + VOID_PAGE_START, false);
    memory
}

// ---------------------------------------------------------------------------
// The outcome the two walkers must agree on
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    /// The rail slot was null: absent is not broken.
    NoRail,
    /// The rail pointer failed the admissibility predicate and was not
    /// followed.
    RailPointerRejected { raw: u64 },
    /// A byte this row consumes could not be read.
    Unreadable { what: &'static str },
    Row {
        pairs: [u64; 2],
        active: u32,
        source: u64,
        selector_key: u32,
        selector_value: u64,
        anchor: [u32; 3],
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RowOutcome {
    province: u32,
    outcome: Outcome,
}

/// d81's walk, over the fixture's own bytes. This is the "live reader" arm:
/// it reads whatever it likes, wherever it likes.
fn reference_walk(memory: &SyntheticMemory) -> Vec<RowOutcome> {
    let read = |addr: u64, len: usize| -> Option<Vec<u8>> {
        let mut out = vec![0u8; len];
        crate::snapshot_recipes::SnapshotMemory::copy_into(memory, addr, &mut out)
            .ok()
            .map(|()| out)
    };
    let u32_at = |addr: u64| read(addr, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64_at = |addr: u64| read(addr, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));

    let mut rows = Vec::new();
    for province in 0..PROVINCES {
        let index = u64::from(province);
        let slot = u64_at(RAIL_TABLE + index * shape::RAIL_SLOT as u64).expect("rail slot");
        if slot == 0 {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::NoRail,
            });
            continue;
        }
        if !(HEAP_LOW..HEAP_HIGH).contains(&slot) || slot % 8 != 0 {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::RailPointerRejected { raw: slot },
            });
            continue;
        }
        let kind = u32_at(slot + RAIL_KIND_OFF).expect("rail kind");
        let pairs = if kind == 1 {
            [
                u64_at(slot + RAIL_INLINE_PAIRS_OFF).expect("inline pair 0"),
                u64_at(slot + RAIL_INLINE_PAIRS_OFF + 8).expect("inline pair 1"),
            ]
        } else {
            let blob = u64_at(slot + RAIL_PAIR_PTR_OFF).expect("pair pointer");
            [
                u64_at(blob).expect("pair 0"),
                u64_at(blob + 8).expect("pair 1"),
            ]
        };
        let active = match u32_at(ACTIVE_TABLE + index * shape::ACTIVE_STRIDE as u64) {
            Some(value) => value,
            None => {
                rows.push(RowOutcome {
                    province,
                    outcome: Outcome::Unreadable { what: "active" },
                });
                continue;
            }
        };
        let source = u64_at(GS_ARRAY + index * shape::POINTER as u64).expect("gs slot");
        let map = u64_at(CONTAINER_ARRAY + index * shape::POINTER as u64).expect("container slot");
        let record = u64_at(map + MAP_RECORD_PTR_OFF).expect("record pointer");
        let selector_key = u32_at(record + RECORD_SELECTOR_KEY_OFF).expect("selector key");
        let selector_value =
            probe_selector(selector_key, |addr, len| read(addr, len)).expect("selector hit");
        let anchor = [
            u32_at(map + MAP_ANCHOR_OFF).expect("anchor 0"),
            u32_at(map + MAP_ANCHOR_OFF + 4).expect("anchor 1"),
            u32_at(map + MAP_ANCHOR_OFF + 8).expect("anchor 2"),
        ];
        rows.push(RowOutcome {
            province,
            outcome: Outcome::Row {
                pairs,
                active,
                source,
                selector_key,
                selector_value,
                anchor,
            },
        });
    }
    rows
}

/// `select_secondary`'s bounded linear probe, over whatever byte source it is
/// handed. Post-bracket in the replay arm — this is the compute a copy plan
/// moves out of the bracket, and it runs unchanged over the copied table.
fn probe_selector(
    key: u32,
    mut read: impl FnMut(u64, usize) -> Option<Vec<u8>>,
) -> Option<u64> {
    let mut slot = key & SELECTOR_MASK;
    for _ in 0..=SELECTOR_MAX_PROBE + 1 {
        let entry = SELECTOR_TABLE + u64::from(slot) * shape::SELECTOR_STRIDE as u64;
        let marker = read(entry + 4, 1)?[0];
        if marker == 0 {
            return None;
        }
        let held = u32::from_le_bytes(read(entry + 8, 4)?.try_into().ok()?);
        if held == key {
            return Some(u64::from_le_bytes(read(entry + 0x10, 8)?.try_into().ok()?));
        }
        slot += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// The recipe program
// ---------------------------------------------------------------------------

const OP_ROOT: u16 = 0;
const OP_RAIL_EXTENT: u16 = 1;
const OP_RAIL_SLOT_PTR: u16 = 2;
const OP_RAIL_HEADER: u16 = 3;
const OP_SELECT_LAYOUT: u16 = 4;
const OP_INLINE_PAIRS: u16 = 5;
const OP_PAIR_PTR_WINDOW: u16 = 6;
const OP_PAIR_PTR: u16 = 7;
const OP_PAIR_BLOB: u16 = 8;
const OP_JOIN_ACTIVE: u16 = 9;
const OP_ACTIVE_ENTRY: u16 = 10;
const OP_JOIN_SOURCE: u16 = 11;
const OP_SOURCE_SLOT: u16 = 12;
const OP_SOURCE_PTR: u16 = 13;
const OP_SOURCE_OBJECT: u16 = 14;
const OP_JOIN_MAP: u16 = 15;
const OP_MAP_SLOT: u16 = 16;
const OP_MAP_PTR: u16 = 17;
const OP_MAP_WINDOWS: u16 = 18;
const OP_RECORD_PTR: u16 = 19;
const OP_RECORD_WINDOW: u16 = 20;
const OP_ACTIVE_EXTENT: u16 = 21;
const OP_SOURCE_EXTENT: u16 = 22;
const OP_MAP_EXTENT: u16 = 23;
const OP_SELECTOR_HEADER_PTR: u16 = 24;
const OP_SELECTOR_HEADER: u16 = 25;
const OP_SELECTOR_EXTENT: u16 = 26;
const OP_WITNESS: u16 = 27;
const OP_FREEZE: u16 = 28;
/// The SECOND route to the gamestate province slot: d81's anchor-selector
/// resolver serves that slot from the walk's own opening read when the selector
/// key is the province's own id. Two logical reads, one address, one interval —
/// which is the shape ruling B's elision exists for.
const OP_JOIN_SELECTOR: u16 = 29;
const OP_SELECTOR_SLOT: u16 = 30;
const OP_COUNT: usize = 31;

const TABLE_ACTIVE: TableId = TableId(1);
const TABLE_SOURCE: TableId = TableId(2);
const TABLE_MAP: TableId = TableId(3);

/// Stage tags. The closing program orders them and elides the source stage into
/// the map stage, which is d81's coincident-slot elision.
const STAGE_RAIL: u16 = 1;
const STAGE_SELECTOR: u16 = 4;
const STAGE_SOURCE: u16 = 2;
const STAGE_MAP: u16 = 3;

/// Provinces the fixture gives a rail slot.
fn railed_count() -> usize {
    (0..PROVINCES).filter(|p| railed(*p)).count()
}

fn heap_window(on_reject: RejectAction) -> Admissibility {
    Admissibility {
        low: HEAP_LOW,
        high: HEAP_HIGH,
        align: 8,
        singletons: [0, 0],
        on_reject,
    }
}

fn d81_recipe() -> Recipe {
    let mut ops = vec![RecipeOp::Freeze; OP_COUNT];

    ops[OP_ROOT as usize] = RecipeOp::CopyHeader {
        size: 0x40,
        next: vec![
            OP_RAIL_EXTENT,
            OP_ACTIVE_EXTENT,
            OP_SOURCE_EXTENT,
            OP_MAP_EXTENT,
            OP_SELECTOR_HEADER_PTR,
            OP_WITNESS,
        ],
    };

    // The four province-indexed tables, each copied whole from the ENGINE's
    // declared count. Three of them are registered for the joins; the rail
    // table drives the walk through its own elements.
    ops[OP_RAIL_EXTENT as usize] = RecipeOp::DecodeExtent {
        count_offset: 0x00,
        count_width: 4,
        ptr_offset: 0x08,
        stride: shape::RAIL_SLOT as u32,
        max_count: 1 << 16,
        max_bytes: 1 << 24,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![OP_RAIL_SLOT_PTR],
    };
    ops[OP_ACTIVE_EXTENT as usize] = RecipeOp::DecodeExtent {
        count_offset: 0x00,
        count_width: 4,
        ptr_offset: 0x10,
        stride: shape::ACTIVE_STRIDE as u32,
        max_count: 1 << 16,
        max_bytes: 1 << 24,
        trailer_elements: 0,
        guard: None,
        register: Some(TABLE_ACTIVE),
        element: vec![OP_FREEZE],
    };
    ops[OP_SOURCE_EXTENT as usize] = RecipeOp::DecodeExtent {
        count_offset: 0x00,
        count_width: 4,
        ptr_offset: 0x18,
        stride: shape::POINTER as u32,
        max_count: 1 << 16,
        max_bytes: 1 << 24,
        trailer_elements: 0,
        guard: None,
        register: Some(TABLE_SOURCE),
        element: vec![OP_FREEZE],
    };
    ops[OP_MAP_EXTENT as usize] = RecipeOp::DecodeExtent {
        count_offset: 0x00,
        count_width: 4,
        ptr_offset: 0x20,
        stride: shape::POINTER as u32,
        max_count: 1 << 16,
        max_bytes: 1 << 24,
        trailer_elements: 0,
        guard: None,
        register: Some(TABLE_MAP),
        element: vec![OP_FREEZE],
    };

    // The selector map, and the derived extent that is the reason M1 exists.
    ops[OP_SELECTOR_HEADER_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0x28,
        guard: None,
        next: OP_SELECTOR_HEADER,
    };
    ops[OP_SELECTOR_HEADER as usize] = RecipeOp::CopyHeader {
        size: 0x40,
        next: vec![OP_SELECTOR_EXTENT],
    };
    ops[OP_SELECTOR_EXTENT as usize] = RecipeOp::DecodeExtent {
        // Unused: the count comes from the expression on this op.
        count_offset: 0x34,
        count_width: 4,
        ptr_offset: 0x08,
        stride: shape::SELECTOR_STRIDE as u32,
        max_count: 1 << 16,
        max_bytes: 1 << 24,
        trailer_elements: 0,
        guard: None,
        register: None,
        element: vec![OP_FREEZE],
    };

    // Each rail slot: follow its pointer under the admissibility predicate,
    // carrying the ELEMENT INDEX — which is the province id and is written
    // nowhere in the slot.
    ops[OP_RAIL_SLOT_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: OP_RAIL_HEADER,
    };
    ops[OP_RAIL_HEADER as usize] = RecipeOp::CopyHeader {
        size: RAIL_HEADER_LEN,
        next: vec![
            OP_SELECT_LAYOUT,
            OP_JOIN_ACTIVE,
            OP_JOIN_SOURCE,
            OP_JOIN_SELECTOR,
            OP_JOIN_MAP,
        ],
    };
    // The selector resolver's route to the SAME gamestate slot. It is a
    // separate logical read at a separate stage, and deduplication gives the
    // two one interval — so the closing program can elide the earlier close
    // into the later one and keep the elided stage's attribution.
    ops[OP_JOIN_SELECTOR as usize] = RecipeOp::JoinBounded {
        table: TABLE_SOURCE,
        key_offset: 0,
        key_width: 4,
        next: OP_SELECTOR_SLOT,
    };
    ops[OP_SELECTOR_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![OP_FREEZE],
    };
    // RULING C: which pair layout to copy next is decided from the copied
    // discriminant. No result leaves the op.
    ops[OP_SELECT_LAYOUT as usize] = RecipeOp::SelectByValue {
        probe: Probe::FieldEquals {
            offset: RAIL_KIND_OFF as u32,
            width: 4,
            value: 1,
        },
        then: OP_INLINE_PAIRS,
        otherwise: Some(OP_PAIR_PTR_WINDOW),
    };
    ops[OP_INLINE_PAIRS as usize] = RecipeOp::CopyRanges {
        windows: vec![RelWindow::new(RAIL_INLINE_PAIRS_OFF as i64, 16)],
        next: vec![OP_FREEZE],
    };
    ops[OP_PAIR_PTR_WINDOW as usize] = RecipeOp::CopyRanges {
        windows: vec![RelWindow::new(RAIL_PAIR_PTR_OFF as i64, 8)],
        next: vec![OP_PAIR_PTR],
    };
    ops[OP_PAIR_PTR as usize] = RecipeOp::DecodePointer {
        offset: RAIL_PAIR_PTR_OFF as u32,
        guard: None,
        next: OP_PAIR_BLOB,
    };
    ops[OP_PAIR_BLOB as usize] = RecipeOp::CopyHeader {
        size: 16,
        next: vec![OP_FREEZE],
    };

    // The three joins, all keyed on the carried province id.
    ops[OP_JOIN_ACTIVE as usize] = RecipeOp::JoinBounded {
        table: TABLE_ACTIVE,
        key_offset: 0,
        key_width: 4,
        next: OP_ACTIVE_ENTRY,
    };
    ops[OP_ACTIVE_ENTRY as usize] = RecipeOp::CopyHeader {
        size: 4,
        next: vec![OP_FREEZE],
    };
    ops[OP_JOIN_SOURCE as usize] = RecipeOp::JoinBounded {
        table: TABLE_SOURCE,
        key_offset: 0,
        key_width: 4,
        next: OP_SOURCE_SLOT,
    };
    ops[OP_SOURCE_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![OP_SOURCE_PTR],
    };
    ops[OP_SOURCE_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: OP_SOURCE_OBJECT,
    };
    ops[OP_SOURCE_OBJECT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![OP_FREEZE],
    };
    ops[OP_JOIN_MAP as usize] = RecipeOp::JoinBounded {
        table: TABLE_MAP,
        key_offset: 0,
        key_width: 4,
        next: OP_MAP_SLOT,
    };
    ops[OP_MAP_SLOT as usize] = RecipeOp::CopyHeader {
        size: 8,
        next: vec![OP_MAP_PTR],
    };
    ops[OP_MAP_PTR as usize] = RecipeOp::DecodePointer {
        offset: 0,
        guard: None,
        next: OP_MAP_WINDOWS,
    };
    // The map province is 4.5 KiB and the reader consumes twenty bytes of it,
    // in two places. Two windows, not one object.
    ops[OP_MAP_WINDOWS as usize] = RecipeOp::CopyRanges {
        windows: vec![
            RelWindow::new(MAP_ANCHOR_OFF as i64, 12),
            RelWindow::new(MAP_RECORD_PTR_OFF as i64, 8),
        ],
        next: vec![OP_RECORD_PTR],
    };
    ops[OP_RECORD_PTR as usize] = RecipeOp::DecodePointer {
        offset: MAP_RECORD_PTR_OFF as u32,
        guard: None,
        next: OP_RECORD_WINDOW,
    };
    ops[OP_RECORD_WINDOW as usize] = RecipeOp::CopyRanges {
        windows: vec![RelWindow::new(RECORD_SELECTOR_KEY_OFF as i64, 4)],
        next: vec![OP_FREEZE],
    };

    ops[OP_WITNESS as usize] = RecipeOp::WitnessClose {
        next: vec![OP_FREEZE],
    };

    let mut mods = vec![OpMod::default(); OP_COUNT];
    // M1: the selector extent's count is the engine's own formula.
    mods[OP_SELECTOR_EXTENT as usize].count = Some(CountExpr {
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
    });
    // RULING A: the four whole-table copies are supersets by construction, so
    // an unreadable span inside one of them refuses only the reads that reach
    // it. The selector table is the same shape.
    //
    // The per-row copies OUT OF those tables are scoped for the same reason and
    // it is the more important half: a row whose slot is in the void must
    // refuse at its own read while every other row of the table publishes. A
    // `Required` copy of a single voided slot would take the whole recipe down,
    // which is precisely the outcome the ruling forbids.
    //
    // The rule the program applies, stated so it is a rule and not a list:
    // anything derived from an ENGINE TABLE the plan copies wholesale is
    // scoped; anything reached through a pointer the recipe proved admissible
    // is `Required`, because a copy plan that cannot read an object it was
    // handed the address of has found a real defect and must say so.
    for op in [
        OP_RAIL_EXTENT,
        OP_ACTIVE_EXTENT,
        OP_SOURCE_EXTENT,
        OP_MAP_EXTENT,
        OP_SELECTOR_EXTENT,
        OP_ACTIVE_ENTRY,
        OP_SOURCE_SLOT,
        OP_SELECTOR_SLOT,
        OP_MAP_SLOT,
    ] {
        mods[op as usize].tolerance = Tolerance::Scoped;
    }
    mods[OP_JOIN_SELECTOR as usize].key_from_carry = true;
    mods[OP_SELECTOR_SLOT as usize].stage = STAGE_SELECTOR;
    // M2: the province id is the rail table's element index, carried into the
    // three joins.
    mods[OP_RAIL_SLOT_PTR as usize].carry = Some(CarrySource::ElementIndex);
    for op in [OP_JOIN_ACTIVE, OP_JOIN_SOURCE, OP_JOIN_MAP] {
        mods[op as usize].key_from_carry = true;
    }
    // M3: the rail pointer is admitted before it is followed, and a rejection
    // is a branch outcome that the reader turns into its own typed failure.
    mods[OP_RAIL_SLOT_PTR as usize].admit = Some(heap_window(RejectAction::StopBranch));
    mods[OP_MAP_PTR as usize].admit = Some(heap_window(RejectAction::StopBranch));
    mods[OP_RECORD_PTR as usize].admit = Some(heap_window(RejectAction::StopBranch));
    // M4 + RULING B: the rail header is taken as an adjacent pair and compared
    // only over the two fields the reader consumes, so the four dead bytes
    // between them cannot manufacture a torn gap.
    mods[OP_RAIL_HEADER as usize].samples = SampleSpec {
        count: 2,
        on_disagree: DisagreeAction::Gap,
    };
    mods[OP_RAIL_HEADER as usize].compare = CompareMask {
        windows: vec![
            RelWindow::new(RAIL_KIND_OFF as i64, 4),
            RelWindow::new(RAIL_PAIR_COUNT_OFF as i64, 4),
            RelWindow::new(RAIL_PAIR_PTR_OFF as i64, 8),
            RelWindow::new(RAIL_INLINE_PAIRS_OFF as i64, 16),
        ],
    };
    // The two conditional arms copy INTO the same object, so deduplication puts
    // them in the same interval. A contributor that declares no mask makes the
    // whole interval consumed again — deliberately, and it is the safe
    // direction — so an op that copies exactly what it consumes still has to
    // say so, or it silently undoes its neighbour's mask.
    mods[OP_INLINE_PAIRS as usize].compare = CompareMask {
        windows: vec![RelWindow::new(RAIL_INLINE_PAIRS_OFF as i64, 16)],
    };
    mods[OP_PAIR_PTR_WINDOW as usize].compare = CompareMask {
        windows: vec![RelWindow::new(RAIL_PAIR_PTR_OFF as i64, 8)],
    };
    // RULING B: stages, their closure order, and the source stage elided into
    // the map stage.
    mods[OP_RAIL_HEADER as usize].stage = STAGE_RAIL;
    mods[OP_SOURCE_SLOT as usize].stage = STAGE_SOURCE;
    mods[OP_MAP_SLOT as usize].stage = STAGE_MAP;

    Recipe::new(
        RecipeId(81),
        ConsumerId(81),
        ops,
        vec![Root {
            op: OP_ROOT,
            addr: ROOT,
        }],
    )
    .with_mods(mods)
    .with_closing(ClosingProgram {
        stages: vec![
            CloseStage {
                stage: STAGE_RAIL,
                elide_into: None,
                when: CloseWhen::Final,
            },
            // d81's elision, in its own shape: the selector's close is skipped
            // in favour of the LATER source-position read, and the selector
            // stage keeps its attribution.
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
    })
}

// ---------------------------------------------------------------------------
// The post-bracket walk, over the view and nothing else
// ---------------------------------------------------------------------------

fn replay_walk(
    view: &SnapshotView<'_>,
    rejected: &[u64],
) -> Vec<RowOutcome> {
    let read = |addr: u64, len: usize| -> Result<Vec<u8>, ViewRefusal> {
        let mut out = vec![0u8; len];
        view.read_into(addr, &mut out)?;
        Ok(out)
    };
    let u32_at =
        |addr: u64| read(addr, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64_at =
        |addr: u64| read(addr, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));

    let mut rows = Vec::new();
    for province in 0..PROVINCES {
        let index = u64::from(province);
        let slot = u64_at(RAIL_TABLE + index * shape::RAIL_SLOT as u64).expect("rail slot copied");
        if slot == 0 {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::NoRail,
            });
            continue;
        }
        // The recipe recorded the rejection; the reader raises its own typed
        // failure from it, which is exactly what M3 exists for.
        if rejected.contains(&slot) {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::RailPointerRejected { raw: slot },
            });
            continue;
        }
        let kind = u32_at(slot + RAIL_KIND_OFF).expect("rail kind copied");
        let pairs = if kind == 1 {
            [
                u64_at(slot + RAIL_INLINE_PAIRS_OFF).expect("inline pair 0"),
                u64_at(slot + RAIL_INLINE_PAIRS_OFF + 8).expect("inline pair 1"),
            ]
        } else {
            let blob = u64_at(slot + RAIL_PAIR_PTR_OFF).expect("pair pointer copied");
            [
                u64_at(blob).expect("pair 0"),
                u64_at(blob + 8).expect("pair 1"),
            ]
        };
        let active = match u32_at(ACTIVE_TABLE + index * shape::ACTIVE_STRIDE as u64) {
            Ok(value) => value,
            Err(ViewRefusal::Unreadable { .. }) => {
                rows.push(RowOutcome {
                    province,
                    outcome: Outcome::Unreadable { what: "active" },
                });
                continue;
            }
            Err(other) => panic!("province {province}: active entry {other:?}"),
        };
        let source = u64_at(GS_ARRAY + index * shape::POINTER as u64).expect("gs slot copied");
        let map =
            u64_at(CONTAINER_ARRAY + index * shape::POINTER as u64).expect("container slot copied");
        let selector_key = u32_at(record_of(province) + RECORD_SELECTOR_KEY_OFF)
            .expect("selector key copied");
        // The probe is POST-BRACKET compute over the copied table: the win FX
        // measured is that the whole table is one copy and the probe costs no
        // guarded calls at all.
        let selector_value = probe_selector(selector_key, |addr, len| read(addr, len).ok())
            .expect("selector hit over the copied table");
        let anchor = [
            u32_at(map + MAP_ANCHOR_OFF).expect("anchor 0"),
            u32_at(map + MAP_ANCHOR_OFF + 4).expect("anchor 1"),
            u32_at(map + MAP_ANCHOR_OFF + 8).expect("anchor 2"),
        ];
        rows.push(RowOutcome {
            province,
            outcome: Outcome::Row {
                pairs,
                active,
                source,
                selector_key,
                selector_value,
                anchor,
            },
        });
    }
    rows
}

/// The declared closure order must be the order that RAN — not merely some
/// order, and not interval-id order.
///
/// The check is on the SEQUENCE: walking `closing_order`, the effective stage
/// rank of each interval must never decrease. An interval's rank is the
/// earliest listed stage it carries, with an elided stage counting as the stage
/// it was elided into. Intervals carrying no listed stage sort last.
///
/// It also asserts the order is not the id order it would be without a program,
/// because "the program ran" and "some order happened" are different claims.
fn assert_closing_order_follows_the_program(
    build: &crate::snapshot_recipes::executor::SnapshotBuild,
) {
    let rank_of = |stage: u16| -> usize {
        match stage {
            STAGE_RAIL => 0,
            // Elided into the source stage, so it takes the source stage's
            // position rather than its own.
            STAGE_SELECTOR | STAGE_SOURCE => 2,
            STAGE_MAP => 3,
            _ => usize::MAX,
        }
    };
    let mut previous = 0usize;
    let mut ranks = Vec::new();
    for id in &build.closing_order {
        let rank = build
            .snapshot
            .annotations()
            .stages_of(*id)
            .iter()
            .map(|stage| rank_of(*stage))
            .min()
            .unwrap_or(usize::MAX);
        assert!(
            rank >= previous,
            "the closing wave ran a stage-{rank} interval after a stage-{previous} one; \
             the declared order is not the order that ran",
        );
        previous = rank;
        ranks.push(rank);
    }
    assert!(
        ranks.iter().any(|rank| *rank == 0) && ranks.iter().any(|rank| *rank == 3),
        "the program must actually order more than one stage",
    );
    let by_id: Vec<_> = {
        let mut sorted = build.closing_order.clone();
        sorted.sort();
        sorted
    };
    assert_ne!(
        build.closing_order, by_id,
        "a declared order that happens to equal interval-id order proves nothing",
    );
}

// ---------------------------------------------------------------------------
// THE GATE
// ---------------------------------------------------------------------------

/// THE S4 GATE. Every shape lane FX staged, written as one recipe program, run
/// against a staged world, producing the same rows as a direct-read walk of the
/// same world — refusal paths included.
///
/// MUTATION RUNS that must fail this test, each naming the mechanism it
/// removes:
///
///  * drop `CarrySource::ElementIndex` from `OP_RAIL_SLOT_PTR` → the three
///    joins have no key and the recipe refuses `CarryNotSet`;
///  * drop the `CountExpr` from `OP_SELECTOR_EXTENT` → the table is copied
///    `mask` slots long and every probe that walks into the tail refuses;
///  * drop `Tolerance::Scoped` from the extents → the unreadable page
///    refuses the whole recipe and no province publishes;
///  * drop the `Admissibility` from `OP_RAIL_SLOT_PTR` → the out-of-window
///    pointer is followed, its copy fails, and the recipe refuses;
///  * replace `SelectByValue` with an unconditional inline copy → the
///    out-of-line provinces' pairs are never copied.
#[test]
fn the_substrate_expresses_d81s_observation_phase_and_produces_the_same_rows() {
    let memory = stage_world();
    let expected = reference_walk(&memory);

    let hour = HourStamp(19390901);
    let build = observe(
        &[d81_recipe()],
        &memory,
        ObservationConfig::new(hour),
    )
    .expect("custody");

    let report = build.report_for(RecipeId(81)).expect("reported");
    assert!(
        matches!(report.outcome, RecipeOutcome::Captured),
        "the program must capture, not refuse: {:?}",
        report.outcome,
    );
    // RULING A: the unreadable page is a named void and the frame still
    // publishes.
    assert!(
        report.scoped_voids >= 1,
        "the fixture stages an unreadable page inside a speculative copy",
    );
    // M3: exactly one pointer was rejected, and it names its own witness.
    assert_eq!(report.pointers_rejected, 1);

    let rejected: Vec<u64> = build
        .snapshot
        .rejections_for(ConsumerId(81))
        .map(|rejection| rejection.addr)
        .collect();
    assert_eq!(rejected, vec![OUTSIDE_HEAP]);
    // M5: the rejection carries the witness a converted reader raises its own
    // typed InvalidPointer from. Asserted field by field, or "the mechanism is
    // wired" would mean nothing more than "a struct exists".
    let rejection = build
        .snapshot
        .rejections_for(ConsumerId(81))
        .next()
        .expect("one rejection");
    assert_eq!(rejection.witness.raw, OUTSIDE_HEAP, "the value that failed");
    assert_eq!(
        rejection.witness.read_addr,
        RAIL_TABLE + u64::from(REJECTED_PROVINCE) * shape::RAIL_SLOT as u64,
        "the slot it was read out of",
    );
    assert_eq!(rejection.witness.read_width, 8);
    assert_eq!(
        rejection.witness.index, REJECTED_PROVINCE,
        "and the element index, which for this walk IS the province id",
    );

    // M4: the rail headers were taken as adjacent pairs. Without this the
    // program would still produce every row above and the torn-detection
    // instrument would silently not exist.
    let sampled = build
        .snapshot
        .annotations()
        .sampled_intervals()
        .filter(|(_, images)| !images.extra.is_empty())
        .count();
    assert_eq!(
        sampled,
        railed_count() - 1,
        "one adjacent pair per railed province whose rail pointer was admitted",
    );

    // RULING B: the declared closure order RAN, in the declared order, and the
    // source stage was elided into the map stage where the two coincide.
    assert!(
        !build.closing_order.is_empty(),
        "a witnessed program must close something",
    );
    assert_closing_order_follows_the_program(&build);
    // ONE, not one per province, and the difference is the finding: a copy plan
    // takes the gamestate array WHOLE, so every province's slot — by either
    // route — is inside one deduplicated interval. The elision is therefore a
    // property of that interval, and the per-province elision d81 performs
    // today collapses into it. That is the copy plan doing its job, not the
    // elision failing to fire.
    assert_eq!(
        build.elisions.len(),
        1,
        "the selector route and the walk's own route land in the same interval, \
         so the earlier close is elided into the later one exactly once",
    );
    for elision in &build.elisions {
        assert_eq!((elision.elided, elision.into), (STAGE_SELECTOR, STAGE_SOURCE));
    }

    let closed = build.close_witnesses();
    let view = closed
        .snapshot
        .view_for(ConsumerId(81), hour)
        .expect("the consumer keeps its hour");
    let produced = replay_walk(&view, &rejected);

    assert_eq!(
        produced.len(),
        expected.len(),
        "one outcome per province in both arms",
    );
    for (left, right) in produced.iter().zip(expected.iter()) {
        assert_eq!(
            left, right,
            "province {} differs between the copy plan and the direct-read walk",
            left.province,
        );
    }

    // And the fixture must actually contain every outcome the gate claims to
    // cover, or the loop above proves nothing about the refusal paths.
    let kinds = |rows: &[RowOutcome]| {
        (
            rows.iter()
                .filter(|row| matches!(row.outcome, Outcome::NoRail))
                .count(),
            rows.iter()
                .filter(|row| matches!(row.outcome, Outcome::RailPointerRejected { .. }))
                .count(),
            rows.iter()
                .filter(|row| matches!(row.outcome, Outcome::Unreadable { .. }))
                .count(),
            rows.iter()
                .filter(|row| matches!(row.outcome, Outcome::Row { .. }))
                .count(),
        )
    };
    let (no_rail, rejected_rows, unreadable, published) = kinds(&expected);
    assert!(no_rail > 0, "the walk must include railless provinces");
    assert_eq!(rejected_rows, 1, "and exactly one rejected rail pointer");
    assert!(
        unreadable > 0,
        "and at least one row whose consumed bytes are in the void",
    );
    assert!(
        published > unreadable,
        "and the over-read must not have cost the rows that never touch it",
    );
}

/// RULING B, on the same program: a write to the rail header's dead bytes
/// between the waves must not gap the reader, and a write to a consumed field
/// must.
///
/// The whole-interval comparison this replaces would fail the first half.
#[test]
fn the_rail_headers_dead_bytes_do_not_gap_the_reader_but_its_fields_do() {
    use crate::snapshot_recipes::synthetic::MutatingMemory;
    use crate::snapshot_recipes::GapReason;

    let hour = HourStamp(19390901);
    let victim = rail_info_of(0);

    // Where the closing wave starts, measured rather than guessed: every
    // interval is witnessed, so the closing copies are the last
    // `interval_count` of them and the openings are everything before.
    // WHERE THE CLOSING WAVE BEGINS, asked of the observation rather than
    // computed from the interval count. It used to be `total - intervals + 1`,
    // which held only while every interval took ONE closing copy; since the
    // closing wave takes as many observations as an interval's consumers asked
    // for (owner ruling 2026-08-11), that arithmetic put the write AFTER the
    // images it was meant to land between and the control silently stopped
    // gapping.
    let (total_copies, closing_copies) = {
        let plain = stage_world();
        plain.reset_counters();
        let build =
            observe(&[d81_recipe()], &plain, ObservationConfig::new(hour)).expect("custody");
        (plain.copies(), build.closing_copies)
    };
    let mid = total_copies - closing_copies + 1;

    // The control on the control: with the trigger past every copy the fixture
    // writes nothing at all, so an empty gap set here proves only that the
    // program is clean to begin with.
    let quiet = MutatingMemory::new(
        stage_world(),
        total_copies + 1,
        vec![(victim + RAIL_DEAD_OFF, 0xFFFF_FFFFu32.to_le_bytes().to_vec())],
    );
    let closed = observe(&[d81_recipe()], &quiet, ObservationConfig::new(hour))
        .expect("custody")
        .close_witnesses();
    assert!(!quiet.mutation_applied());
    assert!(closed.snapshot.gap_reason(ConsumerId(81)).is_none());

    let dead = MutatingMemory::new(
        stage_world(),
        mid,
        vec![(victim + RAIL_DEAD_OFF, 0xFFFF_FFFFu32.to_le_bytes().to_vec())],
    );
    let closed = observe(&[d81_recipe()], &dead, ObservationConfig::new(hour))
        .expect("custody")
        .close_witnesses();
    assert!(dead.mutation_applied(), "the fixture must actually write");
    assert!(
        closed.snapshot.gap_reason(ConsumerId(81)).is_none(),
        "a write to the four dead bytes is not a torn read: {:?}",
        closed.snapshot.gap_reason(ConsumerId(81)),
    );

    // The control: the same write, one field over, inside a consumed window.
    let live = MutatingMemory::new(
        stage_world(),
        mid,
        vec![(
            victim + RAIL_PAIR_COUNT_OFF,
            0xFFFF_FFFFu32.to_le_bytes().to_vec(),
        )],
    );
    let closed = observe(&[d81_recipe()], &live, ObservationConfig::new(hour))
        .expect("custody")
        .close_witnesses();
    assert!(live.mutation_applied());
    assert!(
        matches!(
            closed.snapshot.gap_reason(ConsumerId(81)),
            Some(GapReason::WitnessChanged { .. }) | Some(GapReason::WitnessDependency { .. })
        ),
        "a write to a consumed field must still gap: {:?}",
        closed.snapshot.gap_reason(ConsumerId(81)),
    );
}

/// The claim FX's census rests on, restated as a property of the PROGRAM rather
/// than of a spreadsheet: the four province-indexed tables are four copy
/// transactions, not one per province.
#[test]
fn the_province_indexed_tables_are_one_transaction_each() {
    let memory = stage_world();
    let hour = HourStamp(19390901);
    memory.reset_counters();
    let build = observe(&[d81_recipe()], &memory, ObservationConfig::new(hour)).expect("custody");

    // THE COUNT, asserted and not merely reset. A plan that copied each
    // province's slot separately and then also happened to hold one covering
    // interval would satisfy every span assertion below while doing exactly the
    // per-province work a copy plan exists to remove.
    //
    // The budget: five whole-table copies (the rail table, the gamestate array,
    // the container array and the selector table land whole; the active table
    // bisects into three because one of its pages is unreadable), plus six
    // scattered copies per admitted railed province — the rail header and its
    // adjacent sample, the pair blob or the inline window, the active entry,
    // the two gamestate-slot routes (deduplicated to one), the container slot,
    // the map windows and the record window — plus one closing copy per
    // interval. Ten per province is a ceiling with room, and the point is that
    // it does not scale with the TABLE, which is 512 entries wide.
    let copies = memory.copies();
    let ceiling = 20 + 10 * railed_count() as u64 + build.snapshot.interval_count() as u64;
    assert!(
        copies < ceiling,
        "the plan issued {copies} copies against a ceiling of {ceiling}; a \
         whole-table copy that has become per-province is the defect this bounds",
    );
    assert!(
        copies > build.snapshot.interval_count() as u64,
        "sanity: the counter is live and the closing wave ran",
    );

    // One interval per whole table, spanning every province's slot.
    let spans_whole_table = |base: u64, stride: usize| {
        let want = Span::new(base, PROVINCES as u64 * stride as u64);
        build
            .snapshot
            .intervals()
            .iter()
            .any(|record| record.span().contains_span(&want))
    };
    assert!(spans_whole_table(RAIL_TABLE, shape::RAIL_SLOT));
    assert!(spans_whole_table(GS_ARRAY, shape::POINTER));
    assert!(spans_whole_table(CONTAINER_ARRAY, shape::POINTER));
    // The active table is the one with a hole in it, by construction.
    assert!(
        !spans_whole_table(ACTIVE_TABLE, shape::ACTIVE_STRIDE),
        "the voided page is genuinely absent from the active table's coverage",
    );
    // And the selector table is one interval as long as the DERIVED count.
    let selector = Span::new(
        SELECTOR_TABLE,
        u64::from(SELECTOR_SLOTS) * shape::SELECTOR_STRIDE as u64,
    );
    assert!(
        build
            .snapshot
            .intervals()
            .iter()
            .any(|record| record.span().contains_span(&selector)),
        "the selector extent must span mask + max_probe + 2 slots",
    );
}
