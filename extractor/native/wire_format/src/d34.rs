//! d34 `air_base_status` — per-air-base capacity, level and basing occupancy.
//!
//! Derived from `world_fold.rs:23790`-`:23906`
//! (`fold_air_base_status`), its payload constants at `:985`-`:986`, `:758` and
// @world-fold-authority domain=34 functions=fold_air_base_status lines=23790-23906 sha256=0f7047afd891bf9477e921027b7c458e2b1f69471564da364ae7342d79e3a9da closure=90b98cd5e3cc84ede49a90111bfcaf6fb375cb063731e999d2603ef0066a2d87
//! `:499`, the accumulator it folds into (`:1870`), the two consumers of that
//! accumulator (`:8169`-`:8219`, `:8926`-`:8962`), and the collector read and
//! payload writer that produce the rows
//! (`extractor/native/collector/src/attach_log.rs:55780`-`:55870`,
//! `:56098`-`:56200`, `:71573`-`:71606`). `extractor/documents/domains/` carries
//! no d34 dossier, so the fold and the collector read are the whole basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_air_base_status`, writer at
//! `attach_log.rs::compact_air_base_status_data_payload`): a 36-byte header
//! followed by `row_count` fixed 44-byte rows. There is no legacy short row —
//! the fold requires `payload.len() == 36 + row_count * 44` exactly, so the
//! length never selects a row form and the mapper accepts exactly one.
//!
//! | header | offset | field | read by the fold? |
//! |---|---|---|---|
//! | u64 | `+0`  | `elapsed_wall_ms`         | no |
//! | u32 | `+8`  | `row_count`               | **yes** |
//! | u32 | `+12` | `changed_rows`            | no |
//! | u32 | `+16` | `status_flags`            | **yes** |
//! | u32 | `+20` | `observed_air_base_rows`  | no |
//! | u32 | `+24` | `observed_pool_rows`      | **yes** — the bound on unresolved rows |
//! | u32 | `+28` | `deleted_rows`            | no |
//! | u32 | `+32` | `payload_version`         | **yes**, must be [`PAYLOAD_VERSION`] |
//!
//! **No word here is unread.** The four the fold ignores are read by
//! `recorder_host/src/domain_data_authority.rs::decode_compact_air_base_status_data_payload`,
//! which publishes each as a per-domain unit count and cross-checks
//! `changed_rows`, `observed_air_base_rows` and `deleted_rows` against
//! `row_count`. `observed_pool_rows` gained its second reader with the anchored
//! rows below: the fold refuses a record carrying more
//! `pool_observed_unresolved` rows than the pool walk saw pools.
//!
//! **`changed_rows == row_count` is NOT an invariant of this domain.** d63's
//! fold asserts it (`:9267`); this one never reads the word at all. The writer
//! happens to set `changed_rows = rows.len()` (`attach_log.rs:71584`-`:71585`),
//! which is why it holds on all 146 records of the reference capture — an
//! accident of one writer, not a rule, so [`split_payload`] does not enforce it.
//! The recipe's own wording: carry "the invariants YOUR authorities actually
//! state".
//!
//! | row | offset | width | field | scale | gate |
//! |---|---|---|---|---|---|
//! | `+0`  | u32 | `air_base_id`       | raw | none (key; zero **not** rejected) |
//! | `+4`  | u32 | `state_id`          | raw | none |
//! | `+8`  | u32 | `flags`             | —   | only `1<<0` DELETED is legal |
//! | `+12` | u32 | `capacity`          | raw | none |
//! | `+16` | u32 | `building_level`    | raw | none |
//! | `+20` | u32 | `based_wing_count`  | raw | none |
//! | `+24` | u32 | `based_plane_count` | raw | none |
//! | `+28` | u32 | `province_id`       | raw | none; `0` means no province of its own |
//! | `+32` | u32 | `anchor_kind`       | —   | one of the three [`ANCHOR_KIND_STATE`] values |
//! | `+36` | u32 | `host_object_type`  | raw | nonzero iff `anchor_kind == 1` |
//! | `+40` | u32 | `host_object_id`    | raw | nonzero iff `anchor_kind == 1` |
//!
//! # An air base is placed by a state OR by a host, never by neither
//!
//! `runs/ue_d34_pool_bases/RE_NON_STATE_AIR_BASE.md` is the engine evidence, and
//! the engine states the rule in its own assertion text: a `CAirBase` with no
//! state row and no host reference is refused at creation
//! (*"Invalid air base, no carrier or state"*, `strategicair.cpp:0x1ac7`), and
//! the province slot at `CAirBase+0x70` is derived only when the state pointer
//! at `+0x68` is set. So a carrier-hosted base has **no province at all**, and
//! `province_id == 0` on such a row is the world, not a gap in the reading.
//!
//! `anchor_kind == 2` is neither: the wing-pool walk named that air-base id and
//! no `CAirBase` walk reached the object. The row exists so the id is named
//! rather than dropped, and its two occupancy counters are what the pool walk
//! measured. It carries **no** status bit: d34 has no expected-partial
//! tolerance, so a failure-class bit would make the recorder withhold every row
//! of the record — this one included. The row is the account.
//!
//! **No scale factors anywhere.** Every one of the nine carried values is a u32
//! the fold stores verbatim (`:15192`-`:15203`) and every consumer reads back
//! verbatim (`:8939`-`:8955`). There is no `FIXED_SCALE` word in the header and
//! no division in the fold, so there is no continuous field and nothing to
//! quantize.
//!
//! # This domain accumulates: one record is not one state
//!
//! `fold_air_base_status` calls `replace_or_sparse_after_first` (`:15169`,
//! defined `:2947`). The FIRST record the fold sees clears the accumulator and
//! is a complete snapshot; every later record is a **sparse patch** — the rows
//! it carries are upserted, rows it omits keep their previous values, and a row
//! with the DELETED bit removes its key (`:15188`-`:15191`). The collector emits
//! exactly that shape: `emit_full_snapshot = tracker.previous.is_empty()`
//! (`attach_log.rs:55824`), then per-hour changed rows plus a tombstone per
//! disappeared id (`:55833`-`:55844`).
//!
//! So the hour's state is the accumulator, not the record, and [`Accumulator`]
//! is that fold — public here rather than rebuilt in a harness, per the recipe's
//! fourth boundary. Measured on `runs/batch2/deepwar_t1`: record 1 carries all
//! 374 air bases, records 2..146 carry 0–30 rows each (mean 4.41 changed
//! rows/hour), the accumulator stays at 374 rows for all 146 hours, and no
//! record sets `COMPACT_STATUS_DELTA_DATA`.
//!
//! **`self.air_base_status` has exactly one writer.** `world_fold.rs` touches it
//! at `world_fold.rs::fold_air_base_status` (clear), `world_fold.rs::fold_air_base_status` (remove) and `world_fold.rs::fold_air_base_status` (insert) and nowhere
//! else; the only other mentions are the two read sites and the field
//! declaration. Unlike d26 there is no cross-domain pruner to model.
//!
//! # Four-state semantics
//!
//! **`anchor_kind` decides which empty state a zero is, and it decides it for
//! every relation a row carries.** Zero is a legal state id, a legal province
//! id and a legal registry id, so no such word can be read on its own; the row
//! says which of the four states applies.
//!
//! | | `state_id` | `province_id == 0` | absent host pair |
//! |---|---|---|---|
//! | `anchor_kind == 0` state | the id, zero included — the fold applies no zero check and this mapper may not add one | *unknown* — the base HAS a province and the recording failed to carry it | *null* — a state base is known to have no host |
//! | `anchor_kind == 1` host | *null* — a base with no state row has no owning state | *null* — and so no province of its own either | (never absent: a host row with no host is refused) |
//! | `anchor_kind == 2` unresolved | *unknown* — no air-base object was reached | *unknown* — likewise | *unknown* — likewise |
//!
//! Every *null* cell is the engine's own mutual exclusion, not an inference:
//! a `CAirBase` is valid iff it has a state row **or** a host
//! (`strategicair.cpp:0x1ac7`), the province slot is derived only from the state
//! (`RE_NON_STATE_AIR_BASE.md` §2), and the recorder refuses a state-anchored
//! row that carries a host pair. Collapsing either cell into *unknown* would say
//! "this existed and we missed it" about something the engine guarantees is not
//! there — the exact conflation this format's four states exist to prevent
//! (`value.rs` module docs).
//!
//! [`AirBaseRow::state`], [`AirBaseRow::empty_province`] and
//! [`AirBaseRow::empty_host`] are the one implementation of that table. Measured on the reference capture: **0**
//! zero-province rows across all 1014 rows, all state-anchored.
//!
//! *Recorded conflict.* Both JSON consumers render a zero province as
//! `Value::Null` (`world_fold.rs::build_order_of_battle`,
//! `world_fold.rs::build_strategic_air`). JSON has no `unknown`, so that is the
//! fold's uniform value-vs-nothing collapse, not a semantic claim that outranks
//! the table above. Both now also emit the anchor beside it, so a consumer can
//! tell the two zeroes apart — which is the whole reason the anchor is on the
//! wire and not inferred.
//!
//! **Nothing else reaches an empty state.** `capacity`, `building_level`,
//! `based_wing_count`, `based_plane_count` and `anchor_kind` are read
//! unconditionally with no gate and no sentinel
//! (`world_fold.rs::fold_air_base_status`); a zero in any of them is a real
//! measured value — an air base with no wings based on it reads `0` and that is
//! a fact, not a missing reading. So the schema declares both empty states on
//! the four placement fields and on nothing else, and **no** `optional` field at
//! all: nothing here is ever inapplicable, because every row is an air base and
//! every air base has a placement.
//!
//! **`flags` is not a field.** Its only legal bit is DELETED (`world_fold.rs::fold_air_base_status`
//! rejects every other bit), and DELETED is a wire tombstone — a removal, not a
//! row of data. [`record_from_row`] refuses a deleted row for the same reason
//! d26's mapper does; the removal is expressed by [`Accumulator`] dropping the
//! key, which is where a keyed set puts it.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock. The mapper still takes `frame_tick` so every domain's boundary has one
//! shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 34;
pub const AIR_BASES: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const AIR_BASE_ID: u32 = 1;
    pub const STATE_ID: u32 = 2;
    pub const CAPACITY: u32 = 3;
    pub const BUILDING_LEVEL: u32 = 4;
    pub const BASED_WING_COUNT: u32 = 5;
    pub const BASED_PLANE_COUNT: u32 = 6;
    pub const PROVINCE_ID: u32 = 7;
    pub const ANCHOR_KIND: u32 = 8;
    pub const HOST_OBJECT_TYPE: u32 = 9;
    pub const HOST_OBJECT_ID: u32 = 10;
}

