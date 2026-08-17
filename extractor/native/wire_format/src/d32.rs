//! d32 `air_wing_loadouts` — per-air-wing air-base placement and equipment banks.
//!
//! Derived from `world_fold.rs:20261`-`:20402`
//! (`fold_air_wing_loadouts`), its payload constants at `:979`-`:981`, its
// @world-fold-authority domain=32 functions=fold_air_wing_loadouts lines=20261-20402 sha256=a5d78d0723cbeb2867a22cfc4b51f3a8f7b279223a598274998d6a73bcd141b4 closure=0a8d276741e28d447fb45b8f4a81dcc173f13141df94822f230db0e827567cb2
//! accumulator's JSON projection (`world_fold.rs::build_order_of_battle`), the end-of-hour orphan prune
//! that also writes that accumulator (`world_fold.rs::prune_air_wing_loadout_orphans_for_current_hour`), and the collector read
//! and packer that produce the rows
//! (`extractor/native/collector/src/attach_log.rs:57153`-`:57193`,
//! `:57391`-`:57463`, `:55966`-`:56045`, `:70227`-`:70283`).
//! There is no `extractor/documents/domains/d32_*.md` dossier, so the fold and
//! the collector read are the whole basis; where they could be read differently
//! the fold wins and the disagreement is recorded.
//!
//! # Payload grammar (`world_fold.rs::fold_air_wing_loadouts`)
//!
//! A 44-byte header, then `row_count` fixed 32-byte **wing rows**, then
//! `equipment_count` fixed 16-byte **entry rows**. The fold requires the length
//! to be exactly `44 + row_count * 32 + equipment_count * 16` (`world_fold.rs::fold_air_wing_loadouts`),
//! so no legacy short row exists and the length selects nothing.
//!
//! Header:
//!
//! | offset | width | field |
//! |---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` |
//! | `+8`  | u32 | `row_count` |
//! | `+12` | u32 | `changed_rows` |
//! | `+16` | u32 | `status_flags` |
//! | `+20` | u32 | `observed_air_wing_rows` |
//! | `+24` | u32 | `observed_equipment_rows` |
//! | `+28` | u32 | `equipment_count` |
//! | `+32` | u32 | `deleted_rows` |
//! | `+36` | u32 | `scale_raw` (asserted `== 100000`, `world_fold.rs::fold_air_wing_loadouts`) |
//! | `+40` | u32 | `reserved0` (must be zero, `world_fold.rs::fold_air_wing_loadouts`) |
//!
//! Wing row (`world_fold.rs::fold_air_wing_loadouts`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id` | raw | none (key; **not** zero-rejected) |
//! | `+4`  | u32 | `wing_id`         | raw | none (key; **not** zero-rejected) |
//! | `+8`  | u64 | `generation`      | raw | none (key) |
//! | `+16` | u32 | `flags`           | —   | only `1<<0` DELETED is legal (`world_fold.rs::fold_air_wing_loadouts`) |
//! | `+20` | u32 | `air_base_id`     | raw | **value test** `!= 0` (`world_fold.rs::fold_air_wing_loadouts`), not a flag |
//! | `+24` | u32 | `entry_first`     | —   | transport, not state |
//! | `+28` | u32 | `entry_count`     | —   | transport, not state |
//!
//! Entry row (`world_fold.rs::fold_air_wing_loadouts`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0` | u32 | `variant_id` | raw     | none (key) |
//! | `+4` | u32 | `reserved`   | —       | must be zero (`world_fold.rs::fold_air_wing_loadouts`) |
//! | `+8` | i64 | `amount_raw` | /100000 | none; **no range check in the fold** |
//!
//! # Two collections, because `(entry_first, entry_count)` is transport
//!
//! A wing row addresses its equipment as a half-open range into one shared entry
//! array (`world_fold.rs::fold_air_wing_loadouts`); the ranges are re-packed by the
//! collector from emission order on every frame
//! (`attach_log.rs:70258`-`:70274`) and carry no state a reader could ask about.
//! What is state is the wing's placement plus the set of
//! `(owner, wing, generation, variant_id) -> amount` facts. So the ranges are
//! flattened away: collection [`WINGS`] carries one row per wing loadout and
//! collection [`EQUIPMENT`] one row per equipment line, keyed by the wing key
//! plus the variant.
//!
//! **The wing's equipment list is replaced wholesale, never merged.** The fold's
//! accumulator value is one `AirWingLoadoutState { air_base_id, equipment }`
//! (`world_fold.rs:23762`-`:23765`) and an upsert re-inserts the whole struct
//! (`world_fold.rs::fold_air_wing_loadouts`), so a variant the new row omits is gone. [`State::fold_payload`]
//! therefore drops every [`EQUIPMENT`] key under a wing before inserting that
//! wing's new lines; merging them would resurrect equipment the recorder dropped.
//!
//! # `amount` is divided here
//!
//! `scale_raw` is asserted equal to `FIXED_SCALE` by the fold (`:14898`), so the
//! scale is a contract rather than a per-frame variable, and the consumer-facing
//! projection divides: the snapshot emits `entry.amount_raw as f64 / FIXED_SCALE`
//! in both the joined `equipment` array (`:8203`) and the `_r2_air_wing_loadout`
//! leaf, which carries `amount_raw` and `amount` side by side (`world_fold.rs::build_order_of_battle`).
//! The division belongs at this boundary; nothing else in the row is scaled.
//!
//! # Four states
//!
//! The schema declares **one** empty state on **one** field.
//!
//! ## `air_base_id` — `unknown`, and nothing else
//!
//! The gate is a value test, not a flag: `(row.air_base_id != 0).then_some(...)`
//! (`world_fold.rs::fold_air_wing_loadouts`). There is no flag to consult, so the meaning of the
//! zero has to come from the producer, per the recipe's step 2.
//!
//! The collector's live-row path never writes it. A wing row is built by
//! `sample_compact_air_wing_loadout_object_once`, whose `air_base_id` comes from
//! `read_air_wing_pool_air_base_id(pool)?` (`attach_log.rs:57183`) — and the `?`
//! **drops the entire wing row** when that read returns `None`.
//! `read_air_wing_pool_air_base_id` (`:57391`-`:57398`) returns `None` on four
//! conditions: the airbase-handle qword is unreadable, the handle's type word is
//! not `AIR_BASE_TYPE`, the id is `0`, or the id exceeds `MAX_AIR_BASE_ID`. Three
//! of the four are read/validation failures — §3's "applicable, not captured" —
//! and none of them survives into a serialized row at all.
//!
//! The **only** writer of `air_base_id == 0` is
//! `deleted_compact_air_wing_loadout_row` (`attach_log.rs:56034`-`:56045`), and
//! every row it builds carries `DELETED`; the fold removes the key and never
//! constructs a state from it (`world_fold.rs:14979`-`:14982`). So today's
//! collector cannot emit a live row carrying the zero.
//!
//! "This wing is at no air base" is carried by **key absence**, not by a zero:
//! the snapshot's own comment for a wing with no d32 row reads *"A wing without a
//! current D32 row has no authenticated air-base handle to join. Keep every
//! placement leaf null."* (`world_fold.rs::fold_air_wing_loadouts` region, `world_fold.rs::build_order_of_battle`).
//! Nothing in either authority assigns "the relation genuinely points at nothing"
//! to the zero word.
//!
//! That leaves `null` and `unknown` undecidable from the evidence, and the
//! recipe's tie-break is explicit: choose `unknown`, because `null` asserts
//! "definitively empty" about a value that may merely have been uncaptured and
//! that assertion is irreversible. `absent` is wrong outright — every air wing
//! has a placement, so the concept never fails to apply. Recorded as an open
//! question. Measured: across 1,571 live wing rows in 146 hours of
//! `runs/batch2/deepwar_t1`, `air_base_id` is **never** zero (max observed 408).
//!
//! The row is **not** rejected. The fold accepts it and stores a state, and the
//! recipe's precedence rule plus d22's mirroring precedent both say to represent
//! what the fold stores rather than refuse a frame the recorder folds.
//!
//! ## `amount` — no empty state, and zero is a real value
//!
//! The entry row has no gate. Unlike d26, the collector does **not** drop zero
//! lines and the fold does **not** reject `amount_raw <= 0`: a bank slot holding
//! nothing is a row carrying `0`. Measured: 3,331 of 5,565 entry rows carry
//! `amount_raw == 0`. §3 keeps zero out of the three empty states, so `amount`
//! declares no capability and its mapper can emit only a value.
//!
//! ## A deleted wing is a removal, not a row
//!
//! `DELETED` (`1<<0`, `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`) erases the
//! key (`:14979`-`:14982`). It maps to wire tombstones in both collections, so it
//! never reaches a record: [`record_from_wing_row`] refuses one rather than
//! inventing a placement.
//!
//! # The child-uniqueness check is this layer's, not the fold's
//!
//! `fold_air_wing_loadouts` parses entries into a `Vec` and slices it
//! (`world_fold.rs::fold_air_wing_loadouts`). It applies **no** uniqueness check, and
//! a repeated `variant_id` inside one wing's range is perfectly representable in
//! that `Vec` — both lines survive and both reach the snapshot array.
//!
//! d22's mirroring precedent does not reach this case, and the reason is worth
//! stating precisely. Mirroring is defensible there because the fold's permissive
//! `.entry().or_default()` collapse is *itself* last-writer-wins, so the wire's
//! keyed collapse reproduces the fold's stored state exactly — information is
//! preserved in both directions (`d22.rs:119`-`:135`). Here it is not: the fold
//! stores an ordered list in which two same-variant lines coexist, and
//! `(owner, wing, generation, variant_id)` cannot express that. Mirroring would
//! mean silently keeping one line and dropping the other, which is the data-loss
//! defect itself. `Shape::OrderedOccurrenceList` is now available, but adopting
//! it here would be a separate d32 schema migration requiring corpus evidence;
//! the other alternative is an ordinal key component re-packed from emission
//! order every frame, which would churn every row of every hour. This reference
//! port therefore still **rejects** a duplicated variant inside one wing's
//! range. That is a deliberate divergence from the fold, bounded by the
//! producer-side rejection below rather than by a missing codec primitive.
//!
//! It is not stricter than the **producer**. The collector's
//! `read_air_wing_equipment_bank_refs` rejects a duplicate variant outright —
//! `if seen.insert(variant_id, ()).is_some() { return None; }`
//! (`attach_log.rs:57448`-`:57450`) — and that `None` propagates through the `?`
//! at `:57184`, dropping the wing row. So no collector-produced frame can carry
//! one. Measured: 0 duplicates across 5,565 entry rows.
//!
//! **The wing key is mirrored, not tightened.** A wing key repeated inside one
//! frame is last-writer-wins in the fold (`self.air_wing_loadouts.insert`,
//! `:14986`) and last-writer-wins here, because a keyed set stores exactly what
//! that leaves behind. That contrast is the whole point: tightening is justified
//! only where mirroring would lose information.
//!
//! # Not modelled: the cross-domain d30↔d32 orphan prune
//!
//! The fold registers every key it inserts into
//! `air_wing_loadout_orphan_check_keys_this_hour` (`:14984`-`:14985`), which d30's
//! delete path also writes (`:14731`-`:14732`). At FRAME_END,
//! `prune_air_wing_loadout_orphans_for_current_hour` (`:5122`-`:5147`, called at
//! `:5040`) **removes** every registered key that d30's `air_wings` accumulator
//! does not hold. That is a second writer of this domain's accumulator, and it is
//! cross-domain: modelling it here would mean re-deriving d30's payload grammar,
//! which this crate does not own. `bench_real_d32.rs` instead reads only the d30
//! key set the check needs and **asserts the prune drops nothing**, so a capture
//! where it bites fails the harness loudly instead of yielding a wrong number.
//! Measured on `runs/batch2/deepwar_t1`: 0 drops in 146 hours, with d30 and d32
//! holding the identical 643 keys at the end.
//!
//! # No derived field (§6)
//!
//! Nothing here changes deterministically with the clock: an equipment amount
//! moves with attrition and production, and a placement moves on order. The
//! mappers still take `frame_tick` so every domain's boundary has one shape.