// --- Payload grammar, from `world_fold.rs:985`-`:986`, `:758`, `:499`. --------

// FROM THE SHARED GRAMMAR, and this port keeps its own semantic projection,
// schema and refusals. `d34_grammar` is a dependency-free leaf -- the same
// shape as `d48_grammar` and `id_set_codec` -- so it carries no recorder or
// capture-lane policy and does not widen the `compact_contract` allow-list.
// It exists because these words WERE written out here and in three other
// crates that cannot see each other, so a producer-side renumbering compiled
// and this port went on decoding the old codes (Codex round 41, finding 3).
pub use d34_grammar::HEADER_LEN;
pub use d34_grammar::ROW_LEN;

/// `d34_grammar/src/lib.rs::PAYLOAD_VERSION`, in the header word that used to
/// be `reserved`. A payload declaring any other version is refused rather than
/// reinterpreted.
pub use d34_grammar::PAYLOAD_VERSION;

/// `d34_grammar/src/lib.rs::ANCHOR_KIND_STATE`. Reached through the per-state
/// air-base table; `province_id` is the base's own.
pub use d34_grammar::ANCHOR_KIND_STATE;
/// `d34_grammar/src/lib.rs::ANCHOR_KIND_HOST`. Reached through the manager's
/// stateless vector; the host handle is the only placement the base has.
pub use d34_grammar::ANCHOR_KIND_HOST;
/// `d34_grammar/src/lib.rs::ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED`. The wing-pool
/// walk named this id and no `CAirBase` walk reached it.
pub use d34_grammar::ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`. The **only** legal flag bit:
/// `flags & !ROW_FLAG_DELETED_LOW_BIT != 0` is a hard error (`:15178`-`:15180`).
pub use d34_grammar::ROW_FLAG_DELETED;

/// `SUPPLY_MAX_PROVINCE_ID`, `world_fold.rs::SUPPLY_MAX_PROVINCE_ID`. A row whose province id
/// exceeds this is rejected (`:15185`-`:15187`) — checked **before** the delete
/// branch, so it applies to tombstones too.
///
/// This constant used to be `13_413`, the VANILLA PROVINCE COUNT, after the
/// producer had already stopped using that number. `bound_trips.rs:54` records
/// what it cost the first time: `runs/live_unlock/t15` (555 hours, unmodded)
/// observed exactly 13_413 as the maximum in every province-id field, so the
/// margin was zero and the first province a mod adds was already past it. The
/// producer's ceiling is now the engine's own province-array length
/// (`attach_log.rs`'s `supply_province` module) and the shared representation
/// envelope is `portable_admission::SUPPLY_MAX_PROVINCE_ID`; this consumer was
/// left behind, so the recorder collected rows d34 then refused — the exact
/// producer-widened / consumer-still-narrow shape
/// `tests/wire_format_suite/producer_consumer_admission_agreement.rs` exists to prevent.
///
/// It was then `65_535` and documented here as "a REPRESENTATION ENVELOPE".
/// **That was a guess too, and lane IA retired it.** Its stated derivation ("the
/// u16 index space, the same ceiling the tree already applies ... from
/// independent evidence") was circular: all three constants cited as that
/// independent evidence are bare literals in `collector/src/attach_log/
/// offsets.rs` with no derivation, and this constant cited them while they
/// cited nothing.
///
/// It is now the province id space's STRUCTURAL width, `i32::MAX - 1`, proven
/// `int` province id against a signed `int` size at `ai_area.cpp:0xcc`
/// (`claim_347234c20a542ca79032`). The engine also stores a province id in
/// THIRTY-TWO bits (`CProvince + 0xA4`), so 65 535 was never a field width
/// here; no wire row in this tree carries a province id in sixteen bits, which
/// is why this is a bounds edit and not a format change. The native identity
/// carrier is retained as `domain-34::Air-base identity carrier` in
/// `extractor/documents/technical_evidence.json`.
///
/// Still not a province count: nothing here knows how many provinces the loaded
/// mod set defines, and the number that does is on the producer side, which
/// bounds against the engine's own array length.
/// `tests/wire_format_suite/producer_consumer_admission_agreement.rs` pins this spelling to the
/// shared one.
pub const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;