use std::collections::BTreeMap;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 32;
/// One row per air-wing loadout: the wing identity and its air-base placement.
pub const WINGS: u32 = 1;
/// One row per equipment line, keyed by its wing plus the variant.
pub const EQUIPMENT: u32 = 2;

/// Field ids are stable and append-only; §11 forbids reuse for a different
/// field. The three key components appear in both collections because they are
/// the same field in both — the wing's identity — which is what §11 forbids
/// reusing an id *against*.
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;
    pub const WING_ID: u32 = 2;
    pub const GENERATION: u32 = 3;
    pub const AIR_BASE_ID: u32 = 4;
    pub const VARIANT_ID: u32 = 5;
    pub const AMOUNT: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs:979`-`:981` and `:14856`-`:14968`. --

/// `AIR_WING_LOADOUT_HEADER_LEN`, `world_fold.rs::AIR_WING_LOADOUT_HEADER_LEN`.
pub const HEADER_LEN: usize = 44;
/// `AIR_WING_LOADOUT_ROW_LEN`, `world_fold.rs::AIR_WING_LOADOUT_ROW_LEN`.
pub const WING_ROW_LEN: usize = 32;
/// `AIR_WING_LOADOUT_ENTRY_ROW_LEN`, `world_fold.rs::AIR_WING_LOADOUT_ENTRY_ROW_LEN`.
pub const ENTRY_ROW_LEN: usize = 16;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`.
pub const WING_FLAG_DELETED: u32 = 1;
/// `world_fold.rs::fold_air_wing_loadouts`: the delete bit is the only legal flag, and any
/// other bit is a hard error — an unrecognised flag word means the row is not
/// this grammar, so nothing in it can be trusted.
pub const WING_KNOWN_FLAGS: u32 = WING_FLAG_DELETED;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`. `ensure_fixed_scale` (`:14898`) asserts the
/// header word equal to it, so this is the only scale d32 ever uses and a frame
/// declaring another is rejected rather than read at its own scale.
pub const FIXED_SCALE: f64 = 100_000.0;
/// The `scale_raw` word the collector packs
/// (`COMPACT_FIXED_SCALE_RAW`, `attach_log/offsets.rs:291`) and the fold
/// requires. Kept as an integer so the header check is exact.
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own
/// grid — see [`schema`] and `fixed_five_is_lossless_over_the_producers_range`.
pub const SCALE_DIGITS: u8 = 5;

/// `MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW`, `attach_log/offsets.rs:1067`. This is the
/// **producer's** bound, not the fold's: `fold_air_wing_loadouts` applies no
/// range check to `amount_raw` at all (`world_fold.rs::fold_air_wing_loadouts`), so the mapper does
/// not either. It is named here because it is what makes `Fixed(5)` lossless over
/// every value a capture can contain (`1e15 < 2^53`).
pub const PRODUCER_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;

/// `COMPACT_STATUS_DELTA_DATA`. Named here because d32 is not admitted to the
/// production-codec dependency surface; the measurement bin asserts the local
/// value and shipping contract agree. d24 declares it the same way.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: WINGS,
                // Wings persist across hours and the frame grammar is
                // clear-on-first-baseline / upsert-after / DELETED-removes
                // (`world_fold.rs::fold_air_wing_loadouts`), which is exactly a keyed
                // set's grammar — and only a keyed set has deltas (§7.3).
                shape: Shape::KeyedSet,
                fields: vec![
                    // The fold's key is `AirWingKey { owner_tag_db_id, wing_id,
                    // generation }` (`world_fold.rs::fold_air_wing_loadouts`), the same key
                    // type d30 air_wings uses (`world_fold.rs::fold_air_wings`). None of the
                    // three is zero-rejected by this fold, so all three are
                    // concrete in every row by construction rather than by check.
                    //
                    // `owner_tag_db_id` is a country tag db id — content-defined
                    // (§5): a manifest must resolve it for it to keep its meaning
                    // under another mod set. d63 keys its countries the same way,
                    // d26 keys its divisions the same way, d20 classes its
                    // `owner` the same way. This is a ContentId **key**, the
                    // heavier retrofit class.
                    Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
                    // A runtime instance identity created during play: air wing 5
                    // is a different wing next campaign and no manifest resolves
                    // it. `Unsigned`, exactly as d20 classes `div_id`.
                    Field::new(f::WING_ID, Kind::Unsigned).key(1),
                    // A collector-assigned re-use discriminator: the tracker
                    // stamps a fresh counter value whenever a wing body address
                    // it has not seen appears
                    // (`attach_log.rs:56028`-`:56032`, `:55993`). Purely runtime,
                    // no manifest, so `Unsigned`.
                    Field::new(f::GENERATION, Kind::Unsigned).key(2),
                    // The wing's air base. `Unsigned`, not `ContentId`: the id is
                    // the high half of a runtime engine handle whose type word is
                    // `AIR_BASE_TYPE` (`attach_log.rs:57391`-`:57397`), i.e. an
                    // instance created during play — no content manifest resolves
                    // it, which is §5's deciding question. d34 keys
                    // `air_base_status` by the same id and classes the
                    // content-defined leaves it joins (`state_id`,
                    // `province_id`) separately.
                    //
                    // `unknown_capable` and nothing else — see the module docs.
                    Field::new(f::AIR_BASE_ID, Kind::Unsigned).unknown_capable(),
                ],
            },
            Collection {
                collection_id: EQUIPMENT,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
                    Field::new(f::WING_ID, Kind::Unsigned).key(1),
                    Field::new(f::GENERATION, Kind::Unsigned).key(2),
                    // An equipment variant id, and literally the same id space
                    // d26's equipment uses: the collector validates it against
                    // `DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA` /
                    // `DIVISION_EQUIPMENT_VARIANT_TYPE` and bounds it by
                    // `MAX_DIVISION_EQUIPMENT_VARIANT_ID`
                    // (`attach_log.rs:57434`-`:57450`). `ContentId` for d26's
                    // reason: under another mod set the same number names other
                    // equipment, and the replay layer cannot say what a variant
                    // *is* without the content-side catalog. A ContentId **key**,
                    // so the heavier retrofit class again.
                    Field::new(f::VARIANT_ID, Kind::ContentId).key(3),
                    // `Continuous` because the fold's own projection is
                    // `amount_raw as f64 / FIXED_SCALE` (`world_fold.rs:8203`,
                    // `:8219`-`:8220`).
                    //
                    // `Quant::Fixed(5)` is not a precision *choice*: the source is
                    // an i64 over a denominator the fold asserts to be exactly
                    // `10^5` (`world_fold.rs::fold_air_wing_loadouts`), so `fixed(5)` reproduces the source
                    // integer bit for bit over every value a producer can emit
                    // (proved in the tests). §6's "exact unless justified" bar is
                    // met by a stronger fact than a precision budget: nothing is
                    // rounded at all. It also matters that the raw integer
                    // survives, because the snapshot publishes `amount_raw`
                    // beside `amount` (`world_fold.rs::build_order_of_battle`) — `fixed(5)` is precisely the
                    // quantization under which `round(value * 1e5)` returns that
                    // integer unchanged, and `exact` would spend 8 raw bytes per
                    // value to carry the same information.
                    Field::new(f::AMOUNT, Kind::Continuous).quant(Quant::Fixed(SCALE_DIGITS)),
                ],
            },
        ],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 44-byte payload header (`world_fold.rs::fold_air_wing_loadouts`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_air_wing_rows: u32,
    pub observed_equipment_rows: u32,
    pub equipment_count: u32,
    pub deleted_rows: u32,
    pub scale_raw: u32,
    pub reserved0: u32,
}

impl PayloadHeader {
    /// `status_flags & COMPACT_STATUS_DELTA_DATA` (`world_fold.rs:14892`,
    /// `:2948`). The one header invariant that is mode-dependent reads this, and
    /// so does the baseline-vs-sparse decision in [`State::fold_payload`].
    pub fn is_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// One parsed wing row. `entry_first`/`entry_count` are kept because the caller
/// needs them to find the row's entries, not because they are state: they index
/// the frame's shared entry array and are re-packed from emission order every
/// frame (`attach_log.rs:70258`-`:70274`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WingRow {
    pub owner_tag_db_id: u32,
    pub wing_id: u32,
    pub generation: u64,
    pub flags: u32,
    /// `None` when the word is zero (`world_fold.rs::fold_air_wing_loadouts`) — §3's `unknown`, see
    /// the module docs.
    pub air_base_id: Option<u32>,
    pub entry_first: usize,
    pub entry_count: usize,
    /// `DELETED`: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
}

/// One parsed equipment entry row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EquipmentRow {
    pub variant_id: u32,
    pub amount_raw: i64,
}

/// Read the 44-byte payload header and apply exactly the checks
/// `fold_air_wing_loadouts` applies **before** it consults `status_flags`
/// (`world_fold.rs::fold_air_wing_loadouts`).
///
/// The split is the fold's own call order and it is load-bearing for a harness:
/// the fold validates these header counts, then returns early on a failure
/// status or a delta-before-baseline (`world_fold.rs::fold_air_wing_loadouts`), and only then checks
/// the payload length and reads any row. A caller that needs the status word in
/// order to decide whether the record folds at all must therefore be able to
/// read the header without paying for the body — which is what this is for.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_wing_loadouts payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_air_wing_rows: u32_at(payload, 20),
        observed_equipment_rows: u32_at(payload, 24),
        equipment_count: u32_at(payload, 28),
        deleted_rows: u32_at(payload, 32),
        scale_raw: u32_at(payload, 36),
        reserved0: u32_at(payload, 40),
    };

    // `ensure_count_at_most`, `world_fold.rs:14870`-`:14883`.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "air_wing_loadouts changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "air_wing_loadouts deleted_rows {} > row_count {}",
            header.deleted_rows, header.row_count
        ));
    }
    // `ensure_row_count_with_deletes`, `world_fold.rs:14884`-`:14890` /
    // `:30228`-`:30243`: a delete row has no observed wing behind it, so the
    // roster count is allowed to fall short by exactly the deletions. Widened in
    // u64 because the fold's `saturating_add` is on `usize` and this crate must
    // reject rather than depend on a platform width.
    let allowed = header.observed_air_wing_rows as u64 + header.deleted_rows as u64;
    if header.row_count as u64 > allowed {
        return Err(format!(
            "air_wing_loadouts row_count {} > observed_air_wing_rows {} + deleted_rows {}",
            header.row_count, header.observed_air_wing_rows, header.deleted_rows
        ));
    }
    // `world_fold.rs::fold_air_wing_loadouts`: a full frame serializes every equipment row
    // it observed, a delta only the changed ones — so the bound holds only when
    // `DELTA_DATA` is clear.
    if header.equipment_count > header.observed_equipment_rows && !header.is_delta() {
        return Err(format!(
            "air_wing_loadouts emitted_equipment_rows {} > observed_equipment_rows {}",
            header.equipment_count, header.observed_equipment_rows
        ));
    }
    // `ensure_fixed_scale`, `world_fold.rs:14898`. The mapper's `/1e5` is only
    // correct while this holds; a frame declaring another scale is not this
    // grammar.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "air_wing_loadouts scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    // `world_fold.rs::fold_air_wing_loadouts`.
    if header.reserved0 != 0 {
        return Err("air_wing_loadouts header reserved field is nonzero".into());
    }
    Ok(header)
}

/// Split a d32 DATA payload into its header, its wing rows and its entry rows,
/// applying every structural check `fold_air_wing_loadouts` applies — plus the
/// one check this layer must carry itself.
///
/// The fold's own checks are [`peek_header`]'s, then the exact payload length
/// (`world_fold.rs::fold_air_wing_loadouts`), then per wing row the flag word and the
/// entry-range bound (`world_fold.rs::fold_air_wing_loadouts`), then per entry row the reserved word
/// (`world_fold.rs::fold_air_wing_loadouts`).
///
/// The extra check is **child uniqueness**: a `variant_id` repeated inside one
/// wing's range is rejected here although the fold accepts it. See the module
/// docs for why mirroring is not available for that one case and why the
/// collector makes the check non-restrictive in practice.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing. It is fail-closed
/// and total — no input of any length panics.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;

    // Checked arithmetic throughout: both counts are attacker-shaped words from a
    // file on disk, and either product overflows `usize` for a large enough one.
    // `world_fold.rs::fold_air_wing_loadouts` uses saturating arithmetic, which turns an
    // overflowing claim into a length that cannot match; rejecting reaches the
    // same outcome without relying on that.
    let wings_len = (header.row_count as usize)
        .checked_mul(WING_ROW_LEN)
        .ok_or_else(|| format!("air_wing_loadouts row_count {} overflows", header.row_count))?;
    let entries_len = (header.equipment_count as usize)
        .checked_mul(ENTRY_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "air_wing_loadouts equipment_count {} overflows",
                header.equipment_count
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(wings_len)
        .and_then(|len| len.checked_add(entries_len))
        .ok_or_else(|| "air_wing_loadouts payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!(
            "air_wing_loadouts len {} != {expected}",
            payload.len()
        ));
    }

    let wings = &payload[HEADER_LEN..HEADER_LEN + wings_len];
    let entries = &payload[HEADER_LEN + wings_len..];

    let equipment_count = header.equipment_count as usize;

    // Pass 1 — the FOLD's per-row checks, on every row: the flag word and the
    // entry-range bound (`world_fold.rs::fold_air_wing_loadouts`). The fold applies both
    // before it ever looks at the delete bit, so delete rows are checked too.
    let mut parsed_rows: Vec<WingRow> = Vec::with_capacity(wings.len() / WING_ROW_LEN);
    let mut last_row_of_key: BTreeMap<Key, usize> = BTreeMap::new();
    for (index, row) in wings.chunks_exact(WING_ROW_LEN).enumerate() {
        let parsed =
            parse_wing_row(row).map_err(|e| format!("air_wing_loadouts row {index}: {e}"))?;
        entry_index_range(&parsed, equipment_count)
            .map_err(|e| format!("air_wing_loadouts row {index}: {e}"))?;
        last_row_of_key.insert(
            wing_key(parsed.owner_tag_db_id, parsed.wing_id, parsed.generation),
            index,
        );
        parsed_rows.push(parsed);
    }

    // Every entry row, referenced or not: the fold parses the whole array and
    // rejects a non-zero reserved word wherever it is
    // (`world_fold.rs::fold_air_wing_loadouts`). Ranges may legally overlap or leave gaps,
    // so a per-range loop alone would miss an unreferenced row.
    for (index, row) in entries.chunks_exact(ENTRY_ROW_LEN).enumerate() {
        parse_equipment_row(row).map_err(|e| format!("air_wing_loadouts entry {index}: {e}"))?;
    }

    // Pass 2 — THIS layer's own check, and only where it is load-bearing.
    //
    // The property that has to hold is that the state the frame LEAVES BEHIND is
    // representable in the wire key, not that every byte of the frame is. A row
    // whose equipment the fold never stores cannot cause a collision:
    //
    //   * a DELETE row's range is never read — the fold `continue`s at `:14980`,
    //     before the slice at `:14990`;
    //   * a row for a key that a LATER row in the same frame replaces is
    //     discarded whole, because the fold's `insert` (`:14986`) is
    //     last-writer-wins and the accumulator value carries the whole equipment
    //     list. That later row may itself be a delete, in which case nothing for
    //     the key survives at all.
    //
    // So the check runs on exactly the surviving rows: the last row for each wing
    // key, when that row is live. Checking more would refuse frames whose folded
    // state this schema represents perfectly well.
    for (index, parsed) in parsed_rows.iter().enumerate() {
        if parsed.deleted {
            continue;
        }
        let key = wing_key(parsed.owner_tag_db_id, parsed.wing_id, parsed.generation);
        if last_row_of_key.get(&key) != Some(&index) {
            continue;
        }
        let range = entry_index_range(parsed, equipment_count)
            .map_err(|e| format!("air_wing_loadouts row {index}: {e}"))?;
        let mut seen: Vec<u32> = Vec::with_capacity(range.len());
        for entry_index in range {
            let start = entry_index * ENTRY_ROW_LEN;
            let entry = parse_equipment_row(&entries[start..start + ENTRY_ROW_LEN])
                .map_err(|e| format!("air_wing_loadouts row {index} entry {entry_index}: {e}"))?;
            if seen.contains(&entry.variant_id) {
                return Err(format!(
                    "air_wing_loadouts row {index} repeats variant_id {} inside one wing's \
                     equipment range; the wire key (owner, wing, generation, variant) cannot \
                     carry both lines",
                    entry.variant_id
                ));
            }
            seen.push(entry.variant_id);
        }
    }

    Ok((header, wings, entries))
}

/// Parse one 32-byte wing row, applying every per-row check the fold applies
/// (`world_fold.rs::fold_air_wing_loadouts`) except the entry-range bound, which needs the
/// frame's `equipment_count` and lives in [`entry_index_range`].
///
/// Mirrored deliberately: the fold does **not** reject a zero `owner_tag_db_id`
/// or a zero `wing_id` here (contrast d26's roots, `world_fold.rs::fold_division_equipment`), and
/// neither does this. Tightening would refuse a frame the recorder folds.
pub fn parse_wing_row(row: &[u8]) -> core::result::Result<WingRow, String> {
    if row.len() != WING_ROW_LEN {
        return Err(format!(
            "air_wing_loadouts wing row length {} != {WING_ROW_LEN}",
            row.len()
        ));
    }
    let owner_tag_db_id = u32_at(row, 0);
    let wing_id = u32_at(row, 4);
    let generation = u64_at(row, 8);
    let flags = u32_at(row, 16);
    let air_base_word = u32_at(row, 20);
    let entry_first = u32_at(row, 24) as usize;
    let entry_count = u32_at(row, 28) as usize;

    // `world_fold.rs::fold_air_wing_loadouts`.
    if flags & !WING_KNOWN_FLAGS != 0 {
        return Err(format!("air_wing_loadouts row has unknown flags {flags:#x}"));
    }
    let deleted = flags & WING_FLAG_DELETED != 0;

    Ok(WingRow {
        owner_tag_db_id,
        wing_id,
        generation,
        flags,
        // `world_fold.rs::fold_air_wing_loadouts`: a value test, not a flag. The fold does NOT
        // require a delete row to zero the rest of its payload (contrast d26's
        // `world_fold.rs::fold_division_equipment`), so a delete row's `air_base_id` word is read the same way
        // and simply never reaches a record.
        air_base_id: (air_base_word != 0).then_some(air_base_word),
        entry_first,
        entry_count,
        deleted,
    })
}