/// The sentinel the fold documents as "province unobtained"
/// (`world_fold.rs::fold_air_base_status`). Mapped to [`Value::Unknown`].
pub const PROVINCE_ID_UNOBTAINED: u32 = 0;

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // Key. `Unsigned`, not `ContentId`: the id is the high half of a live
        // engine object handle (`attach_log.rs:56151`), minted while the campaign
        // runs and bounded by `MAX_AIR_BASE_ID`. No content manifest resolves
        // "air base 407" — it is a runtime instance identity like d20's `div_id`,
        // and content-identity contract's deciding question ("must a manifest resolve this for it to
        // keep its meaning under another mod set?") answers no.
        Field::new(f::AIR_BASE_ID, Kind::Unsigned).key(0),
        // The owning state's id (`world_fold.rs::fold_air_base_status`). `ContentId`: a state is
        // defined by map content, so state 1081 is whatever state the mod set
        // defines — the same call d22 makes for `province_state_id`. Both empty
        // states, because a base with no state row has no owning state at all;
        // see the module docs.
        Field::new(f::STATE_ID, Kind::ContentId)
            .nullable()
            .unknown_capable(),
        // Plane slots the base provides (`:15198`). A capacity that is spent and
        // rebuilt, not a counter: `Unsigned`, and it falls when the base is
        // damaged, so `Monotone` would be a false hint.
        Field::new(f::CAPACITY, Kind::Unsigned),
        // The air-base building's level in the owning state
        // (`attach_log.rs:56195`-`:56202`). Rises with construction and falls
        // with damage: `Unsigned`.
        Field::new(f::BUILDING_LEVEL, Kind::Unsigned),
        // Occupancy, joined in from the strategic-air country containers
        // (`attach_log.rs:56205`-`:56280`). Both move in both directions every
        // time a wing rebases: `Unsigned`, never `Monotone`.
        Field::new(f::BASED_WING_COUNT, Kind::Unsigned),
        Field::new(f::BASED_PLANE_COUNT, Kind::Unsigned),
        // The province the base sits in (`:15184`). `ContentId` — a province is
        // content-defined, exactly as d20 and d22 class theirs. BOTH empty
        // states, because `anchor_kind` decides which one a zero is: see the
        // module docs.
        Field::new(f::PROVINCE_ID, Kind::ContentId)
            .nullable()
            .unknown_capable(),
        // Which of the three placements this row carries. `Unsigned`: an engine
        // enum this producer defines, not content, and never absent -- every row
        // declares one, and a row that does not is refused rather than mapped to
        // an empty state. `runs/ue_d34_pool_bases/RE_NON_STATE_AIR_BASE.md`.
        Field::new(f::ANCHOR_KIND, Kind::Unsigned),
        // The hosting object's reference pair (`CAirBase+0x60`). The type word is
        // a clausewitz registry id and the id word is that registry's instance
        // id -- for a carrier base the pair is `(CSHIP_TYPE, d29 ship instance
        // id)`, which is the join. Both empty states, for the reason in the
        // module docs.
        Field::new(f::HOST_OBJECT_TYPE, Kind::Unsigned)
            .nullable()
            .unknown_capable(),
        Field::new(f::HOST_OBJECT_ID, Kind::Unsigned)
            .nullable()
            .unknown_capable(),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: AIR_BASES,
            // The fold keys by `air_base_id` into a `BTreeMap` that persists
            // across hours (`world_fold.rs:1870`, `:15192`) and removes keys by
            // the DELETED bit — a keyed set with a delta grammar, which collection-shape contract
            // gives only to `KeyedSet`.
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 36-byte payload header (`world_fold.rs::fold_air_base_status`, writer at
/// `attach_log.rs:71583`-`:71590`). Every word is carried, including the four
/// the fold never reads, so a caller can report them without re-deriving offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_air_base_rows: u32,
    pub observed_pool_rows: u32,
    pub deleted_rows: u32,
    pub payload_version: u32,
}

/// Read the header **without** validating the payload's total length.
///
/// This split exists because the fold's checks are ordered and the order is
/// load-bearing (recipe event-continuity contract: "derive the exclusion's POSITION from your fold's
/// call order"). `fold_air_base_status` runs, in this sequence:
///
/// 1. `need(payload, 36)` and the `payload_version != 1` rejection;
/// 2. `handle_compact_failure` — returns without folding on a failure status;
/// 3. `reject_delta_before_baseline` — returns without folding;
/// 4. only then the exact-length check (`:15161`-`:15168`).
///
/// A record excluded at step 2 or 3 is therefore never length-checked by the
/// recorder, and a caller that validated the length first would refuse a stream
/// the recorder accepts. [`peek_header`] is steps 1's checks alone.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_base_status payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    // `world_fold.rs::fold_air_base_status`: the trailing header word declares the
    // payload version, and only the current one folds.
    let payload_version = u32_at(payload, 32);
    if payload_version != PAYLOAD_VERSION {
        return Err(format!(
            "air_base_status payload version {payload_version} != {PAYLOAD_VERSION}"
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_air_base_rows: u32_at(payload, 20),
        observed_pool_rows: u32_at(payload, 24),
        deleted_rows: u32_at(payload, 28),
        payload_version,
    })
}

/// Split a d34 DATA payload into its header and its rows slice, applying the
/// fold's structural checks (`world_fold.rs::fold_air_base_status`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
///
/// `row_count == 0` is a well-formed payload (a header-only record), and the
/// reference capture contains many: an hour in which nothing changed emits no
/// rows at all.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is input from a file on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough word. The fold
    // saturates instead (`:15161`-`:15162`), which cannot wrap either — both
    // reject; this one reports which word was impossible.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("air_base_status row_count {} overflows", header.row_count))?;
    if payload.len() != expected {
        return Err(format!(
            "air_base_status len {} != {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// One compact row, parsed and validated exactly as the fold's row loop does
/// (`world_fold.rs::fold_air_base_status`) — including the order of its checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AirBaseRow {
    pub air_base_id: u32,
    pub state_id: u32,
    /// `DELETED`: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
    pub capacity: u32,
    pub building_level: u32,
    pub based_wing_count: u32,
    pub based_plane_count: u32,
    /// `None` is the zero sentinel. WHICH empty state that is depends on
    /// `anchor_kind`, and [`AirBaseRow::empty_province`] is the one place that
    /// decides: *null* on a host-anchored row, because the engine derives the
    /// province slot only when the state pointer is set
    /// (`runs/ue_d34_pool_bases/RE_NON_STATE_AIR_BASE.md` §2), and *unknown*
    /// otherwise.
    pub province_id: Option<u32>,
    /// One of [`ANCHOR_KIND_STATE`], [`ANCHOR_KIND_HOST`],
    /// [`ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED`].
    pub anchor_kind: u32,
    /// The hosting object's `(type, id)` pair, `None` on a row with no host.
    pub host: Option<(u32, u32)>,
}

impl AirBaseRow {
    /// Which empty state an absent host is, for THIS row.
    ///
    /// A state-anchored base is *known* to have no host: the engine's two
    /// validity rules are mutually exclusive (`RE_NON_STATE_AIR_BASE.md` §3) and
    /// the recorder refuses a state-anchored row that carries a host pair. That
    /// is [`Value::Null`] — "the relation genuinely points at nothing" — and not
    /// [`Value::Unknown`], which would say the host existed and was not
    /// captured. On a `pool_observed_unresolved` row no air-base object was
    /// reached at all, so *unknown* is exactly right there.
    pub fn empty_host(&self) -> Value {
        match self.anchor_kind {
            ANCHOR_KIND_STATE => Value::Null,
            _ => Value::Unknown,
        }
    }

    /// The owning state, whose empty states are chosen by `anchor_kind`.
    ///
    /// A state-anchored row's id is a VALUE even when it is zero: the fold
    /// applies no zero check to this word, and this mapper may not refuse or
    /// reinterpret a row the fold folds. The other two kinds have no owning
    /// state at all — the host-anchored one because the engine's two validity
    /// rules are mutually exclusive (`RE_NON_STATE_AIR_BASE.md` §3), which is
    /// *null*, and the unresolved one because no air-base object was reached,
    /// which is *unknown*.
    pub fn state(&self) -> Value {
        match self.anchor_kind {
            ANCHOR_KIND_STATE => Value::ContentId(self.state_id as u64),
            ANCHOR_KIND_HOST => Value::Null,
            _ => Value::Unknown,
        }
    }

    /// Which empty state an absent province is, for THIS row.
    ///
    /// The mirror of [`AirBaseRow::empty_host`]: a base with no state row has no
    /// province of its own — the engine derives the slot only from the state
    /// (`RE_NON_STATE_AIR_BASE.md` §2) — so a host-anchored zero is *null*. A
    /// state-anchored zero is a province the recording failed to carry, and an
    /// unresolved row captured nothing; both are *unknown*.
    pub fn empty_province(&self) -> Value {
        match self.anchor_kind {
            ANCHOR_KIND_HOST => Value::Null,
            _ => Value::Unknown,
        }
    }
}

/// Parse and validate one [`ROW_LEN`]-byte row.
///
/// The check order mirrors the fold exactly, because it decides which of two
/// errors a doubly-invalid row reports and, more importantly, that a **deleted**
/// row is still province-range-checked: `:15185` runs before the delete branch
/// at `:15188`. A tombstone carrying a wild province word is a malformed frame
/// to the recorder, so it is one here.
///
/// The fold applies **no** other content rule: `air_base_id == 0` is accepted
/// (unlike d63's `tag_db_id`, `:9279`), `state_id == 0` is accepted, and the
/// four counters have no upper bound. Tightening any of those here would make
/// the mapper refuse a row the recorder folds, which the recipe's precedence
/// rule forbids.
pub fn parse_row(row: &[u8]) -> core::result::Result<AirBaseRow, String> {
    check_row_len(row)?;
    let air_base_id = u32_at(row, 0);
    let state_id = u32_at(row, 4);
    let flags = u32_at(row, 8);
    // `world_fold.rs::fold_air_base_status`: DELETED is the only bit this grammar has.
    if flags & !ROW_FLAG_DELETED != 0 {
        return Err(format!(
            "air_base_status row {air_base_id} has unknown flags {flags:#x}"
        ));
    }
    // `:15184`-`:15187`, before the delete branch.
    let province_word = u32_at(row, 28);
    if province_word > MAX_PROVINCE_ID {
        return Err(format!(
            "air_base_status row {air_base_id} province id {province_word} is out of range \
             (max {MAX_PROVINCE_ID})"
        ));
    }
    let anchor_kind = u32_at(row, 32);
    let host_object_type = u32_at(row, 36);
    let host_object_id = u32_at(row, 40);
    // `world_fold.rs::fold_air_base_status`: an unknown kind is a hard error, and
    // a host-anchored row with no host is one too.
    match anchor_kind {
        ANCHOR_KIND_STATE | ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED => {}
        ANCHOR_KIND_HOST if host_object_type != 0 && host_object_id != 0 => {}
        ANCHOR_KIND_HOST => {
            return Err(format!(
                "air_base_status row {air_base_id} is host-anchored and carries no host"
            ))
        }
        other => {
            return Err(format!(
                "air_base_status row {air_base_id} has unknown anchor kind {other}"
            ))
        }
    }
    Ok(AirBaseRow {
        air_base_id,
        state_id,
        deleted: flags & ROW_FLAG_DELETED != 0,
        capacity: u32_at(row, 12),
        building_level: u32_at(row, 16),
        based_wing_count: u32_at(row, 20),
        based_plane_count: u32_at(row, 24),
        province_id: (province_word != PROVINCE_ID_UNOBTAINED).then_some(province_word),
        anchor_kind,
        host: (anchor_kind == ANCHOR_KIND_HOST).then_some((host_object_type, host_object_id)),
    })
}

/// The key of one compact row: its `air_base_id` (`world_fold.rs:15175`,
/// `:15192`-`:15193`). Valid for deleted rows too — a tombstone names the key
/// it removes.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[parse_row(row)?.air_base_id as u64]))
}

/// One compact row of the `air_bases` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw [`ROW_LEN`]-byte slice (the caller has
/// already consumed the table header via [`split_payload`]); d34 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d34 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_air_base_status`: a row the fold would
/// reject is an `Err` here, never a coerced value. A **deleted** row is a removal
/// and has no record at all — it is an `Err` here and the caller (see
/// [`Accumulator::fold_payload`]) drops the key instead. Nothing in this function
/// can panic on any [`ROW_LEN`]-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d34 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_row(row)?;
    record_from_parsed_row(collection, &parsed)
}