/// Parse one 16-byte entry row, applying the fold's per-row checks
/// (`world_fold.rs::fold_air_wing_loadouts`).
///
/// The fold checks the reserved word and nothing else: `variant_id` has no bound
/// (contrast d26's `world_fold.rs::fold_division_equipment`) and `amount_raw` has no range check at all (contrast
/// d26's `world_fold.rs::fold_division_equipment`). Both are mirrored — see [`PRODUCER_MAX_AMOUNT_RAW`] for the
/// producer-side bound that the fold declines to restate.
pub fn parse_equipment_row(row: &[u8]) -> core::result::Result<EquipmentRow, String> {
    if row.len() != ENTRY_ROW_LEN {
        return Err(format!(
            "air_wing_loadouts entry row length {} != {ENTRY_ROW_LEN}",
            row.len()
        ));
    }
    let variant_id = u32_at(row, 0);
    let reserved = u32_at(row, 4);
    let amount_raw = i64_at(row, 8);
    if reserved != 0 {
        return Err("air_wing_loadouts equipment reserved field is nonzero".into());
    }
    Ok(EquipmentRow {
        variant_id,
        amount_raw,
    })
}

/// The half-open index range of one wing's entry rows within the frame's shared
/// entry array, checked exactly as the fold checks it
/// (`world_fold.rs::fold_air_wing_loadouts`).
///
/// [`split_payload`] runs this over every row, and any caller that wants a wing's
/// entries runs it again to find them: one derivation of the range rule, so the
/// check and the use cannot disagree. Ranges may legally overlap and may leave
/// entry rows unreferenced — the fold constrains only their bounds.
///
/// A delete row is not special-cased: the fold checks its range with the same
/// arithmetic as any other row's, before it ever looks at the delete bit.
pub fn entry_index_range(
    row: &WingRow,
    equipment_count: usize,
) -> core::result::Result<core::ops::Range<usize>, String> {
    let end = row
        .entry_first
        .checked_add(row.entry_count)
        .ok_or_else(|| "air_wing_loadouts equipment range overflow".to_string())?;
    if end > equipment_count {
        return Err(format!(
            "air_wing_loadouts equipment range {}+{} exceeds {equipment_count}",
            row.entry_first, row.entry_count
        ));
    }
    Ok(row.entry_first..end)
}

/// The [`WINGS`] key of one wing: `(owner_tag_db_id, wing_id, generation)`
/// (`world_fold.rs::fold_air_wing_loadouts`).
pub fn wing_key(owner_tag_db_id: u32, wing_id: u32, generation: u64) -> Key {
    key(&[owner_tag_db_id as u64, wing_id as u64, generation])
}

/// The [`EQUIPMENT`] key of one entry: its wing's key plus the variant.
pub fn equipment_key(
    owner_tag_db_id: u32,
    wing_id: u32,
    generation: u64,
    variant_id: u32,
) -> Key {
    key(&[
        owner_tag_db_id as u64,
        wing_id as u64,
        generation,
        variant_id as u64,
    ])
}

/// The [`WINGS`] key of one raw wing row.
pub fn key_from_wing_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_wing_row(row)?;
    Ok(wing_key(
        parsed.owner_tag_db_id,
        parsed.wing_id,
        parsed.generation,
    ))
}

/// The [`EQUIPMENT`] key of one raw entry row under one raw wing row.
pub fn key_from_equipment_row(
    wing_row: &[u8],
    entry_row: &[u8],
) -> core::result::Result<Key, String> {
    let wing = parse_wing_row(wing_row)?;
    let entry = parse_equipment_row(entry_row)?;
    Ok(equipment_key(
        wing.owner_tag_db_id,
        wing.wing_id,
        wing.generation,
        entry.variant_id,
    ))
}

/// One compact row of the `wings` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 32-byte wing row slice (the caller has
/// already consumed the payload header via [`split_payload`]); d32 has exactly
/// one wing row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d32 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_air_wing_loadouts`: a row the fold
/// would reject is an `Err` here, never a coerced value. A **deleted** row is an
/// `Err` too — it is a removal (`world_fold.rs::fold_air_wing_loadouts`) and has no
/// record; a caller must map it to wire tombstones in both collections, and
/// quietly handing one back a placement-less row would turn a delete into an
/// upsert. Nothing in this function can panic on any input.
pub fn record_from_wing_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d32 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_wing_row(row)?;
    if parsed.deleted {
        return Err(format!(
            "air_wing_loadouts wing owner={} wing_id={} generation={} is a delete row, not a record",
            parsed.owner_tag_db_id, parsed.wing_id, parsed.generation
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // A zero word means the collector could not authenticate an air-base
            // handle, not that the wing is at no base — §3's `unknown`. See the
            // module docs; no live row a collector produces can reach it.
            field::AIR_BASE_ID => match parsed.air_base_id {
                Some(id) => Value::Unsigned(id as u64),
                None => Value::Unknown,
            },
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "air_wing_loadouts wings has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact entry row of the `equipment` table onto its stored Record. `row`
/// is the raw 16-byte entry row slice; see [`record_from_wing_row`] for the
/// `frame_tick` contract.
///
/// `amount` is `amount_raw / FIXED_SCALE` — the fold's own projection
/// (`world_fold.rs::build_order_of_battle`). `amount_raw == 0` is a real measured
/// value (a bank slot holding nothing) and never an empty state.
pub fn record_from_equipment_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let parsed = parse_equipment_row(row)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::AMOUNT => Value::Continuous(parsed.amount_raw as f64 / FIXED_SCALE),
            other => {
                return Err(format!(
                    "air_wing_loadouts equipment has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// What [`State::fold_payload`] did with a payload, mirroring the three paths
/// `fold_air_wing_loadouts` can take once its status checks have passed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The first non-delta frame: `replace_or_sparse_after_first` returned
    /// `false`, so the accumulator was cleared and refilled
    /// (`world_fold.rs:14969`-`:14972`, `:2947`-`:2960`). The domain now has a
    /// baseline.
    Baseline,
    /// Every frame after that, delta or not: rows are upserted, delete rows
    /// remove, and nothing else is touched. `replace_or_sparse_after_first`
    /// returns `true` for **any** frame once the domain has a baseline, which is
    /// why a later `status_flags == 0` frame is still sparse rather than a second
    /// full replace.
    Sparse,
    /// A frame with `DELTA_DATA` set before any baseline. `reject_delta_before_baseline`
    /// (`world_fold.rs:14911`-`:14915`, `:2895`-`:2917`) marks the domain-hour
    /// unknown, folds **nothing**, and retains the accumulator. The hour carries
    /// no d32 data and belongs in no measurement.
    DeltaBeforeBaseline,
}

/// The two per-hour keyed states d32 folds into, plus the one bit of fold state
/// the payload grammar depends on.
///
/// One record is not one state for this domain: `replace_or_sparse_after_first`
/// clears only on the FIRST non-delta frame and every frame after that is a
/// sparse upsert into the retained accumulator, so an hour's state is the whole
/// history of the stream up to that hour, not that hour's rows. This is the
/// recipe's fourth public boundary, and it exists so the measurement bin, the
/// tests and any future caller share ONE replay of the shipping layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Collection [`WINGS`], keyed by [`wing_key`].
    pub wings: KeyedState,
    /// Collection [`EQUIPMENT`], keyed by [`equipment_key`].
    pub equipment: KeyedState,
    /// `delta_baselines_seen.contains(32)` (`world_fold.rs::replace_or_sparse_after_first`).
    baseline_seen: bool,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a baseline has landed. Public because the two exclusion paths a
    /// harness must implement — failure status and delta-before-baseline —
    /// are decided partly from it.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d32 DATA payload exactly as `fold_air_wing_loadouts` folds it
    /// (`world_fold.rs::fold_air_wing_loadouts`).
    ///
    /// The caller is responsible for the **failure-status** exclusion, because
    /// the predicate is `compact_contract`'s
    /// (`compact_status_failure_flags_after_expected_partial_tolerance`) and this
    /// crate's library half must not depend on that crate nor copy its mask. The
    /// fold consults it at `world_fold.rs::fold_air_wing_loadouts`, i.e. after [`peek_header`]'s checks and
    /// before the payload length check — so a caller that gets the order right
    /// calls [`peek_header`], excludes on status, and only then calls this.
    ///
    /// The **delta-before-baseline** exclusion is handled here, because it needs
    /// no vocabulary this crate lacks: it is `DELTA_DATA` against
    /// [`State::baseline_seen`], and the fold makes the same decision before
    /// touching anything (`world_fold.rs::fold_air_wing_loadouts`).
    ///
    /// A structural error leaves the state exactly as the fold would have left
    /// it: every row is parsed before the first mutation, mirroring the fold,
    /// whose own errors all precede its `clear`/`insert` loop.
    pub fn fold_payload(
        &mut self,
        wings_collection: &Collection,
        equipment_collection: &Collection,
        payload: &[u8],
    ) -> core::result::Result<FoldOutcome, String> {
        // Read the header first so the delta decision is made before any
        // structural work, exactly as the fold orders it.
        let header = peek_header(payload)?;
        let delta = header.is_delta();
        if delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        let (_, wing_rows, entry_rows) = split_payload(payload)?;
        let equipment_count = header.equipment_count as usize;

        // Build everything before mutating anything.
        struct Pending {
            key: Key,
            owner_tag_db_id: u32,
            wing_id: u32,
            generation: u64,
            deleted: bool,
            record: Option<Record>,
            entries: Vec<(Key, Record)>,
        }
        let mut pending: Vec<Pending> = Vec::with_capacity(wing_rows.len() / WING_ROW_LEN);
        for row in wing_rows.chunks_exact(WING_ROW_LEN) {
            let parsed = parse_wing_row(row)?;
            let key = wing_key(parsed.owner_tag_db_id, parsed.wing_id, parsed.generation);
            if parsed.deleted {
                pending.push(Pending {
                    key,
                    owner_tag_db_id: parsed.owner_tag_db_id,
                    wing_id: parsed.wing_id,
                    generation: parsed.generation,
                    deleted: true,
                    record: None,
                    entries: Vec::new(),
                });
                continue;
            }
            let record = record_from_wing_row(wings_collection, row, 0)?;
            let range = entry_index_range(&parsed, equipment_count)?;
            let mut entries = Vec::with_capacity(range.len());
            for entry_index in range {
                let start = entry_index * ENTRY_ROW_LEN;
                let raw = &entry_rows[start..start + ENTRY_ROW_LEN];
                let entry = parse_equipment_row(raw)?;
                entries.push((
                    equipment_key(
                        parsed.owner_tag_db_id,
                        parsed.wing_id,
                        parsed.generation,
                        entry.variant_id,
                    ),
                    record_from_equipment_row(equipment_collection, raw, 0)?,
                ));
            }
            pending.push(Pending {
                key,
                owner_tag_db_id: parsed.owner_tag_db_id,
                wing_id: parsed.wing_id,
                generation: parsed.generation,
                deleted: false,
                record: Some(record),
                entries,
            });
        }

        // `replace_or_sparse_after_first` (`world_fold.rs:14969`, `:2947`): only
        // the FIRST non-delta frame clears. Everything after is sparse, delta bit
        // or not.
        let sparse = delta || self.baseline_seen;
        if !sparse {
            self.wings.clear();
            self.equipment.clear();
            self.baseline_seen = true;
        }

        for row in pending {
            // The fold's accumulator value carries the wing's WHOLE equipment
            // list (`world_fold.rs:23762`-`:23765`), so both a delete and an
            // upsert replace it entirely. Merging would resurrect a variant the
            // new row dropped.
            self.drop_equipment_of(row.owner_tag_db_id, row.wing_id, row.generation);
            if row.deleted {
                // `world_fold.rs::fold_air_wing_loadouts`.
                self.wings.remove(&row.key);
                continue;
            }
            // `world_fold.rs::fold_air_wing_loadouts`: a plain insert, so a key repeated inside one
            // frame is last-writer-wins rather than a rejected frame.
            self.wings
                .insert(row.key, row.record.expect("a live row has a record"));
            for (key, record) in row.entries {
                self.equipment.insert(key, record);
            }
        }

        if sparse {
            Ok(FoldOutcome::Sparse)
        } else {
            Ok(FoldOutcome::Baseline)
        }
    }

    /// Remove every [`EQUIPMENT`] key under one wing key. The equipment keys are
    /// the wing key with the variant appended, and `Key` orders lexicographically
    /// with a prefix sorting before any extension of itself, so the wing's lines
    /// are exactly the half-open key range `[o, w, g] .. [o, w, g + 1]`.
    fn drop_equipment_of(&mut self, owner_tag_db_id: u32, wing_id: u32, generation: u64) {
        let lower = Key(vec![
            owner_tag_db_id as i128,
            wing_id as i128,
            generation as i128,
        ]);
        let upper = Key(vec![
            owner_tag_db_id as i128,
            wing_id as i128,
            generation as i128 + 1,
        ]);
        let doomed: Vec<Key> = self
            .equipment
            .range(lower..upper)
            .map(|(k, _)| k.clone())
            .collect();
        for k in doomed {
            self.equipment.remove(&k);
        }
    }
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{escapes, quantize};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn wings() -> Collection {
        schema().collections[0].clone()
    }

    fn equipment() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 32-byte wing row, built the way the collector packs one
    /// (`attach_log.rs:70264`-`:70271`).
    fn wing_row(
        owner_tag_db_id: u32,
        wing_id: u32,
        generation: u64,
        flags: u32,
        air_base_id: u32,
        entry_first: u32,
        entry_count: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(WING_ROW_LEN);
        row.extend_from_slice(&owner_tag_db_id.to_le_bytes());
        row.extend_from_slice(&wing_id.to_le_bytes());
        row.extend_from_slice(&generation.to_le_bytes());
        for word in [flags, air_base_id, entry_first, entry_count] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), WING_ROW_LEN);
        row
    }

    /// A raw 16-byte entry row (`attach_log.rs:70277`-`:70280`).
    fn entry_row(variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(ENTRY_ROW_LEN);
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        row.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(row.len(), ENTRY_ROW_LEN);
        row
    }

    /// A whole payload: header, then wing rows, then entry rows. `deleted_rows`
    /// and the observed counters are filled consistently with the rows given, so
    /// a test that wants an invalid header patches it through `overrides`.
    fn payload(status_flags: u32, rows: &[Vec<u8>], entries: &[Vec<u8>]) -> Vec<u8> {
        payload_with(status_flags, rows, entries, &[])
    }

    fn payload_with(
        status_flags: u32,
        rows: &[Vec<u8>],
        entries: &[Vec<u8>],
        overrides: &[(usize, u32)],
    ) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let entry_count = entries.len() as u32;
        let deleted = rows
            .iter()
            .filter(|r| u32::from_le_bytes(r[16..20].try_into().unwrap()) & WING_FLAG_DELETED != 0)
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(row_count - deleted).to_le_bytes()); // observed_air_wing_rows
        p.extend_from_slice(&entry_count.to_le_bytes()); // observed_equipment_rows
        p.extend_from_slice(&entry_count.to_le_bytes()); // equipment_count
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        assert_eq!(p.len(), HEADER_LEN);
        for (offset, word) in overrides {
            p[*offset..*offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        for row in rows {
            p.extend_from_slice(row);
        }
        for row in entries {
            p.extend_from_slice(row);
        }
        p
    }

    fn full_wing_row() -> Vec<u8> {
        wing_row(7, 11, 3, 0, 408, 0, 2)
    }

    // --- Step 8.1: the schema validates -------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d32 schema is well-formed");
    }

    // --- Step 8.5: shape and key --------------------------------------------

    #[test]
    fn both_collections_are_keyed_sets_keyed_by_the_air_wing() {
        let w = wings();
        assert_eq!(w.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = w.key_fields();
        assert_eq!(keys.len(), 3, "AirWingKey is (owner, wing_id, generation)");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert_eq!(keys[1].field_id, field::WING_ID);
        assert_eq!(keys[1].kind, Kind::Unsigned, "a runtime instance identity");
        assert_eq!(keys[2].field_id, field::GENERATION);
        assert_eq!(
            keys[2].kind,
            Kind::Unsigned,
            "a collector-assigned re-use counter, not content"
        );
        for k in &keys {
            assert!(k.kind.legal_as_key());
            assert!(
                !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
                "key field {} must be concrete",
                k.field_id
            );
        }
        assert_eq!(w.fields.len(), 4, "3 key components + air_base_id");

        let e = equipment();
        assert_eq!(e.shape, Shape::KeyedSet);
        let ekeys = e.key_fields();
        assert_eq!(ekeys.len(), 4, "the wing key plus the variant");
        assert_eq!(ekeys[3].field_id, field::VARIANT_ID);
        assert_eq!(
            ekeys[3].kind,
            Kind::ContentId,
            "an equipment variant is content-defined (§5), as in d26"
        );
        for k in &ekeys {
            assert!(k.kind.legal_as_key());
            assert!(!k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none());
        }
        assert_eq!(e.fields.len(), 5, "4 key components + amount");
    }

    #[test]
    fn the_key_extractors_agree_with_the_key_builders() {
        let row = wing_row(7, 11, 0xFFFF_FFFF_0000_0001, 0, 408, 0, 1);
        assert_eq!(
            key_from_wing_row(&row).expect("keys"),
            wing_key(7, 11, 0xFFFF_FFFF_0000_0001),
            "generation is a full u64 and must not be truncated"
        );
        let entry = entry_row(900, 100_000);
        assert_eq!(
            key_from_equipment_row(&row, &entry).expect("keys"),
            equipment_key(7, 11, 0xFFFF_FFFF_0000_0001, 900)
        );
        assert!(key_from_wing_row(&[]).is_err());
        assert!(key_from_equipment_row(&row, &[]).is_err());
    }

    // --- Step 8.2: byte-identical round trips --------------------------------

    #[test]
    fn a_full_wing_row_roundtrips_byte_for_byte() {
        let c = wings();
        let record = record_from_wing_row(&c, &full_wing_row(), 0).expect("maps");
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
    fn an_unknown_air_base_roundtrips_byte_for_byte() {
        let c = wings();
        let record = record_from_wing_row(&c, &wing_row(7, 11, 3, 0, 0, 0, 0), 0).expect("maps");
        assert_eq!(
            record.values[index_of(&c, field::AIR_BASE_ID)],
            Value::Unknown
        );
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, record);
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    #[test]
    fn an_equipment_row_roundtrips_byte_for_byte_including_a_real_zero() {
        let c = equipment();
        for amount_raw in [0i64, 1, 100_000, 10_000_000, PRODUCER_MAX_AMOUNT_RAW] {
            let record =
                record_from_equipment_row(&c, &entry_row(900, amount_raw), 0).expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len(), "amount_raw {amount_raw}");
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "amount_raw {amount_raw}");
        }
    }

    // --- Step 8.3: states through the mapper, from raw bytes -----------------

    #[test]
    fn the_schema_declares_exactly_one_empty_state_on_exactly_one_field() {
        // The mapper may emit exactly what the schema declares and nothing more.
        // `air_base_id` is unknown-capable; nothing else in either collection can
        // reach an empty state at all. Absent would claim the concept fails to
        // apply to an air wing; Null would claim we know the wing is at no base,
        // which is carried by key absence instead. See the module docs.
        for c in [wings(), equipment()] {
            assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
            assert!(
                c.value_fields().all(|f| !f.nullable),
                "no field is a relation that can point at nothing"
            );
        }
        let unknown: Vec<u32> = wings()
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(unknown, vec![field::AIR_BASE_ID]);
        assert!(
            equipment().value_fields().all(|f| !f.unknown_capable),
            "amount has no gate: the entry row carries a value or the row does not exist"
        );
    }

    #[test]
    fn a_real_zero_air_base_word_maps_to_unknown_and_a_real_id_to_a_value() {
        // The whole two-state space of the one capability the schema declares,
        // built from raw bytes through the public mapper.
        let c = wings();
        let unknown = record_from_wing_row(&c, &wing_row(7, 11, 3, 0, 0, 0, 0), 0).expect("maps");
        assert_eq!(
            unknown.values[index_of(&c, field::AIR_BASE_ID)],
            Value::Unknown,
            "a zero word is `unknown`, not a zero air base and not null"
        );
        for id in [1u32, 408, u32::MAX] {
            let r = record_from_wing_row(&c, &wing_row(7, 11, 3, 0, id, 0, 0), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::AIR_BASE_ID)],
                Value::Unsigned(id as u64),
                "air_base_id {id} is a value"
            );
        }
    }

    #[test]
    fn a_zero_amount_is_a_value_not_an_empty_state() {
        // 3,331 of 5,565 entry rows in `runs/batch2/deepwar_t1` carry a zero
        // amount. §3 keeps zero out of the three empty states, and unlike d26 no
        // producer drops these lines.
        let c = equipment();
        let r = record_from_equipment_row(&c, &entry_row(900, 0), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::AMOUNT)],
            Value::Continuous(0.0)
        );
    }

    #[test]
    fn a_delete_row_is_refused_rather_than_mapped() {
        // A delete is a removal, not a record. Handing back a placement-less row
        // would turn it into an upsert.
        let c = wings();
        let row = wing_row(7, 11, 3, WING_FLAG_DELETED, 0, 0, 0);
        assert!(record_from_wing_row(&c, &row, 0).is_err());
        // And the delete row is still parseable and keyable, because the fold
        // reads its key before it looks at the bit.
        let parsed = parse_wing_row(&row).expect("parses");
        assert!(parsed.deleted);
        assert_eq!(key_from_wing_row(&row).expect("keys"), wing_key(7, 11, 3));
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d32 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        for c in [wings(), equipment()] {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
        // And the mappers ignore it: the same bytes at two ticks map alike.
        let w = wings();
        assert_eq!(
            record_from_wing_row(&w, &full_wing_row(), 0).expect("maps"),
            record_from_wing_row(&w, &full_wing_row(), 987_654).expect("maps")
        );
        let e = equipment();
        assert_eq!(
            record_from_equipment_row(&e, &entry_row(900, 5), 0).expect("maps"),
            record_from_equipment_row(&e, &entry_row(900, 5), 987_654).expect("maps")
        );
    }

    // --- Step 8.6: every field's offset, with scale as the special case ------

    #[test]
    fn every_wing_field_reads_from_its_own_offset() {
        // Distinct values in every word, so a transposition fails rather than
        // passing by coincidence — the d62 failure class.
        let c = wings();
        let row = wing_row(0x1111_1111, 0x2222_2222, 0x3333_3333_4444_4444, 0, 0x5555_5555, 7, 9);
        let parsed = parse_wing_row(&row).expect("parses");
        assert_eq!(parsed.owner_tag_db_id, 0x1111_1111, "owner at +0");
        assert_eq!(parsed.wing_id, 0x2222_2222, "wing_id at +4");
        assert_eq!(parsed.generation, 0x3333_3333_4444_4444, "generation at +8");
        assert_eq!(parsed.flags, 0, "flags at +16");
        assert_eq!(parsed.air_base_id, Some(0x5555_5555), "air_base_id at +20");
        assert_eq!(parsed.entry_first, 7, "entry_first at +24");
        assert_eq!(parsed.entry_count, 9, "entry_count at +28");
        let r = record_from_wing_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::AIR_BASE_ID)],
            Value::Unsigned(0x5555_5555),
            "air_base_id is emitted raw by the fold, never divided"
        );
        // The key components come from their own offsets too.
        assert_eq!(
            key_from_wing_row(&row).expect("keys"),
            wing_key(0x1111_1111, 0x2222_2222, 0x3333_3333_4444_4444)
        );
    }

    #[test]
    fn every_entry_field_reads_from_its_own_offset_at_the_fixed_scale() {
        // The test that catches a wrong divisor: known fixed-point words against
        // their exact f64 readings, each at its own offset.
        let c = equipment();
        let cases: [(i64, f64); 8] = [
            (0, 0.0),
            (1, 0.000_01),
            (-1, -0.000_01),
            (100_000, 1.0),
            (150_000, 1.5),
            (-100_000, -1.0),
            (10_000_000, 100.0),
            (PRODUCER_MAX_AMOUNT_RAW, 10_000_000_000.0),
        ];
        for (raw, expected) in cases {
            let row = entry_row(0x6666_6666, raw);
            let parsed = parse_equipment_row(&row).expect("parses");
            assert_eq!(parsed.variant_id, 0x6666_6666, "variant_id at +0");
            assert_eq!(parsed.amount_raw, raw, "amount_raw at +8");
            let r = record_from_equipment_row(&c, &row, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::AMOUNT)],
                Value::Continuous(expected),
                "raw {raw} must read {expected} at scale {FIXED_SCALE}"
            );
        }
        // A 32768-scaled reading of the same word would be ~3x off; pin the
        // difference explicitly so the constant cannot drift silently.
        assert_eq!(FIXED_SCALE, 100_000.0);
        assert_eq!(FIXED_SCALE_RAW, 100_000);
    }

    // --- Step 8.4: quantization actually saves bytes -------------------------

    #[test]
    fn fixed_five_is_lossless_over_the_producers_range() {
        // `Quant::Fixed(5)` is chosen because it is the source's own grid, so the
        // claim to prove is `quantize(raw / 1e5) == raw` with no escape, over
        // every value a producer can emit — not "the error is small". The
        // producer's bound is `MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW`
        // (`attach_log/offsets.rs:1067`), which is `1e15 < 2^53`.
        let q = Quant::Fixed(SCALE_DIGITS);
        let check = |raw: i64| {
            let v = raw as f64 / FIXED_SCALE;
            assert!(!escapes(v, q), "raw {raw} escaped fixed(5)");
            assert_eq!(quantize(v, q), Some(raw), "raw {raw} did not survive");
        };
        for raw in [
            0,
            1,
            -1,
            100_000,
            -100_000,
            10_000_000,
            PRODUCER_MAX_AMOUNT_RAW,
            -PRODUCER_MAX_AMOUNT_RAW,
            // The exact-integer boundary of binary64, where the property ends.
            1i64 << 53,
            -(1i64 << 53),
        ] {
            check(raw);
        }
        // Exhaustive across the unit interval, where rounding ties live.
        for raw in -100_000..=100_000 {
            check(raw);
        }
        // A stride over the whole producer range, coprime with 10^5 so the
        // samples do not all land on the same residue.
        let mut raw: i64 = 0;
        while raw <= PRODUCER_MAX_AMOUNT_RAW {
            check(raw);
            raw += 1_000_000_007;
        }
    }

    #[test]
    fn beyond_the_binary64_integer_range_the_escape_bit_carries_the_value() {
        // The fold applies NO bound to `amount_raw` (`world_fold.rs::fold_air_wing_loadouts`), so
        // the mapper accepts words no producer can emit. Past `2^53` the source
        // integer is no longer exactly representable, `fixed(5)` stops being
        // stable, and the codec's escape path stores the raw f64 instead. That is
        // the codec behaving correctly on a value that lost precision at the
        // `as f64` conversion before the codec ever saw it — pinned here so the
        // limit is a recorded property rather than a surprise.
        let c = equipment();
        let raw = i64::MAX;
        let record = record_from_equipment_row(&c, &entry_row(900, raw), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, record, "the escape path is still a faithful round trip");
        assert!(escapes(raw as f64 / FIXED_SCALE, Quant::Fixed(SCALE_DIGITS)));
    }

    #[test]
    fn quantization_saves_bytes_against_an_all_exact_variant() {
        let quantized = equipment();
        let mut exact = quantized.clone();
        for f in &mut exact.fields {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let mut quantized_len = 0usize;
        let mut exact_len = 0usize;
        for i in 0..64u32 {
            let row = entry_row(900 + i, 100_000 * i as i64 + 37);
            for (c, total) in [(&quantized, &mut quantized_len), (&exact, &mut exact_len)] {
                let mut bytes = Vec::new();
                record_from_equipment_row(c, &row, 0)
                    .expect("maps")
                    .encode(c, &mut bytes)
                    .expect("encodes");
                *total += bytes.len();
            }
        }
        assert!(
            quantized_len < exact_len,
            "fixed(5) {quantized_len} must beat exact {exact_len}"
        );
        // The wings collection has no continuous field at all, so step 4's test
        // does not apply there and there is nothing to quantize.
        assert_eq!(
            wings()
                .value_fields()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
    }

    // --- Rejection: every row and header the fold rejects --------------------

    #[test]
    fn the_mappers_reject_every_row_the_fold_rejects() {
        let w = wings();
        let e = equipment();
        for len in [0usize, 1, WING_ROW_LEN - 1, WING_ROW_LEN + 1] {
            assert!(record_from_wing_row(&w, &vec![0u8; len], 0).is_err(), "wing len {len}");
        }
        for len in [0usize, 1, ENTRY_ROW_LEN - 1, ENTRY_ROW_LEN + 1] {
            assert!(
                record_from_equipment_row(&e, &vec![0u8; len], 0).is_err(),
                "entry len {len}"
            );
        }
        // Every unknown flag bit, not one sample (`world_fold.rs::fold_air_wing_loadouts`).
        for bit in 1..32u32 {
            let row = wing_row(7, 11, 3, 1 << bit, 408, 0, 0);
            assert!(
                record_from_wing_row(&w, &row, 0).is_err(),
                "flag bit {bit} is not in WING_KNOWN_FLAGS and must be rejected"
            );
            assert!(parse_wing_row(&row).is_err(), "flag bit {bit}");
        }
        // A non-zero entry reserved word (`:14959`), every bit of it.
        for bit in 0..32u32 {
            let mut row = entry_row(900, 5);
            row[4..8].copy_from_slice(&(1u32 << bit).to_le_bytes());
            assert!(
                record_from_equipment_row(&e, &row, 0).is_err(),
                "reserved bit {bit}"
            );
        }
    }

    /// The FOLD's tolerance and the live PRODUCER's rules are different sets,
    /// and the offline consumer holds the producer's.
    ///
    /// This module documents what the fold accepts, which is deliberately
    /// looser than what `read_air_wing_equipment_bank_refs` in the collector
    /// emits -- see the test directly below, which pins `entry_row(0, 0)` and
    /// `entry_row(u32::MAX, i64::MIN)` as folded. Reading that looseness as the
    /// contract is what would let a consumer sum a bank no recording can
    /// contain, so `portable_admission` carries the producer's tighter bounds
    /// and `replay_core::leaf_exact_air_wing_loadout` refuses against those.
    /// This asserts the one bound both layers name, so the two cannot drift.
    ///
    /// kills: changing `PRODUCER_MAX_AMOUNT_RAW` here or
    /// `AIR_WING_BANK_MAX_AMOUNT_RAW` in the shared crate without the other.
    #[test]
    fn the_producer_amount_bound_matches_the_shared_refusal_rule() {
        assert_eq!(
            PRODUCER_MAX_AMOUNT_RAW,
            portable_admission::AIR_WING_BANK_MAX_AMOUNT_RAW
        );
        // And the looseness this module documents is real, not a mistake: the
        // shared rule refuses exactly what the fold tolerates here.
        assert!(portable_admission::air_wing_bank_rejection(vec![(0u32, 0i64)]).is_some());
        assert!(portable_admission::air_wing_bank_rejection(vec![(1u32, 0i64)]).is_none());
    }

    #[test]
    fn the_mappers_mirror_what_the_fold_does_not_reject() {
        // The fold applies no bound to `owner_tag_db_id`, `wing_id`,
        // `variant_id` or `amount_raw` for this domain. Tightening would refuse a
        // frame the recorder folds, so each is pinned as accepted.
        let w = wings();
        let e = equipment();
        assert!(record_from_wing_row(&w, &wing_row(0, 0, 0, 0, 408, 0, 0), 0).is_ok());
        assert!(record_from_equipment_row(&e, &entry_row(0, 0), 0).is_ok());
        assert!(record_from_equipment_row(&e, &entry_row(u32::MAX, i64::MIN), 0).is_ok());
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut w = wings();
        w.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_wing_row(&w, &full_wing_row(), 0).is_err());
        let mut e = equipment();
        e.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_equipment_row(&e, &entry_row(900, 5), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![full_wing_row()];
        let entries = vec![entry_row(900, 100_000), entry_row(901, 0)];
        let p = payload(0, &rows, &entries);
        let (header, wing_bytes, entry_bytes) = split_payload(&p).expect("well-formed");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.equipment_count, 2);
        assert_eq!(wing_bytes, &rows[0][..]);
        assert_eq!(entry_bytes.len(), 2 * ENTRY_ROW_LEN);

        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(peek_header(&p[..len]).is_err(), "len {len}");
        }
        // A length that does not match the declared counts, both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());

        // Each header invariant, one at a time, at its own offset.
        // changed_rows > row_count (`:14870`).
        assert!(split_payload(&payload_with(0, &rows, &entries, &[(12, 2)])).is_err());
        // deleted_rows > row_count (`:14877`).
        assert!(split_payload(&payload_with(0, &rows, &entries, &[(32, 2)])).is_err());
        // row_count > observed_air_wing_rows + deleted_rows (`:14884`).
        assert!(split_payload(&payload_with(0, &rows, &entries, &[(20, 0)])).is_err());
        // equipment_count > observed_equipment_rows on a NON-delta frame (`:14891`).
        assert!(split_payload(&payload_with(0, &rows, &entries, &[(24, 1)])).is_err());
        // ...and the same header IS accepted with DELTA_DATA set, because the
        // bound is mode-dependent. A delta serializes only changed entry rows.
        assert!(
            split_payload(&payload_with(STATUS_DELTA_DATA, &rows, &entries, &[(24, 1)])).is_ok(),
            "the equipment bound must not be applied to a delta frame"
        );
        // scale_raw != 100000 (`:14898`) — including the 32768 confusion.
        for wrong in [0u32, 1, 32_768, 100_001] {
            assert!(
                split_payload(&payload_with(0, &rows, &entries, &[(36, wrong)])).is_err(),
                "scale_raw {wrong}"
            );
        }
        // reserved0 != 0 (`:14899`), every bit.
        for bit in 0..32u32 {
            assert!(
                split_payload(&payload_with(0, &rows, &entries, &[(40, 1 << bit)])).is_err(),
                "reserved0 bit {bit}"
            );
        }
        // An entry range past the frame's entries (`:14940`).
        let over = vec![wing_row(7, 11, 3, 0, 408, 1, 2)];
        assert!(split_payload(&payload(0, &over, &entries)).is_err());
        // An entry range whose arithmetic overflows (`:14942`).
        let overflow = vec![wing_row(7, 11, 3, 0, 408, u32::MAX, u32::MAX)];
        assert!(split_payload(&payload(0, &overflow, &entries)).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        assert!(split_payload(&payload_with(0, &rows, &entries, &[
            (8, u32::MAX),
            (12, u32::MAX),
            (20, u32::MAX),
        ]))
        .is_err());
        // An unreferenced entry row is still parsed, so its reserved word still
        // has to be zero (`:14956`-`:14968`).
        let none = vec![wing_row(7, 11, 3, 0, 408, 0, 0)];
        let mut bad_entry = entry_row(900, 5);
        bad_entry[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&payload(0, &none, &[bad_entry])).is_err());
    }

    #[test]
    fn split_payload_rejects_a_variant_repeated_inside_one_wings_range() {
        // The one check that is this layer's rather than the fold's. The fold
        // stores an ordered Vec in which both lines survive; the wire key cannot
        // carry both, so keeping one silently would be the data loss itself. The
        // collector already refuses such a bank (`attach_log.rs:57448`).
        let rows = vec![wing_row(7, 11, 3, 0, 408, 0, 2)];
        let dup = vec![entry_row(900, 100_000), entry_row(900, 200_000)];
        assert!(split_payload(&payload(0, &rows, &dup)).is_err());

        // The same variant under DIFFERENT wings is fine — the key differs.
        let two = vec![
            wing_row(7, 11, 3, 0, 408, 0, 1),
            wing_row(7, 12, 4, 0, 408, 1, 1),
        ];
        assert!(split_payload(&payload(0, &two, &dup)).is_ok());

        // And the same variant twice in the frame's array, referenced by two
        // ranges that each see it once, is also fine.
        let overlapping = vec![
            wing_row(7, 11, 3, 0, 408, 0, 1),
            wing_row(7, 12, 4, 0, 408, 0, 1),
        ];
        assert!(split_payload(&payload(0, &overlapping, &[entry_row(900, 1)])).is_ok());
    }

    #[test]
    fn the_uniqueness_check_spares_every_row_whose_equipment_the_fold_discards() {
        // Round-1 review finding, generalized. The check must constrain the state
        // the frame LEAVES BEHIND, not every byte in it: a range the fold never
        // reads cannot collide with anything. Three ways a range is discarded,
        // each with a duplicated variant that must therefore be accepted.
        let dup = [entry_row(900, 100_000), entry_row(900, 200_000)];

        // 1. A DELETE row's range is never read (`world_fold.rs::fold_air_wing_loadouts` returns
        //    before the slice at `world_fold.rs::fold_air_wing_loadouts`). The reviewer's exact input.
        let deleted = vec![wing_row(7, 11, 3, WING_FLAG_DELETED, 0, 0, 2)];
        assert!(
            split_payload(&payload(0, &deleted, &dup)).is_ok(),
            "a delete row's equipment range is never stored"
        );

        // 2. A live row later replaced by another live row for the same key: the
        //    fold's `insert` is last-writer-wins over the whole state value.
        let overwritten = vec![
            wing_row(7, 11, 3, 0, 408, 0, 2),
            wing_row(7, 11, 3, 0, 409, 0, 1),
        ];
        assert!(split_payload(&payload(0, &overwritten, &dup)).is_ok());

        // 3. A live row later deleted in the same frame.
        let then_deleted = vec![
            wing_row(7, 11, 3, 0, 408, 0, 2),
            wing_row(7, 11, 3, WING_FLAG_DELETED, 0, 0, 0),
        ];
        assert!(split_payload(&payload(0, &then_deleted, &dup)).is_ok());

        // The check still bites where it must: the SURVIVING row.
        let survives = vec![
            wing_row(7, 11, 3, 0, 408, 0, 1),
            wing_row(7, 11, 3, 0, 409, 0, 2),
        ];
        assert!(
            split_payload(&payload(0, &survives, &dup)).is_err(),
            "the last live row for a key is the one that is stored"
        );
        // ...and a surviving row is still checked when an earlier discarded row
        // for a DIFFERENT key was clean.
        let mixed = vec![
            wing_row(7, 10, 1, WING_FLAG_DELETED, 0, 0, 0),
            wing_row(7, 11, 3, 0, 408, 0, 2),
        ];
        assert!(split_payload(&payload(0, &mixed, &dup)).is_err());

        // And in every accepted case the folded state really is collision free —
        // the property the check exists to protect, asserted directly rather than
        // inferred from the check having passed.
        for (rows, expected_wings, expected_lines) in [
            (&deleted, 0usize, 0usize),
            (&overwritten, 1, 1),
            (&then_deleted, 0, 0),
        ] {
            let mut s = State::new();
            s.fold_payload(&wings(), &equipment(), &payload(0, rows, &dup))
                .expect("folds");
            assert_eq!(s.wings.len(), expected_wings);
            assert_eq!(
                s.equipment.len(),
                expected_lines,
                "the surviving state is representable in the wire key"
            );
        }
    }

    #[test]
    fn split_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: these are public entry points reached from a
        // file on disk and must return a diagnosis, never unwind.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..8_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let _ = split_payload(&bytes);
            let _ = peek_header(&bytes);
            let _ = parse_wing_row(&bytes);
            let _ = parse_equipment_row(&bytes);
            let _ = record_from_wing_row(&wings(), &bytes, state);
            let _ = record_from_equipment_row(&equipment(), &bytes, state);
            let mut s = State::new();
            let _ = s.fold_payload(&wings(), &equipment(), &bytes);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = payload(0, &[full_wing_row()], &[entry_row(900, 1), entry_row(901, 2)]);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let _ = split_payload(&p);
            let mut s = State::new();
            let _ = s.fold_payload(&wings(), &equipment(), &p);
        }
    }

    // --- The frame-level fold ------------------------------------------------

    fn fold(state: &mut State, p: &[u8]) -> FoldOutcome {
        state
            .fold_payload(&wings(), &equipment(), p)
            .expect("folds")
    }

    #[test]
    fn the_first_non_delta_frame_is_a_baseline_and_every_later_frame_is_sparse() {
        // `replace_or_sparse_after_first` (`world_fold.rs::replace_or_sparse_after_first`) clears
        // only ONCE. A later `status_flags == 0` frame is still sparse, which is
        // the trap: treating it as a second full replace would delete every wing
        // the frame did not mention.
        let mut s = State::new();
        assert_eq!(
            fold(&mut s, &payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 1)], &[entry_row(900, 1)])),
            FoldOutcome::Baseline
        );
        assert_eq!(s.wings.len(), 1);
        assert_eq!(s.equipment.len(), 1);
        assert!(s.baseline_seen());

        // A second frame with the SAME status word mentioning a different wing.
        assert_eq!(
            fold(&mut s, &payload(0, &[wing_row(7, 12, 4, 0, 409, 0, 1)], &[entry_row(901, 2)])),
            FoldOutcome::Sparse
        );
        assert_eq!(s.wings.len(), 2, "the first wing survives a later frame");
        assert_eq!(s.equipment.len(), 2);

        // A frame carrying no rows at all changes nothing.
        assert_eq!(fold(&mut s, &payload(0, &[], &[])), FoldOutcome::Sparse);
        assert_eq!(s.wings.len(), 2);
        assert_eq!(s.equipment.len(), 2);
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing_and_retains_the_state() {
        // `reject_delta_before_baseline` (`world_fold.rs:14911`, `:2895`) returns
        // before anything is cleared or upserted.
        let mut s = State::new();
        assert_eq!(
            fold(
                &mut s,
                &payload(
                    STATUS_DELTA_DATA,
                    &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                    &[entry_row(900, 1)]
                )
            ),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(s.wings.is_empty());
        assert!(s.equipment.is_empty());
        assert!(!s.baseline_seen());

        // After a baseline the same delta applies.
        fold(&mut s, &payload(0, &[], &[]));
        assert_eq!(
            fold(
                &mut s,
                &payload(
                    STATUS_DELTA_DATA,
                    &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                    &[entry_row(900, 1)]
                )
            ),
            FoldOutcome::Sparse
        );
        assert_eq!(s.wings.len(), 1);
    }

    #[test]
    fn an_upsert_replaces_the_wings_whole_equipment_list() {
        // The fold's accumulator value carries the whole list
        // (`world_fold.rs:23762`), so a variant the new row omits is gone.
        // Merging instead would resurrect equipment the recorder dropped — the
        // defect this test exists to catch.
        let e = equipment();
        let mut s = State::new();
        fold(
            &mut s,
            &payload(
                0,
                &[wing_row(7, 11, 3, 0, 408, 0, 2)],
                &[entry_row(900, 100_000), entry_row(901, 200_000)],
            ),
        );
        assert_eq!(s.equipment.len(), 2);
        fold(
            &mut s,
            &payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 1)], &[entry_row(901, 300_000)]),
        );
        assert_eq!(s.equipment.len(), 1, "variant 900 is gone, not merged");
        let record = &s.equipment[&equipment_key(7, 11, 3, 901)];
        assert_eq!(
            record.values[index_of(&e, field::AMOUNT)],
            Value::Continuous(3.0)
        );
        // A wing with an empty bank keeps its WINGS row and loses every line.
        fold(&mut s, &payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]));
        assert_eq!(s.wings.len(), 1);
        assert!(s.equipment.is_empty());
    }

    #[test]
    fn a_delete_row_removes_the_wing_and_all_of_its_equipment() {
        let mut s = State::new();
        fold(
            &mut s,
            &payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 2),
                    wing_row(7, 12, 4, 0, 409, 2, 1),
                ],
                &[
                    entry_row(900, 100_000),
                    entry_row(901, 200_000),
                    entry_row(902, 300_000),
                ],
            ),
        );
        assert_eq!(s.wings.len(), 2);
        assert_eq!(s.equipment.len(), 3);
        fold(
            &mut s,
            &payload(0, &[wing_row(7, 11, 3, WING_FLAG_DELETED, 0, 0, 0)], &[]),
        );
        assert_eq!(s.wings.len(), 1, "the deleted wing is gone");
        assert_eq!(
            s.equipment.len(),
            1,
            "and so are both of its equipment lines, but not the other wing's"
        );
        assert!(s.equipment.contains_key(&equipment_key(7, 12, 4, 902)));
    }

    #[test]
    fn only_the_named_generation_is_touched() {
        // The equipment sweep is a key-range operation, so a neighbouring
        // generation of the SAME wing must survive it. The off-by-one that would
        // wipe `(7, 11, 4)` when `(7, 11, 3)` is upserted is exactly what a
        // prefix range gets wrong.
        let mut s = State::new();
        fold(
            &mut s,
            &payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 1),
                    wing_row(7, 11, 4, 0, 409, 1, 1),
                    wing_row(7, 12, 3, 0, 410, 2, 1),
                ],
                &[entry_row(900, 1), entry_row(901, 2), entry_row(902, 3)],
            ),
        );
        assert_eq!(s.equipment.len(), 3);
        fold(&mut s, &payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]));
        assert_eq!(s.equipment.len(), 2, "only generation 3's line went");
        assert!(s.equipment.contains_key(&equipment_key(7, 11, 4, 901)));
        assert!(s.equipment.contains_key(&equipment_key(7, 12, 3, 902)));
        // The same at the u64 ceiling, where `generation + 1` would overflow a
        // narrower type.
        let mut s = State::new();
        fold(
            &mut s,
            &payload(0, &[wing_row(7, 11, u64::MAX, 0, 408, 0, 1)], &[entry_row(900, 1)]),
        );
        assert_eq!(s.equipment.len(), 1);
        fold(
            &mut s,
            &payload(0, &[wing_row(7, 11, u64::MAX, 0, 408, 0, 0)], &[]),
        );
        assert!(s.equipment.is_empty());
    }

    #[test]
    fn a_wing_key_repeated_in_one_frame_is_last_writer_wins() {
        // `self.air_wing_loadouts.insert` (`world_fold.rs::fold_air_wing_loadouts`) makes a repeat
        // a whole-row overwrite, not a rejected frame — and the equipment list
        // that survives is the last row's, because each upsert clears first.
        // Mirrored, unlike the child-uniqueness case: here the fold's own result
        // and the keyed set's coincide exactly.
        let w = wings();
        let mut s = State::new();
        fold(
            &mut s,
            &payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 1),
                    wing_row(7, 11, 3, 0, 409, 1, 1),
                ],
                &[entry_row(900, 1), entry_row(901, 2)],
            ),
        );
        assert_eq!(s.wings.len(), 1);
        assert_eq!(
            s.wings[&wing_key(7, 11, 3)].values[index_of(&w, field::AIR_BASE_ID)],
            Value::Unsigned(409),
            "the last row for a key wins"
        );
        assert_eq!(s.equipment.len(), 1);
        assert!(s.equipment.contains_key(&equipment_key(7, 11, 3, 901)));
    }

    #[test]
    fn a_structural_error_leaves_the_state_untouched() {
        let mut s = State::new();
        fold(&mut s, &payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 1)], &[entry_row(900, 1)]));
        let before = s.clone();
        // A second row with an unknown flag bit: the first row must not land.
        let broken = payload(
            0,
            &[
                wing_row(7, 12, 4, 0, 409, 0, 1),
                wing_row(7, 13, 5, 1 << 5, 0, 0, 0),
            ],
            &[entry_row(901, 2)],
        );
        assert!(s
            .fold_payload(&wings(), &equipment(), &broken)
            .is_err());
        assert_eq!(s, before, "a refused frame changes nothing");
    }

    #[test]
    fn the_folded_state_encodes_and_replays_through_the_reader() {
        // The whole pipeline in miniature: fold two hours, encode a baseline and
        // a stored-minimal delta over BOTH collections, and require the reader's
        // reconstruction to be stored-equal to the fold's.
        use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
        let schema = schema();
        let w = wings();
        let e = equipment();
        let mut s = State::new();
        fold(
            &mut s,
            &payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 2),
                    wing_row(7, 12, 4, 0, 0, 2, 1),
                ],
                &[entry_row(900, 100_000), entry_row(901, 0), entry_row(902, 5)],
            ),
        );
        let hour0 = (s.wings.clone(), s.equipment.clone());
        let mut bytes = Vec::new();
        encode_baseline(&schema, &[hour0.0.clone(), hour0.1.clone()], &mut bytes).expect("encodes");
        let (states, consumed) = decode_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(states[0], hour0.0);
        assert_eq!(states[1], hour0.1);

        // Hour 1: one amount changes, one wing is deleted.
        fold(
            &mut s,
            &payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 2),
                    wing_row(7, 12, 4, WING_FLAG_DELETED, 0, 0, 0),
                ],
                &[entry_row(900, 250_000), entry_row(901, 0)],
            ),
        );
        let hour1 = (s.wings.clone(), s.equipment.clone());
        let dw = diff(&w, &hour0.0, &hour1.0);
        let de = diff(&e, &hour0.1, &hour1.1);
        assert_eq!(dw.tombstones.len(), 1, "the deleted wing");
        assert_eq!(de.tombstones.len(), 1, "and its equipment line");
        assert_eq!(de.changed.len(), 1, "the changed amount");
        let mut dbytes = Vec::new();
        encode_delta(
            &schema,
            &[hour0.0.clone(), hour0.1.clone()],
            &[Some(dw), Some(de)],
            &mut dbytes,
        )
        .expect("encodes");
        let (deltas, consumed) =
            decode_delta(&schema, &[hour0.0.clone(), hour0.1.clone()], &dbytes, 0).expect("decodes");
        assert_eq!(consumed, dbytes.len());
        let mut replayed_w = hour0.0;
        let mut replayed_e = hour0.1;
        apply(&mut replayed_w, deltas[0].as_ref().unwrap());
        apply(&mut replayed_e, deltas[1].as_ref().unwrap());
        assert!(diff(&w, &replayed_w, &hour1.0).is_empty());
        assert!(diff(&e, &replayed_e, &hour1.1).is_empty());
    }
}