/// [`record_from_row`] on an already-parsed row, so a caller that must branch on
/// [`AirBaseRow::deleted`] does not parse the same bytes twice.
pub fn record_from_parsed_row(
    collection: &Collection,
    parsed: &AirBaseRow,
) -> core::result::Result<Record, String> {
    if parsed.deleted {
        return Err(format!(
            "air_base_status row {} is a DELETED tombstone and has no record",
            parsed.air_base_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::STATE_ID => parsed.state(),
            field::CAPACITY => Value::Unsigned(parsed.capacity as u64),
            field::BUILDING_LEVEL => Value::Unsigned(parsed.building_level as u64),
            field::BASED_WING_COUNT => Value::Unsigned(parsed.based_wing_count as u64),
            field::BASED_PLANE_COUNT => Value::Unsigned(parsed.based_plane_count as u64),
            // Both empty states, chosen by the anchor. See the module docs.
            field::PROVINCE_ID => match parsed.province_id {
                Some(id) => Value::ContentId(id as u64),
                None => parsed.empty_province(),
            },
            field::ANCHOR_KIND => Value::Unsigned(parsed.anchor_kind as u64),
            field::HOST_OBJECT_TYPE => match parsed.host {
                Some((host_object_type, _)) => Value::Unsigned(host_object_type as u64),
                None => parsed.empty_host(),
            },
            field::HOST_OBJECT_ID => match parsed.host {
                Some((_, host_object_id)) => Value::Unsigned(host_object_id as u64),
                None => parsed.empty_host(),
            },
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "air_base_status has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The accumulator: one record is not one state ----------------------------

/// What one folded record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The first record: `replace_or_sparse_after_first` returned `false`, the
    /// accumulator was cleared, and this record's rows are the whole state
    /// (`world_fold.rs:15170`-`:15172`).
    Replace,
    /// A sparse patch onto the retained accumulator (`:15169`).
    Sparse,
    /// `reject_delta_before_baseline` fired (`:15154`-`:15160`): the fold marked
    /// the domain-hour unknown, folded nothing, and RETAINED the accumulator. The
    /// hour carries no d34 state and must not be measured.
    DeltaBeforeBaseline,
}

/// The `air_base_status` accumulator (`world_fold.rs:1870`), folded exactly as
/// `fold_air_base_status` folds it.
///
/// This is the recipe's fourth public boundary: d34's state is not one record,
/// so the measurement harness, the tests and any future caller share ONE
/// implementation of clear-on-first, upsert-on-sparse and tombstone-removes
/// rather than each rebuilding it.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `air_bases` collection's current content, keyed by `air_base_id`.
    pub bases: KeyedState,
    baseline_seen: bool,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a replacing record has been folded — `delta_baselines_seen`
    /// membership in `world_fold.rs::replace_or_sparse_after_first`, which is what decides both
    /// whether an arriving DELTA is data and whether the next record clears.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d34 DATA payload, exactly as `fold_air_base_status` does from
    /// `:15151` onward.
    ///
    /// `is_delta` is the caller's reading of `COMPACT_STATUS_DELTA_DATA` in
    /// `header.status_flags`. It is a parameter rather than a bit tested here
    /// because that constant belongs to `compact_contract` and d34 is not
    /// admitted to the production-codec dependency surface. Copying the literal
    /// into this module is exactly the drift the recipe forbids.
    ///
    /// **The caller must already have excluded failure-status records.** The
    /// fold checks `handle_compact_failure` first (`:15151`-`:15153`) and returns
    /// having folded nothing; that check needs `compact_contract`'s predicate, so
    /// it belongs to the caller. Everything after it is here, in the fold's own
    /// order — note that the delta-before-baseline exclusion precedes the
    /// exact-length check, so a delta-before-baseline record is never
    /// length-validated.
    ///
    /// On `Err` the accumulator is left exactly as it was. The fold instead
    /// clears and applies rows as it validates them, so a row error there leaves
    /// a half-applied map — but a fold `Err` aborts the record's processing
    /// entirely, so that half-applied state is never read, and refusing to
    /// mutate is the strictly safer of two unobservable behaviours.
    ///
    /// A key repeated inside one frame is **last-writer-wins in row order**,
    /// including an insert followed by a tombstone, because that is what the
    /// fold's `insert`/`remove` loop does (`:15188`-`:15203`). It is not a
    /// rejected frame: the fold applies no uniqueness rule, and the collector
    /// cannot produce one anyway (a base is either in `current` or deleted,
    /// `attach_log.rs:55826`-`:55844`).
    pub fn fold_payload(
        &mut self,
        collection: &Collection,
        payload: &[u8],
        is_delta: bool,
    ) -> core::result::Result<FoldOutcome, String> {
        // `world_fold.rs::fold_air_base_status`, BEFORE the length check at `world_fold.rs::fold_air_base_status`.
        peek_header(payload)?;
        if is_delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }
        let (_header, rows) = split_payload(payload)?;

        // Parse and map every row before mutating anything (see the doc comment).
        let mut parsed: Vec<(Key, Option<Record>)> = Vec::with_capacity(rows.len() / ROW_LEN);
        for row in rows.chunks_exact(ROW_LEN) {
            let r = parse_row(row)?;
            let k = key(&[r.air_base_id as u64]);
            if r.deleted {
                parsed.push((k, None));
            } else {
                parsed.push((k, Some(record_from_parsed_row(collection, &r)?)));
            }
        }

        // `world_fold.rs:15169`-`:15172` — `replace_or_sparse_after_first` with
        // no expected-partial tolerance for domain 34 (`compact_contract`'s
        // `COMPACT_EXPECTED_PARTIAL_TOLERANCES` lists 26, 27, 28 and 54 only), so
        // the whole rule is: a delta is sparse, and otherwise the first record
        // replaces and every later one is sparse.
        let sparse = if is_delta || self.baseline_seen {
            true
        } else {
            self.baseline_seen = true;
            false
        };
        if !sparse {
            self.bases.clear();
        }
        for (k, record) in parsed {
            match record {
                Some(record) => {
                    self.bases.insert(k, record);
                }
                None => {
                    self.bases.remove(&k);
                }
            }
        }
        Ok(if sparse {
            FoldOutcome::Sparse
        } else {
            FoldOutcome::Replace
        })
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_base_status row length {} != {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach this only after a length check, so the slice is in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{diff, key as make_key};
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw row, packed the way the collector packs one
    /// (`attach_log.rs::compact_air_base_status_data_payload`) — note that
    /// `province_id` is not adjacent to `state_id` but sits after the two
    /// occupancy counters, and the anchor triple is last.
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        air_base_id: u32,
        state_id: u32,
        flags: u32,
        capacity: u32,
        building_level: u32,
        based_wing_count: u32,
        based_plane_count: u32,
        province_id: u32,
    ) -> Vec<u8> {
        raw_row_anchored(
            air_base_id,
            state_id,
            flags,
            capacity,
            building_level,
            based_wing_count,
            based_plane_count,
            province_id,
            ANCHOR_KIND_STATE,
            0,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn raw_row_anchored(
        air_base_id: u32,
        state_id: u32,
        flags: u32,
        capacity: u32,
        building_level: u32,
        based_wing_count: u32,
        based_plane_count: u32,
        province_id: u32,
        anchor_kind: u32,
        host_object_type: u32,
        host_object_id: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [
            air_base_id,
            state_id,
            flags,
            capacity,
            building_level,
            based_wing_count,
            based_plane_count,
            province_id,
            anchor_kind,
            host_object_type,
            host_object_id,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// Every field a distinct value, so a transposition cannot pass.
    fn full_row() -> Vec<u8> {
        raw_row(407, 1081, 0, 2000, 10, 23, 1995, 13367)
    }

    /// The shape lane UE added: no state, no province, a hosting object.
    fn host_row() -> Vec<u8> {
        raw_row_anchored(403, 0, 0, 12, 0, 1, 1, 0, ANCHOR_KIND_HOST, 51, 4211)
    }

    fn payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        payload_with_counts(status_flags, rows, rows.len() as u32, rows.len() as u32)
    }

    fn payload_with_counts(
        status_flags: u32,
        rows: &[Vec<u8>],
        row_count: u32,
        changed_rows: u32,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&changed_rows.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // observed_air_base_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // observed_pool_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // deleted_rows
        p.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        assert_eq!(p.len(), HEADER_LEN);
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    // --- Step 8.1 -----------------------------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d34 schema is well-formed");
    }

    // --- Step 8.5 -----------------------------------------------------------

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_air_base() {
        let c = collection();
        assert_eq!(
            c.shape,
            Shape::KeyedSet,
            "the first record replaces, later records upsert and remove"
        );
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by air_base_id alone");
        assert_eq!(keys[0].field_id, field::AIR_BASE_ID);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "a runtime engine handle, not content a manifest resolves"
        );
        assert!(keys[0].kind.legal_as_key());
        // A key member must be concrete: never optional, nullable, unknown or
        // derived (recipe step 5).
        assert!(!keys[0].optional && !keys[0].nullable && !keys[0].unknown_capable);
        assert!(keys[0].derived.is_none());
        assert_eq!(c.fields.len(), 10, "1 key + 9 carried values");
    }

    // --- Step 8.3: the states the schema declares, through the mapper --------

    /// The two empty states are declared on exactly the four fields that reach
    /// them, and `optional` on none.
    ///
    /// `optional` would claim an air base can lack the CONCEPT of a placement.
    /// Every row is an air base and every air base has one, so nothing here is
    /// ever inapplicable. `anchor_kind` carries neither state: every row
    /// declares one, and a row that does not is refused by [`parse_row`] rather
    /// than mapped to an empty value.
    #[test]
    fn both_empty_states_are_declared_on_exactly_the_four_placement_fields() {
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        let placement = vec![
            field::STATE_ID,
            field::PROVINCE_ID,
            field::HOST_OBJECT_TYPE,
            field::HOST_OBJECT_ID,
        ];
        let nullable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.nullable)
            .map(|f| f.field_id)
            .collect();
        let unknown_capable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(nullable, placement);
        assert_eq!(unknown_capable, placement);
    }

    /// *null* and *unknown* are selected by `anchor_kind`, and never collapsed.
    ///
    /// The table in the module docs, driven through the real mapper from raw row
    /// bytes. Each cell is a different sentence about the world, and this is the
    /// test that fails if any two of them become the same value.
    ///
    /// kills: mapping an absent host to `Unknown` on a state-anchored row (the
    /// engine guarantees a state base has no host, so that would claim a host
    /// existed and was missed); mapping a host-anchored zero province to
    /// `Unknown` (same claim, one relation over); mapping either to a real zero;
    /// and dropping `.nullable()` from the schema, which makes the whole mapping
    /// unrepresentable.
    #[test]
    fn the_empty_state_of_a_placement_is_chosen_by_the_anchor() {
        let c = collection();
        let state = index_of(&c, field::STATE_ID);
        let province = index_of(&c, field::PROVINCE_ID);
        let host_type = index_of(&c, field::HOST_OBJECT_TYPE);
        let host_id = index_of(&c, field::HOST_OBJECT_ID);

        // State-anchored: state and province are real readings, the host is
        // KNOWN absent.
        let state_row = record_from_row(&c, &full_row(), 0).expect("maps");
        assert_eq!(state_row.values[state], Value::ContentId(1081));
        assert_eq!(state_row.values[province], Value::ContentId(13367));
        assert_eq!(state_row.values[host_type], Value::Null);
        assert_eq!(state_row.values[host_id], Value::Null);

        // State-anchored with no province: the base HAS one and the recording
        // did not carry it. Its state id stays a value — including a zero one,
        // which is `a_real_zero_is_a_value_not_an_empty_state`'s subject.
        let unobtained = raw_row(407, 1081, 0, 2000, 10, 23, 1995, PROVINCE_ID_UNOBTAINED);
        let state_zero = record_from_row(&c, &unobtained, 0).expect("maps");
        assert_eq!(state_zero.values[province], Value::Unknown);
        assert_eq!(state_zero.values[host_type], Value::Null);

        // Host-anchored: the state and the province are KNOWN absent, the host
        // is measured. A `ContentId(0)` in either would be a fabricated state.
        let host = record_from_row(&c, &host_row(), 0).expect("maps");
        assert_eq!(host.values[state], Value::Null);
        assert_eq!(host.values[province], Value::Null);
        assert_eq!(host.values[host_type], Value::Unsigned(51));
        assert_eq!(host.values[host_id], Value::Unsigned(4211));

        // Pool-observed unresolved: nothing was reached, so nothing is known.
        let unresolved = raw_row_anchored(
            403,
            0,
            0,
            0,
            0,
            1,
            1,
            0,
            ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED,
            0,
            0,
        );
        let unreached = record_from_row(&c, &unresolved, 0).expect("maps");
        assert_eq!(unreached.values[state], Value::Unknown);
        assert_eq!(unreached.values[province], Value::Unknown);
        assert_eq!(unreached.values[host_type], Value::Unknown);
        assert_eq!(unreached.values[host_id], Value::Unknown);

        // The four states stay four states.
        assert_ne!(Value::Null, Value::Unknown);
    }

    /// A row carrying BOTH empty states survives the encoder.
    ///
    /// `nullable` and `unknown_capable` are declarations, and a declaration the
    /// encoder cannot represent is worse than no declaration at all: the mapper
    /// would build records that fail to encode. A host row carries *null* in two
    /// fields and real values in two others, so it exercises the pair.
    ///
    /// kills: dropping `.nullable()` from any of the four placement fields.
    #[test]
    fn a_host_row_carrying_both_empty_states_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &host_row(), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(back, record, "decode reproduces the record");

        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn an_unobtained_province_is_unknown_through_the_mapper() {
        // Built from raw row BYTES, not a hand-assembled Vec<Value>: this is the
        // test that would fail against a mapper emitting Null, Absent or a real
        // ContentId(0) for the sentinel.
        let c = collection();
        let r = record_from_row(&c, &raw_row(9, 5, 0, 1, 2, 3, 4, PROVINCE_ID_UNOBTAINED), 0)
            .expect("maps");
        assert_eq!(r.values[index_of(&c, field::PROVINCE_ID)], Value::Unknown);
        // Nothing else in the row is disturbed by the sentinel.
        assert_eq!(
            r.values[index_of(&c, field::STATE_ID)],
            Value::ContentId(5)
        );
        assert_eq!(r.values[index_of(&c, field::CAPACITY)], Value::Unsigned(1));
        assert_eq!(parse_row(&raw_row(9, 5, 0, 1, 2, 3, 4, 0)).unwrap().province_id, None);

        // And the smallest real province is a value, not the sentinel: the
        // boundary is exactly at 0, not "small ids are missing".
        let r = record_from_row(&c, &raw_row(9, 5, 0, 1, 2, 3, 4, 1), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE_ID)],
            Value::ContentId(1)
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // The row the collector packs for a real, complete air base that happens
        // to have no capacity, no building level and nothing based on it: every
        // one of those zeros is a measured fact. record-state contract keeps zero out of the three
        // empty states. `state_id` zero is included because the FOLD applies no
        // zero check to it (`:15176`) even though the collector's read does
        // (`attach_log.rs:56161`) — the fold is the authority.
        let c = collection();
        let r = record_from_row(&c, &raw_row(0, 0, 0, 0, 0, 0, 0, 77), 0).expect("maps");
        for (id, expected) in [
            (field::STATE_ID, Value::ContentId(0)),
            (field::CAPACITY, Value::Unsigned(0)),
            (field::BUILDING_LEVEL, Value::Unsigned(0)),
            (field::BASED_WING_COUNT, Value::Unsigned(0)),
            (field::BASED_PLANE_COUNT, Value::Unsigned(0)),
        ] {
            assert_eq!(r.values[index_of(&c, id)], expected, "field {id}");
        }
        // `air_base_id == 0` is a key, not a rejected row (the fold has no check).
        assert_eq!(key_from_row(&raw_row(0, 0, 0, 0, 0, 0, 0, 77)).unwrap(), make_key(&[0]));
    }

    #[test]
    fn a_deleted_row_is_a_tombstone_and_never_a_record() {
        // The one state the mapper refuses to invent. A DELETED row carries a
        // zeroed capacity/level/occupancy and a RETAINED state and province
        // (`attach_log.rs:55855`-`:55868`) — mapping it would publish a live air
        // base with four zeroed counters for an air base that no longer exists.
        let c = collection();
        let row = raw_row(407, 1081, ROW_FLAG_DELETED, 0, 0, 0, 0, 13367);
        let parsed = parse_row(&row).expect("a tombstone is a well-formed row");
        assert!(parsed.deleted);
        assert_eq!(parsed.state_id, 1081, "the fold reads the retained words");
        assert_eq!(parsed.province_id, Some(13367));
        assert!(record_from_row(&c, &row, 0).is_err());
        // But its key is readable — that is the whole point of a tombstone.
        assert_eq!(key_from_row(&row).expect("keys"), make_key(&[407]));
    }

    // --- Step 8.4: quantization ----------------------------------------------

    #[test]
    fn there_is_no_continuous_field_to_quantize() {
        // Step 4's "quantization saves bytes" test does not apply and cannot: d34
        // has no `Kind::Continuous` field at all, because the fold applies no
        // scale factor to anything (`world_fold.rs::fold_air_base_status` stores six
        // u32 words verbatim, and the header carries no scale word to assert).
        // Every value is an exact integer count or id, so there is nothing that
        // is "drawn rather than interpolated".
        let c = collection();
        assert_eq!(
            c.value_fields()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
        assert!(
            c.fields.iter().all(|f| !f.quant.is_quantized()),
            "no integer field may declare a quantizer"
        );
        assert!(c.fields.iter().all(|f| f.quant == Quant::None));
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d34 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        let a = record_from_row(&c, &full_row(), 0).expect("maps");
        for tick in [1u64, 7, 1 << 40, u64::MAX] {
            assert_eq!(
                record_from_row(&c, &full_row(), tick).expect("maps"),
                a,
                "frame_tick {tick} must not change any value"
            );
        }
    }

    // --- Step 8.2: byte-identical round trip ---------------------------------

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &full_row(), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(back, record, "decode reproduces the record");

        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn the_unknown_province_roundtrips_byte_for_byte() {
        let c = collection();
        let record =
            record_from_row(&c, &raw_row(407, 1081, 0, 2000, 10, 23, 1995, 0), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(
            back.values[index_of(&c, field::PROVINCE_ID)],
            Value::Unknown,
            "the empty state survives the codec"
        );
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    // --- Step 8.6: every field's offset --------------------------------------

    #[test]
    fn every_field_reads_its_own_offset_unscaled() {
        // The transposition test. d34 has no scale factor, so the special case
        // step 6 names (32768-vs-100000) cannot arise — but adjacent u32 words
        // are exactly the d62 `owner`/`actor` failure class, so each of the seven
        // words is pinned to a distinct value and checked one at a time.
        let c = collection();
        let row = raw_row(11, 22, 0, 33, 44, 55, 66, 77);
        let parsed = parse_row(&row).expect("parses");
        assert_eq!(parsed.air_base_id, 11, "air_base_id at +0");
        assert_eq!(parsed.state_id, 22, "state_id at +4");
        assert!(!parsed.deleted, "flags at +8");
        assert_eq!(parsed.capacity, 33, "capacity at +12");
        assert_eq!(parsed.building_level, 44, "building_level at +16");
        assert_eq!(parsed.based_wing_count, 55, "based_wing_count at +20");
        assert_eq!(parsed.based_plane_count, 66, "based_plane_count at +24");
        assert_eq!(parsed.province_id, Some(77), "province_id at +28");

        let r = record_from_row(&c, &row, 0).expect("maps");
        for (id, expected) in [
            (field::STATE_ID, Value::ContentId(22)),
            (field::CAPACITY, Value::Unsigned(33)),
            (field::BUILDING_LEVEL, Value::Unsigned(44)),
            (field::BASED_WING_COUNT, Value::Unsigned(55)),
            (field::BASED_PLANE_COUNT, Value::Unsigned(66)),
            (field::PROVINCE_ID, Value::ContentId(77)),
        ] {
            assert_eq!(r.values[index_of(&c, id)], expected, "field {id}");
        }
        assert_eq!(key_from_row(&row).expect("keys"), make_key(&[11]));

        // And "unscaled" as a claim, not an absence: a word that a 100_000 or
        // 32768 divisor would mangle reads back as itself.
        let r = record_from_row(&c, &raw_row(1, 100_000, 0, 32_768, 1, 1, 100_000, 13_413), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::STATE_ID)],
            Value::ContentId(100_000),
            "no divisor is applied to any field"
        );
        assert_eq!(
            r.values[index_of(&c, field::CAPACITY)],
            Value::Unsigned(32_768)
        );
        assert_eq!(
            r.values[index_of(&c, field::BASED_PLANE_COUNT)],
            Value::Unsigned(100_000)
        );
        // u32::MAX in every unbounded word: the fold has no upper bound on the
        // four counters or on `state_id`, so neither has this.
        let r = record_from_row(
            &c,
            &raw_row(u32::MAX, u32::MAX, 0, u32::MAX, u32::MAX, u32::MAX, u32::MAX, MAX_PROVINCE_ID),
            0,
        )
        .expect("the fold bounds none of these");
        assert_eq!(
            r.values[index_of(&c, field::BASED_PLANE_COUNT)],
            Value::Unsigned(u32::MAX as u64)
        );
    }

    // --- Rejection surface ---------------------------------------------------

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(parse_row(&row).is_err(), "length {len}");
            assert!(key_from_row(&row).is_err(), "length {len}");
        }
        // Every flag bit that is not DELETED (`world_fold.rs::fold_air_base_status`), one at a
        // time, with and without DELETED also set.
        for bit in 1..32u32 {
            for base in [0u32, ROW_FLAG_DELETED] {
                let row = raw_row(1, 1, base | (1 << bit), 0, 0, 0, 0, 1);
                assert!(
                    parse_row(&row).is_err(),
                    "flag bit {bit} (base {base}) is not DELETED and must be rejected"
                );
                assert!(record_from_row(&c, &row, 0).is_err());
                assert!(key_from_row(&row).is_err());
            }
        }
        // Province id out of range (`:15185`), at the bound and one past it.
        assert!(parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, MAX_PROVINCE_ID)).is_ok());
        // kills: d34.rs MAX_PROVINCE_ID reverted to the vanilla province count
        // 13_413, OR set to any other value below the shared 65_535 envelope.
        // The literals are deliberate: a test spelled only in terms of the
        // constant passes for every value the constant could take, which is
        // exactly the change it exists to catch. 13_414 kills the vanilla count;
        // 65_535 kills every ceiling below the envelope, including the 20_000
        // that d62 used to carry.
        assert!(
            parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, 13_414)).is_ok(),
            "13_414 is one past the VANILLA province count, not past this build's \
             representation envelope; refusing it is the defect this bound had"
        );
        assert!(
            parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, 65_535)).is_ok(),
            "65_535 was the retired ceiling and is an ordinary province id"
        );
        assert!(
            parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, 65_536)).is_ok(),
            "65_536 is the FIRST id the retired 65_535 ceiling discarded, which              is exactly the loss that ceiling caused on any map above it"
        );
        assert!(
            parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, 1_000_000)).is_ok(),
            "1_000_000 was a THIRD spelling of this same ceiling elsewhere in              the tree; one id space cannot have four ceilings"
        );
        // `2_147_483_647` is spelled out for the reason the derived probe
        // cannot cover: `MAX_PROVINCE_ID + 1` tracks the constant, so widening
        // the ceiling by one carries the probe along and this arm stays green.
        for bad in [MAX_PROVINCE_ID + 1, 2_147_483_647, u32::MAX] {
            assert!(
                parse_row(&raw_row(1, 1, 0, 0, 0, 0, 0, bad)).is_err(),
                "province {bad} exceeds {MAX_PROVINCE_ID}"
            );
        }
        // The range check precedes the delete branch (`:15185` before `:15188`),
        // so a TOMBSTONE with a wild province is rejected too.
        assert!(
            parse_row(&raw_row(1, 1, ROW_FLAG_DELETED, 0, 0, 0, 0, MAX_PROVINCE_ID + 1)).is_err(),
            "a deleted row is still province-range-checked"
        );
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0).is_err());
    }

    // --- Header ---------------------------------------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![full_row()];
        let p = payload(0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(body, &rows[0][..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(peek_header(&p[..len]).is_err(), "len {len}");
        }
        // A length that does not match the row count, both directions.
        let mut truncated = p.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        let mut over = p.clone();
        over.push(0);
        assert!(split_payload(&over).is_err());
        // Any version word but the current one
        // (`world_fold.rs::fold_air_base_status`). Zero is the version-0 shape
        // whose rows were four bytes shorter and carried no anchor.
        for version in [0u32, 2, 3, u32::MAX] {
            let mut bad = p.clone();
            bad[32..36].copy_from_slice(&version.to_le_bytes());
            assert!(peek_header(&bad).is_err(), "version {version}");
            assert!(split_payload(&bad).is_err(), "version {version}");
        }
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());

        // A header-only record (`row_count == 0`) is well-formed and common: the
        // reference capture emits one whenever nothing changed that hour.
        let empty = payload(0, &[]);
        let (header, body) = split_payload(&empty).expect("row_count 0 is legal");
        assert_eq!(header.row_count, 0);
        assert!(body.is_empty());
    }

    #[test]
    fn changed_rows_is_carried_but_never_enforced() {
        // d63's fold asserts `changed_rows == row_count` (`:9267`); THIS fold
        // never reads the word (`:15145`-`:15147` read only rows, status, the
        // pool count and the version). Enforcing it here would refuse a payload
        // the recorder folds.
        let p = payload_with_counts(0, &[full_row()], 1, 0);
        let (header, _) = split_payload(&p).expect("accepted");
        assert_eq!(header.changed_rows, 0);
        assert_eq!(header.row_count, 1);
        let p = payload_with_counts(0, &[full_row()], 1, u32::MAX);
        assert!(split_payload(&p).is_ok());
    }

    #[test]
    fn the_header_words_the_fold_ignores_are_still_reported_at_their_offsets() {
        let mut p = payload(0, &[]);
        p[0..8].copy_from_slice(&123_456u64.to_le_bytes());
        p[12..16].copy_from_slice(&7u32.to_le_bytes());
        p[16..20].copy_from_slice(&0x40Cu32.to_le_bytes());
        p[20..24].copy_from_slice(&374u32.to_le_bytes());
        p[24..28].copy_from_slice(&302u32.to_le_bytes());
        p[28..32].copy_from_slice(&5u32.to_le_bytes());
        let h = peek_header(&p).expect("reads");
        assert_eq!(h.elapsed_wall_ms, 123_456);
        assert_eq!(h.changed_rows, 7);
        assert_eq!(h.status_flags, 0x40C);
        assert_eq!(h.observed_air_base_rows, 374);
        assert_eq!(h.observed_pool_rows, 302);
        assert_eq!(h.deleted_rows, 5);
        assert_eq!(h.payload_version, PAYLOAD_VERSION);
    }

    /// A host-anchored row maps onto the anchor triple, and neither half of the
    /// pair may be missing.
    ///
    /// kills: reading either host word from the wrong offset; dropping the
    /// host-anchored-with-no-host refusal; admitting an anchor kind the grammar
    /// does not define.
    #[test]
    fn the_anchor_triple_round_trips_and_a_hostless_host_row_is_refused() {
        let c = collection();
        let parsed = parse_row(&host_row()).expect("a host row parses");
        assert_eq!(parsed.anchor_kind, ANCHOR_KIND_HOST);
        assert_eq!(parsed.host, Some((51, 4211)));
        assert_eq!(parsed.province_id, None);
        let record = record_from_row(&c, &host_row(), 0).expect("maps");
        assert_eq!(
            record.values[index_of(&c, field::ANCHOR_KIND)],
            Value::Unsigned(ANCHOR_KIND_HOST as u64)
        );
        assert_eq!(
            record.values[index_of(&c, field::HOST_OBJECT_TYPE)],
            Value::Unsigned(51)
        );
        assert_eq!(
            record.values[index_of(&c, field::HOST_OBJECT_ID)],
            Value::Unsigned(4211)
        );

        let state_record = record_from_row(&c, &full_row(), 0).expect("maps");
        assert_eq!(
            state_record.values[index_of(&c, field::ANCHOR_KIND)],
            Value::Unsigned(ANCHOR_KIND_STATE as u64)
        );
        // What the state row's EMPTY host words are is
        // `the_empty_state_of_a_placement_is_chosen_by_the_anchor`'s subject.

        // Host-anchored with either half of the pair missing.
        for (host_type, host_id) in [(0u32, 4211u32), (51, 0), (0, 0)] {
            let row = raw_row_anchored(403, 0, 0, 12, 0, 1, 1, 0, ANCHOR_KIND_HOST, host_type, host_id);
            assert!(
                parse_row(&row).is_err(),
                "host-anchored ({host_type}, {host_id}) must be refused"
            );
        }
        // An anchor kind the grammar does not define.
        for kind in [3u32, 4, u32::MAX] {
            let row = raw_row_anchored(403, 0, 0, 12, 0, 1, 1, 0, kind, 0, 0);
            assert!(parse_row(&row).is_err(), "anchor kind {kind} must be refused");
        }
    }

    // --- The accumulator ------------------------------------------------------

    #[test]
    fn the_first_record_replaces_and_later_records_patch() {
        let c = collection();
        let mut acc = Accumulator::new();
        assert!(!acc.baseline_seen());

        // Record 1: the complete snapshot.
        let first = payload(
            0,
            &[
                raw_row(1, 10, 0, 100, 1, 0, 0, 501),
                raw_row(2, 20, 0, 200, 2, 1, 30, 502),
            ],
        );
        assert_eq!(
            acc.fold_payload(&c, &first, false).expect("folds"),
            FoldOutcome::Replace
        );
        assert!(acc.baseline_seen());
        assert_eq!(acc.bases.len(), 2);

        // Record 2: a sparse patch that changes ONE base and omits the other.
        // The omitted row must keep its previous value — the property that makes
        // this domain's state the accumulator rather than the record.
        let second = payload(0, &[raw_row(2, 20, 0, 200, 3, 1, 30, 502)]);
        assert_eq!(
            acc.fold_payload(&c, &second, false).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.bases.len(), 2, "the omitted base is retained");
        let one = &acc.bases[&make_key(&[1])];
        assert_eq!(one.values[index_of(&c, field::CAPACITY)], Value::Unsigned(100));
        let two = &acc.bases[&make_key(&[2])];
        assert_eq!(
            two.values[index_of(&c, field::BUILDING_LEVEL)],
            Value::Unsigned(3),
            "the carried row is upserted"
        );

        // Record 3: a header-only record changes nothing at all.
        let third = payload(0, &[]);
        assert_eq!(
            acc.fold_payload(&c, &third, false).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.bases.len(), 2);

        // Record 4: a tombstone removes its key and nothing else.
        let fourth = payload(0, &[raw_row(1, 10, ROW_FLAG_DELETED, 0, 0, 0, 0, 501)]);
        acc.fold_payload(&c, &fourth, false).expect("folds");
        assert_eq!(acc.bases.len(), 1);
        assert!(!acc.bases.contains_key(&make_key(&[1])));
        assert!(acc.bases.contains_key(&make_key(&[2])));

        // A second non-delta record does NOT clear: only the FIRST one does
        // (`world_fold.rs::replace_or_sparse_after_first`). This is the property that separates
        // `replace_or_sparse_after_first` from d22's clear-on-every-baseline.
        let fifth = payload(0, &[raw_row(3, 30, 0, 1, 1, 1, 1, 503)]);
        acc.fold_payload(&c, &fifth, false).expect("folds");
        assert_eq!(acc.bases.len(), 2, "base 2 survived a later non-delta record");
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing_and_is_not_length_checked() {
        // `reject_delta_before_baseline` (`:15154`-`:15160`) runs BEFORE the
        // exact-length check (`:15161`), so a delta-before-baseline record is
        // reported as such even when its length is impossible — a caller that
        // validated the length first would refuse a stream the recorder accepts.
        let c = collection();
        let mut acc = Accumulator::new();
        let mut malformed = payload(0, &[full_row()]);
        malformed.pop();
        assert_eq!(
            acc.fold_payload(&c, &malformed, true).expect("folds nothing"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(acc.bases.is_empty());
        assert!(!acc.baseline_seen(), "nothing was folded, so nothing was seen");

        // The same record after a baseline IS length-checked.
        acc.fold_payload(&c, &payload(0, &[full_row()]), false)
            .expect("baseline");
        assert!(acc.fold_payload(&c, &malformed, true).is_err());

        // But a header that peek_header itself rejects is an error even before
        // the delta check, because `need()` and the reserved check precede
        // `reject_delta_before_baseline` in the fold (`:15140`-`:15150`).
        let mut fresh = Accumulator::new();
        let mut bad_version = payload(0, &[]);
        bad_version[32..36].copy_from_slice(&0u32.to_le_bytes());
        assert!(fresh.fold_payload(&c, &bad_version, true).is_err());
        assert!(fresh.fold_payload(&c, &[], true).is_err());
    }

    #[test]
    fn a_delta_flagged_first_record_is_sparse_not_a_replace() {
        // `replace_or_sparse_after_first` returns `true` for a DELTA record even
        // when no baseline was seen (`:2954`) — but `reject_delta_before_baseline`
        // has already sent that case away, so the reachable combination is a
        // DELTA record AFTER a baseline. It must patch, never clear.
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[raw_row(1, 10, 0, 1, 1, 1, 1, 501)]), false)
            .expect("baseline");
        assert_eq!(
            acc.fold_payload(&c, &payload(0, &[raw_row(2, 20, 0, 2, 2, 2, 2, 502)]), true)
                .expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.bases.len(), 2);
    }

    #[test]
    fn a_repeated_key_in_one_frame_is_last_writer_wins_in_row_order() {
        // The fold applies no uniqueness rule: it inserts and removes in row
        // order (`:15188`-`:15203`), unlike d63 whose duplicate `tag_db_id` is a
        // rejected frame (`:9383`). Rejecting here would refuse a frame the
        // recorder folds.
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    raw_row(1, 10, 0, 100, 1, 0, 0, 501),
                    raw_row(1, 10, 0, 900, 9, 0, 0, 501),
                ],
            ),
            false,
        )
        .expect("folds");
        assert_eq!(acc.bases.len(), 1);
        assert_eq!(
            acc.bases[&make_key(&[1])].values[index_of(&c, field::CAPACITY)],
            Value::Unsigned(900),
            "the last row in the frame wins"
        );

        // Insert-then-delete in one frame removes the key; delete-then-insert
        // leaves it present. Order, not row kind, decides.
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    raw_row(1, 10, 0, 100, 1, 0, 0, 501),
                    raw_row(1, 10, ROW_FLAG_DELETED, 0, 0, 0, 0, 501),
                ],
            ),
            false,
        )
        .expect("folds");
        assert!(acc.bases.is_empty());

        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    raw_row(1, 10, ROW_FLAG_DELETED, 0, 0, 0, 0, 501),
                    raw_row(1, 10, 0, 100, 1, 0, 0, 501),
                ],
            ),
            false,
        )
        .expect("folds");
        assert_eq!(acc.bases.len(), 1);
    }

    #[test]
    fn a_tombstone_in_the_first_record_is_a_harmless_no_op() {
        // The first record clears before applying, so a tombstone in it removes
        // nothing. The collector never emits one there
        // (`attach_log.rs:55832`-`:55844` skips deletes on a full snapshot), but
        // the fold would accept it and so must this.
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    raw_row(1, 10, ROW_FLAG_DELETED, 0, 0, 0, 0, 501),
                    raw_row(2, 20, 0, 1, 1, 1, 1, 502),
                ],
            ),
            false,
        )
        .expect("folds");
        assert_eq!(acc.bases.len(), 1);
        assert!(acc.bases.contains_key(&make_key(&[2])));
    }

    #[test]
    fn an_error_leaves_the_accumulator_exactly_as_it_was() {
        // A malformed row must not half-apply the frame. (The fold itself clears
        // and applies as it validates, so its post-error map is half-applied —
        // but a fold `Err` aborts the record entirely, so that state is never
        // read, and refusing to mutate is the safer of two unobservable
        // behaviours. Recorded as an intentional divergence.)
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[raw_row(1, 10, 0, 100, 1, 0, 0, 501)]), false)
            .expect("baseline");
        let before = acc.bases.clone();
        let bad = payload(
            0,
            &[
                raw_row(2, 20, 0, 1, 1, 1, 1, 502),
                raw_row(3, 30, 1 << 5, 1, 1, 1, 1, 503), // illegal flag bit
            ],
        );
        assert!(acc.fold_payload(&c, &bad, false).is_err());
        assert_eq!(acc.bases, before, "no row of a rejected frame was applied");
        assert!(diff(&c, &before, &acc.bases).is_empty());
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: this is a public entry point reached from a
        // file on disk and must return a diagnosis, never unwind.
        let c = collection();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 200;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &bytes, state & 1 == 0);
            let _ = split_payload(&bytes);
            let _ = peek_header(&bytes);
            if bytes.len() >= ROW_LEN {
                let _ = parse_row(&bytes[..ROW_LEN]);
                let _ = record_from_row(&c, &bytes[..ROW_LEN], state);
                let _ = key_from_row(&bytes[..ROW_LEN]);
            }
        }
    }
}
