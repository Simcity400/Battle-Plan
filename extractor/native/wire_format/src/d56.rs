//! d56 `naval_missions_status_live` — per-task-force naval mission status, plus
//! the per-strategic-region naval-supremacy sidecar.
//!
//! Derived from `world_fold.rs:22511`-`:22618`
//! (`fold_naval_missions_status`, dispatched at `:4723`), the row grammar in
// @world-fold-authority domain=56 functions=fold_naval_missions_status lines=22511-22618 sha256=c2b2f8413318f52c2efc47516abb2f44d6b52c3ca40d77574d631eafab5dcd58
//! `extractor/native/lanes/naval_missions_status_live/src/domain56_compact.rs`
//! (authority #2 for offsets, bounds and scales), and the collector read that
//! produces the presence flags,
//! `extractor/native/lanes/naval_missions_status_live/src/lib.rs:493`-`:605`
//! (`read_task_force_row`). There is no `documents/domains/d56_*.md` dossier.
//!
//! **The fold decodes the WHOLE payload before it looks at status**
//! (`world_fold.rs:22120`-`:22122`: `naval56::decode_status_payload(...)?`), so a
//! malformed payload is a refused stream whatever its status word — the d83 shape,
//! not the d63 one. Production's second authority
//! (`recorder_host/src/domain_data_authority.rs:8628`-`:8653`) runs the same lane
//! decode on every DOMAIN_DATA record whatever its status, so every check below is
//! reached on every record.
//!
//! **Payload grammar** (`domain56_compact.rs:182`-`:292`): a 44-byte header —
//! `elapsed_wall_ms` u64 `+0`, `row_count` u32 `+8`, `changed_rows` u32 `+12`,
//! `status_flags` u32 `+16`, `observed_task_forces` u32 `+20`,
//! `validated_location_rows` u32 `+24`, `derived_region_rows` u32 `+28`,
//! `fixture_required_rows` u32 `+32`, `convoy_sunk_count_rows` u32 `+36`,
//! `schema_version` u32 `+40` — then `row_count` fixed 64-byte rows, then an
//! optional `NSUP` trailer (12-byte header, 32-byte rows). There is no legacy
//! short row.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `task_force_id`            | raw | none (key; `1..=MAX_OOB_ID`) |
//! | `+4`  | u32 | `fleet_id`                 | raw | none (`1..=MAX_OOB_ID`) |
//! | `+8`  | u32 | `owner_tag_db_id`          | raw | none (`<= 2048`) |
//! | `+12` | u32 | `flags`                    | —   | — |
//! | `+16` | u32 | `mission_enum`             | raw | `1<<0` |
//! | `+20` | u32 | `mission_state`            | raw | `1<<1` |
//! | `+24` | u32 | `location_province_id`     | raw | `1<<2` |
//! | `+28` | u32 | `strategic_region_id`      | raw | `1<<3` |
//! | `+32` | u32 | `home_base_province_id`    | raw | `1<<4` |
//! | `+36` | u32 | `repair_base_province_id`  | raw | `1<<5` |
//! | `+40` | u32 | `repair_progress_milli`    | —   | retired: must be 0 |
//! | `+44` | u32 | `retreat_state`            | —   | retired: must be 0 |
//! | `+48` | u32 | `sunk_convoys`             | raw | `1<<9` |
//! | `+52` | u32 | `convoy_route_context_id`  | —   | retired: must be 0 |
//! | `+56` | u32 | `source_status`            | —   | bits 8..=10 only |
//! | `+60` | u32 | `reserved0`                | —   | must be 0 |
//!
//! **Three row words are not fields.** `repair_progress_milli`, `retreat_state`
//! and `convoy_route_context_id` are gated by bits in `RETIRED_ROW_FLAGS`
//! (`domain56_compact.rs:94`-`:99`), and the decoder rejects any row that sets one
//! of those bits *and* any row whose word is non-zero without it
//! (`:975`-`:990`, `:1048`-`:1052`). Every row the fold accepts therefore carries
//! zero in all three, so they are constants of the accepted grammar rather than
//! data — the d62/d24 rule. The invariant is enforced in the mapper and pinned by
//! a test instead of being re-encoded. `reserved0` is the same case (`:991`).
//! The batch-3 survey's "`repair_progress_milli` is a x1000 field" hazard is
//! therefore moot on this grammar: no accepted payload can carry a non-zero one,
//! and no scale is applied anywhere. Reported as a survey correction.
//!
//! **`FLAG_FIELD_REJECTED` (bit 14) is retired too** (`:99`), so it cannot appear
//! on any payload the fold accepts: a row setting it fails `validate_row`
//! (`:922`-`:927`), which makes `decode_status_payload` return `Err`, which makes
//! the fold return `Err` — a refused stream, not an Unknown discriminator. The
//! survey's "clear gate + `FIELD_REJECTED` = not captured, without = not
//! applicable" reading describes a schema-v1 position that production forbids.
//! Reported as a survey correction; nothing in this module can read that bit.
//!
//! **Four-state semantics come from the collector read**, `read_task_force_row`
//! (`lib.rs:493`-`:605`), because the fold answers only null-vs-value (it emits
//! JSON `Null` for every clear gate, `world_fold.rs:22159`-`:22193`). Every gated
//! field here is **unknown** on a clear gate, and one field — `sunk_convoys` — is
//! a measured **zero**:
//!
//! * `location_province_id` (`lib.rs:515`-`:524`): the word is read, then admitted
//!   only if it is in `1..=MAX_PROVINCE_ID`. A clear gate means the read returned
//!   0 or an out-of-range word; "no location" and "a garbage read" are
//!   indistinguishable, so it is the d32 `air_base_id` case — undecidable between
//!   Null and Unknown, resolved to **Unknown** by the recipe's tie-break, recorded.
//! * `strategic_region_id` (`:525`-`:533`): derived from the location province
//!   through `province_to_region`. The gate is clear when the location is absent,
//!   when the map lookup returns `None`, or when the region is out of range. A task
//!   force always *has* a region, so none of the three makes the concept
//!   inapplicable: all three are "applicable, not captured" — **Unknown**. (The
//!   d28-style split — Absent when the location gate is clear because the
//!   derivation was never attempted — was considered and rejected: the concept
//!   applies to every row, and Absent would assert it does not.)
//! * `mission_enum` (`:534`-`:547`) and `mission_state` (`:548`-`:560`): the word
//!   is read, then admitted only if it is a proven enum member. A clear gate means
//!   the engine word was unreadable-as-a-member — the value exists and was not
//!   captured. **Unknown**.
//! * `home_base_province_id` (`:561`-`:571`) and `repair_base_province_id`
//!   (`:572`-`:582`): both come from `read_cprovince_id` (`:753`-`:785`), which
//!   returns `None` on a torn pointer read, a null pointer, a non-heap pointer, a
//!   vtable mismatch, a torn id read, or an out-of-range id. Only the second of
//!   those six is §3's null; the other five are capture failures, and `None`
//!   collapses them. Undecidable → **Unknown** (tie-break), recorded.
//! * `sunk_convoys` (`:583`-`:589`): the count is always computed, and the flag is
//!   set exactly when the computed total is non-zero. The gate is therefore
//!   value-derived and a clear gate is a measured **Zero**, not an empty state —
//!   a fidelity gain over the fold, which folds `Null` there
//!   (`world_fold.rs:22189`-`:22193`). It is lossless only because the decoder
//!   requires a zero word under a clear gate (`domain56_compact.rs:1048`-`:1052`),
//!   which this mapper enforces rather than assumes. The collector-side ambiguity
//!   that remains — `aggregate_sunk_convoys` (`lib.rs:787`-`:823`) folds an
//!   unreadable ship vector to `0` rather than failing — is recorded upstream, the
//!   d16 counter ruling: 83% of rows carry a clear gate on the measured capture, so
//!   mapping `0` to Unknown would assert "not captured" over the great majority of
//!   real data.
//!
//! **The five witness bits are carried, not dropped.** `FLAG_VALIDATED_DIRECT`,
//! `FLAG_DERIVED_REGION` and the three live `source_status` bits are discarded by
//! the fold, but they are not derived from any carried field and the decoder
//! accepts every combination of them, so dropping them would lose data on input the
//! fold accepts. They are Booleans here. (Only a value the grammar *hard-enforces*
//! as a function of carried state is not a field — that is the retired trio above.)
//!
//! **The naval-supremacy sidecar is an ordered occurrence list.** `WIRE_FORMAT.md`
//! §7.3 classifies it by name — "ordered occurrence list — meaningful physical
//! order, duplicates legitimate, no unique key. **d56 naval supremacy is this**" —
//! and §13 records its measured port. The fold agrees: it calls the array an
//! "ORDER-PRESERVING occurrence list, not a keyed map", full-replaces it on every
//! successful frame regardless of the delta flag, and pushes every decoded row in
//! emit order into a `Vec` (`world_fold.rs:22589`-`:22609`), and the decoder states
//! that `(region, controller)` is explicitly NOT a uniqueness key, with live RE
//! evidence of repeats (`domain56_compact.rs:438`-`:449`).
//!
//! `Shape::OrderedOccurrenceList` now carries that state directly: all eight
//! fields are values, every occurrence survives, source order is canonical, and
//! a changed list is a full replacement. Collection id 2 is the sidecar.
//!
//! An ordinal-keyed set was measured as an alternative and **rejected**: the
//! recipe's step-5 test is about "every payload the FOLD accepts", not the payloads
//! one capture contains, and the fold accepts both duplicates and any order. That
//! this capture happens to be sorted and duplicate-free is a property of the stream
//! (`runs/batch3/d56_naval_missions_status/measurement.md` records the numbers),
//! and keying by position would publish false continuity across an insertion —
//! the shift is real in this capture, once. That counter-evidence remains part of
//! the implemented reading rather than disappearing once the shape exists.
//!
//! The trailer is still fully **validated** here, because the fold decodes it before
//! it folds anything (`world_fold.rs:22511`-`:22513`): a malformed trailer is a
//! refused stream, and a harness that skipped the walk would measure payloads
//! production rejects. [`split_payload`] does the walk and [`check_supremacy_row`]
//! the per-row bounds; nothing maps them into records.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock; the mappers take `frame_tick` and ignore it so every domain's boundary
//! has one shape.

use crate::body::{key, CollectionState, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 56;
pub const TASK_FORCES: u32 = 1;
pub const NAVAL_SUPREMACY: u32 = 2;

/// Task-force field ids. Stable and append-only; §11 forbids reuse.
pub mod field {
    pub const TASK_FORCE_ID: u32 = 1;
    pub const FLEET_ID: u32 = 2;
    pub const OWNER_TAG_DB_ID: u32 = 3;
    pub const MISSION_ENUM: u32 = 4;
    pub const MISSION_STATE: u32 = 5;
    pub const LOCATION_PROVINCE_ID: u32 = 6;
    pub const STRATEGIC_REGION_ID: u32 = 7;
    pub const HOME_BASE_PROVINCE_ID: u32 = 8;
    pub const REPAIR_BASE_PROVINCE_ID: u32 = 9;
    pub const SUNK_CONVOYS: u32 = 10;
    pub const VALIDATED_DIRECT: u32 = 11;
    pub const DERIVED_REGION: u32 = 12;
    pub const SOURCE_UNAVAILABLE: u32 = 13;
    pub const SOURCE_PARTIAL_DATA: u32 = 14;
    pub const SOURCE_DELTA_DATA: u32 = 15;

    // `naval_supremacy`. All are values because an occurrence list has no key
    // (§3); `(region, controller)` is explicitly non-unique in the game array.
    pub const SUPREMACY_REGION_ID: u32 = 20;
    pub const SUPREMACY_CONTROLLER_TAG_DB_ID: u32 = 21;
    pub const SUPREMACY_CURRENT: u32 = 22;
    pub const SUPREMACY_TARGET: u32 = 23;
    pub const SUPREMACY_BASE_TARGET: u32 = 24;
    pub const SUPREMACY_PREVIOUS: u32 = 25;
    pub const SUPREMACY_DECLINE_FROM: u32 = 26;
    pub const SUPREMACY_INDIVIDUAL_RATIO: u32 = 27;
}

/// Byte offsets of the supremacy row's eight words
/// (`domain56_compact.rs:453`-`:462`), kept beside the validator and mapper.
pub mod supremacy_offset {
    pub const REGION_ID: usize = 0;
    pub const CONTROLLER_TAG_DB_ID: usize = 4;
    pub const CURRENT: usize = 8;
    pub const TARGET: usize = 12;
    pub const BASE_TARGET: usize = 16;
    pub const PREVIOUS: usize = 20;
    pub const DECLINE_FROM: usize = 24;
    pub const INDIVIDUAL_RATIO: usize = 28;
}

// --- Payload grammar, from `domain56_compact.rs`. ----------------------------

/// `HEADER_LEN`, `domain56_compact.rs:24`.
pub const HEADER_LEN: usize = 44;
/// `ROW_LEN`, `domain56_compact.rs:25`.
pub const ROW_LEN: usize = 64;
/// `SCHEMA_VERSION`, `domain56_compact.rs:23`; the decoder rejects anything else
/// (`:209`-`:213`).
pub const SCHEMA_VERSION: u32 = 1;

/// `NAVAL_SUPREMACY_TRAILER_MAGIC`, `domain56_compact.rs:26`.
pub const TRAILER_MAGIC: [u8; 4] = *b"NSUP";
/// `NAVAL_SUPREMACY_TRAILER_VERSION`, `domain56_compact.rs:27`.
pub const TRAILER_VERSION: u16 = 1;
/// `NAVAL_SUPREMACY_TRAILER_HEADER_LEN`, `domain56_compact.rs:28`.
pub const TRAILER_HEADER_LEN: usize = 12;
/// `NAVAL_SUPREMACY_TRAILER_ROW_LEN`, `domain56_compact.rs:29`.
pub const TRAILER_ROW_LEN: usize = 32;
/// `MAX_NAVAL_SUPREMACY_ROWS`, `domain56_compact.rs:40`.
pub const MAX_NAVAL_SUPREMACY_ROWS: usize = 20_000;

/// `MAX_OOB_ID`, `domain56_compact.rs:30`.
pub const MAX_OOB_ID: u32 = 4_000_000;
/// `MAX_COMPACT_OWNER_TAG_DB_ID`, `compact_contract/src/build_profile.rs:45`,
/// reached through `domain56_compact.rs:31`. Named here because d56 is not
/// admitted to the production-codec dependency surface; the measurement bin
/// asserts the local value and shipping contract agree (the d24/d32 pattern).
pub const MAX_OWNER_TAG_DB_ID: u32 = 2_048;
/// `MAX_PROVINCE_ID`, `domain56_compact.rs:32`.
pub const MAX_PROVINCE_ID: u32 = 1_000_000;
/// `MAX_REGION_ID`, `domain56_compact.rs:33`.
pub const MAX_REGION_ID: u32 = 100_000;
/// `MAX_MISSION_ENUM`, `domain56_compact.rs:35`: the exact build-870ebf
/// `ENavalMission` maximum. The enum's cardinality is this **producer codomain**
/// plus one, never the capture histogram (which carries only 0,1,2,3,4,8,9).
pub const MAX_MISSION_ENUM: u32 = 9;
/// `MAX_MISSION_STATE`, `domain56_compact.rs:38`: the exact persisted
/// `detached_activity` enum maximum.
pub const MAX_MISSION_STATE: u32 = 4;
/// `MAX_SUNK_CONVOYS`, `domain56_compact.rs:39`.
pub const MAX_SUNK_CONVOYS: u32 = 1_000_000;

/// `NAVAL_SUPREMACY_FIXED_SCALE_RAW`, `domain56_compact.rs:41`, and the divisor
/// the fold's own projection applies to all six supremacy words
/// (`world_fold.rs:9019`-`:9024`, documented at `:9000`-`:9004`: they are
/// CFixedPoint dominance *points*, `raw / 100_000`, not a normalised fraction —
/// only `individual_ratio` is a ratio).
pub const FIXED_SCALE: f64 = 100_000.0;
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own
/// grid — see `fixed_five_is_lossless_over_every_u32_raw`.
pub const SCALE_DIGITS: u8 = 5;
/// The quantization carried by the six ported supremacy channels. `Fixed(5)` is
/// not a precision budget but the source's own grid, and it reproduces the raw
/// integer exactly over the whole `u32` range (proved by
/// `fixed_five_is_lossless_over_every_u32_raw`).
pub const SUPREMACY_QUANT: Quant = Quant::Fixed(SCALE_DIGITS);

/// `COMPACT_STATUS_DELTA_DATA`. Named here rather than imported, as above; the
/// measurement bin asserts it agrees with `compact_contract`'s.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
/// `STATUS_SOURCE_UNAVAILABLE`, `domain56_compact.rs:43`.
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
/// `STATUS_PARTIAL_DATA`, `domain56_compact.rs:44`.
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
/// `RETIRED_SOURCE_STATUS_MASK`, `domain56_compact.rs:100`-`:103`. The decoder
/// rejects these bits in the frame header (`:232`-`:237`) and in a row's
/// `source_status` word (`:1000`-`:1005`).
pub const RETIRED_SOURCE_STATUS_MASK: u32 = (1 << 16) | (1 << 17) | (1 << 18) | (1 << 19);
/// `KNOWN_SOURCE_STATUS_MASK`, `domain56_compact.rs:50`-`:56`, minus nothing: the
/// row check is `source_status & !KNOWN == 0` (`:994`-`:999`) *and*
/// `source_status & RETIRED == 0`, so bits 8, 9 and 10 are the only ones a row may
/// carry.
pub const KNOWN_SOURCE_STATUS_MASK: u32 =
    STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA | STATUS_DELTA_DATA | RETIRED_SOURCE_STATUS_MASK;
/// The three `source_status` bits a row may actually carry.
pub const LIVE_SOURCE_STATUS_MASK: u32 =
    STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA | STATUS_DELTA_DATA;

pub const FLAG_HAS_MISSION: u32 = 1 << 0;
pub const FLAG_HAS_MISSION_STATE: u32 = 1 << 1;
pub const FLAG_HAS_LOCATION: u32 = 1 << 2;
pub const FLAG_HAS_REGION: u32 = 1 << 3;
pub const FLAG_HAS_HOME_BASE: u32 = 1 << 4;
pub const FLAG_HAS_REPAIR_BASE: u32 = 1 << 5;
pub const FLAG_HAS_REPAIR_PROGRESS: u32 = 1 << 6;
pub const FLAG_HAS_RETREAT_STATE: u32 = 1 << 7;
pub const FLAG_HAS_CONVOY_ROUTE_CONTEXT: u32 = 1 << 8;
pub const FLAG_HAS_SUNK_CONVOY_COUNT: u32 = 1 << 9;
pub const FLAG_VALIDATED_DIRECT: u32 = 1 << 10;
pub const FLAG_DERIVED_REGION: u32 = 1 << 11;
pub const FLAG_FIXTURE_REQUIRED: u32 = 1 << 12;
pub const FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED: u32 = 1 << 13;
pub const FLAG_FIELD_REJECTED: u32 = 1 << 14;

/// `KNOWN_ROW_FLAGS`, `domain56_compact.rs:74`-`:88`.
pub const KNOWN_ROW_FLAGS: u32 = FLAG_HAS_MISSION
    | FLAG_HAS_MISSION_STATE
    | FLAG_HAS_LOCATION
    | FLAG_HAS_REGION
    | FLAG_HAS_HOME_BASE
    | FLAG_HAS_REPAIR_BASE
    | FLAG_HAS_REPAIR_PROGRESS
    | FLAG_HAS_RETREAT_STATE
    | FLAG_HAS_CONVOY_ROUTE_CONTEXT
    | FLAG_HAS_SUNK_CONVOY_COUNT
    | FLAG_VALIDATED_DIRECT
    | FLAG_DERIVED_REGION
    | FLAG_FIXTURE_REQUIRED
    | FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED
    | FLAG_FIELD_REJECTED;

/// `RETIRED_ROW_FLAGS`, `domain56_compact.rs:94`-`:99`. A row setting any of them
/// is rejected (`:922`-`:927`), which is why the three words they gate — and the
/// `FIELD_REJECTED` discriminator — are not fields in this schema.
pub const RETIRED_ROW_FLAGS: u32 = FLAG_HAS_REPAIR_PROGRESS
    | FLAG_HAS_RETREAT_STATE
    | FLAG_HAS_CONVOY_ROUTE_CONTEXT
    | FLAG_FIXTURE_REQUIRED
    | FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED
    | FLAG_FIELD_REJECTED;

/// The row flags a payload the fold accepts may actually carry.
pub const LIVE_ROW_FLAGS: u32 = KNOWN_ROW_FLAGS & !RETIRED_ROW_FLAGS;

pub fn schema() -> Schema {
    use field as f;

    let task_forces = Collection {
        collection_id: TASK_FORCES,
        // The fold keys its accumulator by `task_force_id` alone
        // (`world_fold.rs:22153`), a runtime identity that persists across hours,
        // which is what a keyed set needs — and only a keyed set has a delta
        // grammar (§7.3). Full replace on a non-delta frame, upsert on a delta
        // one, no tombstone contract (`:22138`-`:22143`).
        shape: Shape::KeyedSet,
        fields: vec![
            // A task force id is created during play and means nothing under
            // another campaign, let alone another mod set: a runtime instance
            // identity, so `Unsigned`, not `ContentId` (d20's `div_id` rule).
            Field::new(f::TASK_FORCE_ID, Kind::Unsigned).key(0),
            Field::new(f::FLEET_ID, Kind::Unsigned),
            // A country tag is content-defined (§5): a manifest must resolve it
            // for it to keep its meaning under another mod set. d63's `tag_db_id`
            // and d20's `owner` are the same call.
            Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId),
            // `ENavalMission`, 0..=9 (`domain56_compact.rs:35`, names at
            // `lib.rs:107`-`:118`). The cardinality is the producer's codomain, so
            // 10 — the capture's seven observed codes are not the enum.
            Field::new(f::MISSION_ENUM, Kind::Enum)
                .enum_card(MAX_MISSION_ENUM + 1)
                .unknown_capable(),
            // `detached_activity`, 0..=4 (`lib.rs:128`-`:134`).
            Field::new(f::MISSION_STATE, Kind::Enum)
                .enum_card(MAX_MISSION_STATE + 1)
                .unknown_capable(),
            // Provinces and strategic regions are content-defined (§5).
            Field::new(f::LOCATION_PROVINCE_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::STRATEGIC_REGION_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::HOME_BASE_PROVINCE_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::REPAIR_BASE_PROVINCE_ID, Kind::ContentId).unknown_capable(),
            // A cumulative count that ships carry with them, so transfers can
            // lower it; §4 makes `Monotone` a hint, never a validity constraint
            // (d20's `cumulative_losses`). No capability: a clear gate is a
            // measured zero, see the module docs.
            Field::new(f::SUNK_CONVOYS, Kind::Monotone),
            // Provenance witnesses the fold discards. Not derived from any carried
            // field and accepted in every combination by the decoder, so carrying
            // them is the only lossless mapping.
            Field::new(f::VALIDATED_DIRECT, Kind::Boolean),
            Field::new(f::DERIVED_REGION, Kind::Boolean),
            // The three live bits of the row's own `source_status` word
            // (`domain56_compact.rs:994`-`:1005`). Split into Booleans so the
            // retired bits are inexpressible rather than merely rejected.
            Field::new(f::SOURCE_UNAVAILABLE, Kind::Boolean),
            Field::new(f::SOURCE_PARTIAL_DATA, Kind::Boolean),
            Field::new(f::SOURCE_DELTA_DATA, Kind::Boolean),
        ],
    };

    let naval_supremacy = Collection {
        collection_id: NAVAL_SUPREMACY,
        // The fold clears and pushes every decoded row in emit order on every
        // successful frame (`world_fold.rs:22589`-`:22609`). Duplicate
        // `(region, controller)` pairs are legitimate, so there is no key.
        shape: Shape::OrderedOccurrenceList,
        fields: vec![
            Field::new(f::SUPREMACY_REGION_ID, Kind::ContentId),
            Field::new(f::SUPREMACY_CONTROLLER_TAG_DB_ID, Kind::ContentId),
            Field::new(f::SUPREMACY_CURRENT, Kind::Continuous).quant(SUPREMACY_QUANT),
            Field::new(f::SUPREMACY_TARGET, Kind::Continuous).quant(SUPREMACY_QUANT),
            Field::new(f::SUPREMACY_BASE_TARGET, Kind::Continuous).quant(SUPREMACY_QUANT),
            Field::new(f::SUPREMACY_PREVIOUS, Kind::Continuous).quant(SUPREMACY_QUANT),
            Field::new(f::SUPREMACY_DECLINE_FROM, Kind::Continuous).quant(SUPREMACY_QUANT),
            Field::new(f::SUPREMACY_INDIVIDUAL_RATIO, Kind::Continuous)
                .quant(SUPREMACY_QUANT),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        // The mixed-shape case §7 names: task forces use keyed deltas while the
        // occurrence list contributes a replacement only when its content moves.
        collections: vec![task_forces, naval_supremacy],
        event_part: None,
    }
}

// --- The payload boundary ----------------------------------------------------

/// The 44-byte payload header plus the trailer's row count
/// (`domain56_compact.rs:196`-`:206`, `:412`-`:414`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_task_forces: u32,
    pub validated_location_rows: u32,
    pub derived_region_rows: u32,
    pub fixture_required_rows: u32,
    pub convoy_sunk_count_rows: u32,
    pub schema_version: u32,
    /// Rows in the `NSUP` trailer; 0 when the trailer is absent, which is how the
    /// encoder writes an empty sidecar (`domain56_compact.rs:341`-`:344`).
    pub supremacy_row_count: u32,
}

impl PayloadHeader {
    /// `world_fold.rs:22138`.
    pub fn is_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// A split payload: the header, the task-force row bytes, and the supremacy row
/// bytes (trailer header already consumed).
#[derive(Clone, Copy, Debug)]
pub struct SplitPayload<'a> {
    pub header: PayloadHeader,
    pub rows: &'a [u8],
    pub supremacy_rows: &'a [u8],
}

/// Split a d56 DATA payload, applying every structural check
/// `naval56::decode_status_payload` makes before its per-row work
/// (`domain56_compact.rs:182`-`:281`) plus the whole trailer walk
/// (`:362`-`:470`).
///
/// The fold runs this decode **before** it consults the status word
/// (`world_fold.rs:22120`-`:22122`), so every error here is a refused stream and
/// not an hour to skip — including on a record this harness would otherwise
/// exclude for a failure status. The same decode is production's second authority
/// on every DOMAIN_DATA record (`domain_data_authority.rs:8633`).
///
/// Total: no payload of any length panics.
pub fn split_payload(payload: &[u8]) -> core::result::Result<SplitPayload<'_>, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "naval_missions_status payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
    }
    let row_count = u32_at(payload, 8);
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count,
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_task_forces: u32_at(payload, 20),
        validated_location_rows: u32_at(payload, 24),
        derived_region_rows: u32_at(payload, 28),
        fixture_required_rows: u32_at(payload, 32),
        convoy_sunk_count_rows: u32_at(payload, 36),
        schema_version: u32_at(payload, 40),
        supremacy_row_count: 0,
    };
    // `domain56_compact.rs:209`-`:213`.
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "naval_missions_status schema version {} != {SCHEMA_VERSION}",
            header.schema_version
        ));
    }
    // Checked arithmetic: `row_count` is a word off a stream on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough one.
    let base_len = (row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("naval_missions_status row_count {row_count} overflows"))?;
    // `domain56_compact.rs:214`-`:220`: at LEAST — the trailer follows the rows.
    if payload.len() < base_len {
        return Err(format!(
            "naval_missions_status payload is {} byte(s), expected at least {base_len}",
            payload.len()
        ));
    }
    // `:221`-`:226`. `changed_rows` is bounded, never required to equal
    // `row_count`: d56 is not a `changed_rows == row_count` full-replace grammar
    // (measured 0..=180 against 176..=182 rows on the batch-3 capture), and the
    // fold never reads the word at all.
    if header.changed_rows > row_count {
        return Err(format!(
            "naval_missions_status changed_rows {} exceeds row_count {row_count}",
            header.changed_rows
        ));
    }
    // `:227`-`:231`: the fixture counter is retired and must be zero.
    if header.fixture_required_rows != 0 {
        return Err("naval_missions_status retired fixture_required_rows must be zero".into());
    }
    // `:232`-`:237`.
    if header.status_flags & RETIRED_SOURCE_STATUS_MASK != 0 {
        return Err(format!(
            "naval_missions_status header uses retired fixture status bits {:#x}",
            header.status_flags & RETIRED_SOURCE_STATUS_MASK
        ));
    }
    // `:238`-`:245`.
    if header.validated_location_rows > row_count || header.derived_region_rows > row_count {
        return Err(format!(
            "naval_missions_status header row counters exceed row_count {row_count}"
        ));
    }

    let (supremacy_rows, supremacy_row_count) = split_trailer(payload, base_len)?;
    Ok(SplitPayload {
        header: PayloadHeader {
            supremacy_row_count,
            ..header
        },
        rows: &payload[HEADER_LEN..base_len],
        supremacy_rows,
    })
}

/// The trailer walk of `decode_domain56_trailers` (`domain56_compact.rs:362`-`:390`)
/// and `decode_naval_supremacy_trailer` (`:392`-`:470`): at most one `NSUP`
/// trailer, no other magic, no bytes after it.
fn split_trailer(payload: &[u8], base_len: usize) -> core::result::Result<(&[u8], u32), String> {
    if base_len == payload.len() {
        return Ok((&payload[payload.len()..], 0));
    }
    let trailer = &payload[base_len..];
    // `:371`: the magic read is a bounds-checked `get`, so a 1..3-byte tail is an
    // unknown-magic error rather than a panic.
    if trailer.len() < 4 || trailer[..4] != TRAILER_MAGIC {
        return Err(format!(
            "naval_missions_status unknown trailer magic at byte {base_len}"
        ));
    }
    // `:404`-`:411`.
    if trailer.len() < TRAILER_HEADER_LEN {
        return Err(format!(
            "naval_missions_status naval-supremacy trailer is {} byte(s), expected at least \
             {TRAILER_HEADER_LEN}",
            trailer.len()
        ));
    }
    let version = u16::from_le_bytes([trailer[4], trailer[5]]);
    let header_len = u16::from_le_bytes([trailer[6], trailer[7]]) as usize;
    let row_count = u32_at(trailer, 8) as usize;
    // `:415`-`:421`.
    if version != TRAILER_VERSION || header_len != TRAILER_HEADER_LEN {
        return Err(format!(
            "naval_missions_status naval-supremacy trailer header shape invalid (version \
             {version}, header_len {header_len})"
        ));
    }
    // `:422`-`:427`.
    if row_count > MAX_NAVAL_SUPREMACY_ROWS {
        return Err(format!(
            "naval_missions_status naval-supremacy row_count {row_count} exceeds \
             {MAX_NAVAL_SUPREMACY_ROWS}"
        ));
    }
    // `:428`-`:436`. `row_count` is already bounded by 20_000, so this cannot
    // overflow; the checked form states that rather than relying on it.
    let expected = row_count
        .checked_mul(TRAILER_ROW_LEN)
        .and_then(|len| len.checked_add(TRAILER_HEADER_LEN))
        .ok_or("naval_missions_status naval-supremacy trailer length overflows")?;
    if trailer.len() < expected {
        return Err(format!(
            "naval_missions_status naval-supremacy trailer len {} < {expected}",
            trailer.len()
        ));
    }
    // `:370`-`:388`: the walk continues past the trailer, so anything after it is
    // either a duplicate `NSUP` (`:372`-`:377`) or unknown magic (`:382`-`:387`).
    // Both are errors; the message names which.
    if trailer.len() > expected {
        let rest = &trailer[expected..];
        return Err(if rest.len() >= 4 && rest[..4] == TRAILER_MAGIC {
            "naval_missions_status duplicate naval-supremacy trailer".to_string()
        } else {
            format!(
                "naval_missions_status unknown trailer magic at byte {}",
                base_len + expected
            )
        });
    }
    Ok((
        &trailer[TRAILER_HEADER_LEN..expected],
        row_count.min(u32::MAX as usize) as u32,
    ))
}

// --- The task-force boundary mapping -----------------------------------------

/// The key of one compact task-force row: its `task_force_id`
/// (`world_fold.rs:22153`), with the decoder's own id check
/// (`domain56_compact.rs:908`, `:1027`-`:1033`).
pub fn key_from_task_force_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[task_force_id_of_row(row)? as u64]))
}

/// `task_force_id` with the decoder's `validate_oob_id` bounds.
pub fn task_force_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    validate_oob_id(u32_at(row, 0), "task_force_id")
}

/// One compact row of the `task_forces` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 64-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d56 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d56 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `validate_row` (`domain56_compact.rs:907`-
/// `:1007`): a row the decoder would reject is an `Err` here, never a coerced
/// value. Nothing in this function can panic on any 64-byte input.
pub fn record_from_task_force_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d56 has no derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A collection this mapper cannot fill would otherwise come out silently
    // wrong: the loop below matches on field ids, and another collection's ids
    // would take the `other` arm — but a collection with *no* value fields would
    // produce an empty record with no error at all
    // (`variant_upgrades.rs:633`'s rule).
    if collection.collection_id != TASK_FORCES {
        return Err(format!(
            "naval_missions_status task-force mapper called with collection {}",
            collection.collection_id
        ));
    }
    check_row_len(row)?;
    // The key is not carried in the record body (§3), but its validity is part of
    // this row's validity.
    let _ = task_force_id_of_row(row)?;
    validate_oob_id(u32_at(row, 4), "fleet_id")?;
    let owner_tag_db_id = u32_at(row, 8);
    // `domain56_compact.rs:910`-`:915`: an upper bound only. Zero is accepted by
    // the decoder, so it is accepted here — refusing it would be gating against
    // the fold.
    if owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
        return Err(format!(
            "naval_missions_status owner_tag_db_id {owner_tag_db_id} exceeds {MAX_OWNER_TAG_DB_ID}"
        ));
    }

    let flags = u32_at(row, 12);
    // `:916`-`:921`: an unrecognised flag bit means the row is not this grammar.
    if flags & !KNOWN_ROW_FLAGS != 0 {
        return Err(format!(
            "naval_missions_status row has unknown flags {:#x}",
            flags & !KNOWN_ROW_FLAGS
        ));
    }
    // `:922`-`:927`: the retired/unproven bits, including `FLAG_FIELD_REJECTED`.
    if flags & RETIRED_ROW_FLAGS != 0 {
        return Err(format!(
            "naval_missions_status row uses retired/unproven flags {:#x}",
            flags & RETIRED_ROW_FLAGS
        ));
    }

    let has_mission = flags & FLAG_HAS_MISSION != 0;
    let has_mission_state = flags & FLAG_HAS_MISSION_STATE != 0;
    let has_location = flags & FLAG_HAS_LOCATION != 0;
    let has_region = flags & FLAG_HAS_REGION != 0;
    let has_home_base = flags & FLAG_HAS_HOME_BASE != 0;
    let has_repair_base = flags & FLAG_HAS_REPAIR_BASE != 0;
    let has_sunk_convoys = flags & FLAG_HAS_SUNK_CONVOY_COUNT != 0;

    // `:931`-`:946`: zero is a REAL present value for both enums, and an absent
    // field must still pack a zero.
    let mission_enum = u32_at(row, 16);
    validate_optional_enum(has_mission, mission_enum, MAX_MISSION_ENUM, "mission_enum")?;
    let mission_state = u32_at(row, 20);
    validate_optional_enum(
        has_mission_state,
        mission_state,
        MAX_MISSION_STATE,
        "mission_state",
    )?;
    // `:947`-`:983`: a present id is in `1..=max`; an absent one packs a zero.
    let location_province_id = u32_at(row, 24);
    validate_optional_max(
        has_location,
        location_province_id,
        MAX_PROVINCE_ID,
        "location_province_id",
    )?;
    let strategic_region_id = u32_at(row, 28);
    validate_optional_max(
        has_region,
        strategic_region_id,
        MAX_REGION_ID,
        "strategic_region_id",
    )?;
    let home_base_province_id = u32_at(row, 32);
    validate_optional_max(
        has_home_base,
        home_base_province_id,
        MAX_PROVINCE_ID,
        "home_base_province_id",
    )?;
    let repair_base_province_id = u32_at(row, 36);
    validate_optional_max(
        has_repair_base,
        repair_base_province_id,
        MAX_PROVINCE_ID,
        "repair_base_province_id",
    )?;
    // `:975`-`:976`, `:984`-`:990`: the three retired words. Their gates are
    // retired flags, so `validate_optional_max` is called with `present = false`
    // and a maximum of zero — the word must be zero on every accepted row. That is
    // what makes them constants of the grammar rather than fields.
    validate_optional_max(false, u32_at(row, 40), 0, "repair_progress_milli")?;
    validate_optional_max(false, u32_at(row, 44), 0, "retreat_state")?;
    validate_optional_max(false, u32_at(row, 52), 0, "convoy_route_context_id")?;
    let sunk_convoys = u32_at(row, 48);
    validate_optional_max(
        has_sunk_convoys,
        sunk_convoys,
        MAX_SUNK_CONVOYS,
        "sunk_convoys",
    )?;
    // `:991`-`:993`.
    if u32_at(row, 60) != 0 {
        return Err("naval_missions_status row reserved0 must be zero".into());
    }
    // `:994`-`:1005`.
    let source_status = u32_at(row, 56);
    if source_status & !KNOWN_SOURCE_STATUS_MASK != 0 {
        return Err(format!(
            "naval_missions_status row source_status has unknown bits {:#x}",
            source_status & !KNOWN_SOURCE_STATUS_MASK
        ));
    }
    if source_status & RETIRED_SOURCE_STATUS_MASK != 0 {
        return Err(format!(
            "naval_missions_status row uses retired fixture status bits {:#x}",
            source_status & RETIRED_SOURCE_STATUS_MASK
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::FLEET_ID => Value::Unsigned(u32_at(row, 4) as u64),
            field::OWNER_TAG_DB_ID => Value::ContentId(owner_tag_db_id as u64),
            field::MISSION_ENUM => gated_enum(has_mission, mission_enum),
            field::MISSION_STATE => gated_enum(has_mission_state, mission_state),
            field::LOCATION_PROVINCE_ID => gated_content_id(has_location, location_province_id),
            field::STRATEGIC_REGION_ID => gated_content_id(has_region, strategic_region_id),
            field::HOME_BASE_PROVINCE_ID => gated_content_id(has_home_base, home_base_province_id),
            field::REPAIR_BASE_PROVINCE_ID => {
                gated_content_id(has_repair_base, repair_base_province_id)
            }
            // The one value-derived gate: a clear gate is the measured zero the
            // decoder requires the row to pack, not an empty state.
            field::SUNK_CONVOYS => Value::Unsigned(sunk_convoys as u64),
            field::VALIDATED_DIRECT => Value::Bool(flags & FLAG_VALIDATED_DIRECT != 0),
            field::DERIVED_REGION => Value::Bool(flags & FLAG_DERIVED_REGION != 0),
            field::SOURCE_UNAVAILABLE => {
                Value::Bool(source_status & STATUS_SOURCE_UNAVAILABLE != 0)
            }
            field::SOURCE_PARTIAL_DATA => Value::Bool(source_status & STATUS_PARTIAL_DATA != 0),
            field::SOURCE_DELTA_DATA => Value::Bool(source_status & STATUS_DELTA_DATA != 0),
            other => {
                // Filling an unknown field with a placeholder would publish
                // invented data under a field id that means something else.
                return Err(format!(
                    "naval_missions_status has no mapping for task-force field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The naval-supremacy occurrence list -------------------------------------

/// Validate one row of the `NSUP` trailer, mirroring
/// `validate_naval_supremacy_row` (`domain56_compact.rs:1009`-`:1025`).
///
/// Validation remains separate from mapping because the fold decodes the whole
/// payload, trailer included, before it folds anything
/// (`world_fold.rs:22511`-`:22513`): a row this rejects is a refused stream even
/// on a non-folding status path.
///
/// `ordinal` names the row's position in the trailer for the error message only.
/// Nothing in this function can panic on any input.
pub fn check_supremacy_row(row: &[u8], ordinal: u32) -> core::result::Result<(), String> {
    if row.len() != TRAILER_ROW_LEN {
        return Err(format!(
            "naval_missions_status supremacy row {ordinal} length {} != {TRAILER_ROW_LEN}",
            row.len()
        ));
    }
    // `domain56_compact.rs:1010`-`:1017`: both ids are checked with
    // `validate_optional_max(present = true, ...)`, which rejects zero as well as
    // an out-of-range value.
    validate_optional_max(
        true,
        u32_at(row, supremacy_offset::REGION_ID),
        MAX_REGION_ID,
        "supremacy region_id",
    )?;
    validate_optional_max(
        true,
        u32_at(row, supremacy_offset::CONTROLLER_TAG_DB_ID),
        MAX_OWNER_TAG_DB_ID,
        "supremacy controller_tag_db_id",
    )?;
    // `:1018`-`:1023`: only the ratio is bounded; the five point-valued words are
    // unbounded u32.
    let individual_ratio_raw = u32_at(row, supremacy_offset::INDIVIDUAL_RATIO);
    if individual_ratio_raw > FIXED_SCALE_RAW {
        return Err(format!(
            "naval_missions_status supremacy row {ordinal} individual_ratio_raw \
             {individual_ratio_raw} exceeds {FIXED_SCALE_RAW}"
        ));
    }
    Ok(())
}

/// One `NSUP` row onto its ordered-occurrence record. Every field is carried:
/// §3 gives an occurrence list no key prefix, and dropping either id would make
/// distinct game rows indistinguishable.
pub fn record_from_supremacy_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_supremacy_row(row, 0)?;
    if collection.shape != Shape::OrderedOccurrenceList {
        return Err("naval_supremacy collection is not an ordered occurrence list".into());
    }
    let scaled = |offset| Value::Continuous(u32_at(row, offset) as f64 / FIXED_SCALE);
    let mut values = Vec::with_capacity(collection.fields.len());
    for field in collection.value_fields() {
        let value = match field.field_id {
            field::SUPREMACY_REGION_ID => {
                Value::ContentId(u32_at(row, supremacy_offset::REGION_ID) as u64)
            }
            field::SUPREMACY_CONTROLLER_TAG_DB_ID => {
                Value::ContentId(u32_at(row, supremacy_offset::CONTROLLER_TAG_DB_ID) as u64)
            }
            field::SUPREMACY_CURRENT => scaled(supremacy_offset::CURRENT),
            field::SUPREMACY_TARGET => scaled(supremacy_offset::TARGET),
            field::SUPREMACY_BASE_TARGET => scaled(supremacy_offset::BASE_TARGET),
            field::SUPREMACY_PREVIOUS => scaled(supremacy_offset::PREVIOUS),
            field::SUPREMACY_DECLINE_FROM => scaled(supremacy_offset::DECLINE_FROM),
            field::SUPREMACY_INDIVIDUAL_RATIO => scaled(supremacy_offset::INDIVIDUAL_RATIO),
            other => {
                return Err(format!(
                    "naval_supremacy has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame boundary ------------------------------------------------------

/// One payload, fully decoded and validated: everything the fold does before it
/// looks at the status word.
#[derive(Clone, Debug)]
pub struct DecodedPayload {
    pub header: PayloadHeader,
    pub task_forces: Vec<(Key, Record)>,
    /// Every sidecar row in emit order. A `Vec` is intentional: duplicates and
    /// position are both fold state (`world_fold.rs:22589`-`:22609`).
    pub naval_supremacy: Vec<Record>,
}

/// Decode and validate a whole d56 DATA payload — `naval56::decode_status_payload`
/// (`domain56_compact.rs:182`-`:292`), which is both the fold's first act
/// (`world_fold.rs:22120`-`:22122`) and production's second authority on every
/// DOMAIN_DATA record whatever its status
/// (`domain_data_authority.rs:8628`-`:8653`).
///
/// Separate from [`State::fold_payload`] because a caller must be able to run it
/// on a record it will then exclude: the decode is status-independent and its
/// `Err` is a refused stream, never an hour to skip.
pub fn decode_payload(
    schema: &Schema,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<DecodedPayload, String> {
    if schema.collections.len() != 2 {
        return Err(format!(
            "naval_missions_status schema has {} collections, expected 2",
            schema.collections.len()
        ));
    }
    let task_forces_collection = schema
        .collections
        .iter()
        .find(|c| c.collection_id == TASK_FORCES)
        .ok_or("naval_missions_status schema has no task_forces collection")?;
    let supremacy_collection = schema
        .collections
        .iter()
        .find(|c| c.collection_id == NAVAL_SUPREMACY)
        .ok_or("naval_missions_status schema has no naval_supremacy collection")?;

    let split = split_payload(payload)?;
    let mut task_forces: Vec<(Key, Record)> = Vec::new();
    let mut seen: KeyedState = KeyedState::new();
    for (index, row) in split.rows.chunks_exact(ROW_LEN).enumerate() {
        let key = key_from_task_force_row(row)
            .map_err(|e| format!("naval_missions_status row {index}: {e}"))?;
        let record = record_from_task_force_row(task_forces_collection, row, frame_tick)
            .map_err(|e| format!("naval_missions_status row {index}: {e}"))?;
        // `domain56_compact.rs:269`-`:274`: a repeated `task_force_id` is a
        // rejected frame, never a last-writer-wins overwrite. It lives in the
        // decoder, not the fold, so it runs on excluded records too.
        if seen.insert(key.clone(), record.clone()).is_some() {
            return Err(format!(
                "naval_missions_status duplicate task_force_id at row {index}"
            ));
        }
        task_forces.push((key, record));
    }
    let mut naval_supremacy = Vec::with_capacity(split.header.supremacy_row_count as usize);
    for (index, row) in split.supremacy_rows.chunks_exact(TRAILER_ROW_LEN).enumerate() {
        check_supremacy_row(row, index.min(u32::MAX as usize) as u32)?;
        naval_supremacy.push(
            record_from_supremacy_row(supremacy_collection, row, frame_tick)
                .map_err(|e| format!("naval supremacy row {index}: {e}"))?,
        );
    }
    Ok(DecodedPayload {
        header: split.header,
        task_forces,
        naval_supremacy,
    })
}

/// The accumulator `fold_naval_missions_status` maintains: the task-force map
/// (`world_fold.rs:1718`) and the supremacy array (`:1726`), plus the
/// `delta_baselines_seen` membership the fold's delta-before-baseline check reads
/// (`:22130`-`:22136`, `:22220`-`:22223`).
///
/// **No other writer.** Both accumulator fields are touched only by
/// `fold_naval_missions_status` (`world_fold.rs:22143`, `:22153`, `:22203`,
/// `:22208`); every other mention is a read
/// (`:8417`, `:8600`, `:9012`). Nothing to model, prove inert, or assert inert.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub task_forces: KeyedState,
    pub naval_supremacy: Vec<Record>,
    pub baseline_seen: bool,
}

/// What one payload did to the accumulator. The three outcomes are the recipe's
/// three: a folded hour, and the fold's two non-folding exits. A fourth exists and
/// is not in this enum — an `Err` from [`fold_payload`], which is the payload not
/// being this grammar and therefore a **stream rejection**, never an hour to skip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    Folded,
    /// `world_fold.rs:22123`-`:22129` — `handle_compact_failure` marks the domain
    /// unknown and retains the accumulator. The caller classifies the status word
    /// with `compact_contract`'s own predicate and passes the answer in, so this
    /// module never re-derives a failure mask.
    FailureStatus,
    /// `world_fold.rs:22130`-`:22136`.
    DeltaBeforeBaseline,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one d56 DATA payload exactly as `fold_naval_missions_status` does.
    ///
    /// `status_is_failure` is the caller's classification of
    /// `header.status_flags` — the fold's own
    /// `compact_failure_flags_after_expected_partial_tolerance(56, ...) != 0`
    /// (`world_fold.rs:2872`-`:2874`). It is an input rather than something
    /// re-derived here so the library carries no copied status mask.
    ///
    /// **Error contract: all-or-nothing.** Every structural and per-row check runs
    /// and every record is built before a single accumulator write, which is what
    /// the fold does — `decode_status_payload` decodes and validates the whole
    /// payload before the fold touches its maps (`world_fold.rs:22120`-`:22152`).
    /// A harness that assumed the partial-mutation contract instead would disagree
    /// with the recorder on exactly the frames that fail.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        status_is_failure: bool,
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        // 1. The whole payload, decoded and validated first — the fold's order.
        let decoded = decode_payload(schema, payload, frame_tick)?;

        // 2. Failure status: the accumulator is retained and nothing is folded.
        if status_is_failure {
            return Ok(FoldOutcome::FailureStatus);
        }
        // 3. Delta before baseline: same.
        let delta = decoded.header.is_delta();
        if delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        // 4. `world_fold.rs:22139`-`:22144`: full-replace frames with no deletion
        // tombstone contract, so only a non-delta frame may clear stale rows.
        if !delta {
            self.task_forces.clear();
        }
        for (key, record) in decoded.task_forces {
            self.task_forces.insert(key, record);
        }
        // 5. `:22589`-`:22609`: the sidecar is full-replaced on EVERY successful
        // frame regardless of the task-force delta flag, in emit order.
        self.naval_supremacy = decoded.naval_supremacy;
        // 6. `:22220`-`:22223`.
        if !delta {
            self.baseline_seen = true;
        }
        Ok(FoldOutcome::Folded)
    }

    /// The collection states in schema order, preserving each declared shape.
    pub fn states(&self) -> [CollectionState; 2] {
        [
            CollectionState::Keyed(self.task_forces.clone()),
            CollectionState::OrderedOccurrence(self.naval_supremacy.clone()),
        ]
    }
}

// --- Shared helpers ----------------------------------------------------------

/// A gate that is clear means the collector could not capture the value — see the
/// module docs. §3 calls that **unknown**.
fn gated_enum(present: bool, value: u32) -> Value {
    if present {
        Value::Enum(value)
    } else {
        Value::Unknown
    }
}

fn gated_content_id(present: bool, value: u32) -> Value {
    if present {
        Value::ContentId(value as u64)
    } else {
        Value::Unknown
    }
}

/// `validate_oob_id`, `domain56_compact.rs:1027`-`:1033`.
fn validate_oob_id(value: u32, label: &str) -> core::result::Result<u32, String> {
    if value == 0 || value > MAX_OOB_ID {
        Err(format!(
            "naval_missions_status {label} has invalid id {value}"
        ))
    } else {
        Ok(value)
    }
}

/// `validate_optional_max`, `domain56_compact.rs:1035`-`:1053`: a present value is
/// in `1..=max`, and an absent one must pack a zero.
fn validate_optional_max(
    present: bool,
    value: u32,
    max_value: u32,
    label: &str,
) -> core::result::Result<(), String> {
    if present {
        if value == 0 || value > max_value {
            Err(format!(
                "naval_missions_status {label} has invalid value {value}"
            ))
        } else {
            Ok(())
        }
    } else if value != 0 {
        Err(format!(
            "naval_missions_status {label} has value without flag"
        ))
    } else {
        Ok(())
    }
}

/// `validate_optional_enum`, `domain56_compact.rs:1058`-`:1076`: like
/// `validate_optional_max` but zero is a real present value.
fn validate_optional_enum(
    present: bool,
    value: u32,
    max_value: u32,
    label: &str,
) -> core::result::Result<(), String> {
    if present {
        if value > max_value {
            Err(format!(
                "naval_missions_status {label} has invalid value {value}"
            ))
        } else {
            Ok(())
        }
    } else if value != 0 {
        Err(format!(
            "naval_missions_status {label} has value without flag"
        ))
    } else {
        Ok(())
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "naval_missions_status row length {} != {ROW_LEN}",
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
    use crate::value::{escapes, quantize};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn task_force_collection() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// The sixteen row words, in order, the way the lane packs them
    /// (`domain56_compact.rs:320`-`:335`).
    #[derive(Clone, Copy)]
    struct RawRow {
        task_force_id: u32,
        fleet_id: u32,
        owner_tag_db_id: u32,
        flags: u32,
        mission_enum: u32,
        mission_state: u32,
        location_province_id: u32,
        strategic_region_id: u32,
        home_base_province_id: u32,
        repair_base_province_id: u32,
        repair_progress_milli: u32,
        retreat_state: u32,
        sunk_convoys: u32,
        convoy_route_context_id: u32,
        source_status: u32,
        reserved0: u32,
    }

    impl Default for RawRow {
        fn default() -> Self {
            Self {
                task_force_id: 617,
                fleet_id: 615,
                owner_tag_db_id: 47,
                flags: FLAG_VALIDATED_DIRECT,
                mission_enum: 0,
                mission_state: 0,
                location_province_id: 0,
                strategic_region_id: 0,
                home_base_province_id: 0,
                repair_base_province_id: 0,
                repair_progress_milli: 0,
                retreat_state: 0,
                sunk_convoys: 0,
                convoy_route_context_id: 0,
                source_status: 0,
                reserved0: 0,
            }
        }
    }

    impl RawRow {
        fn bytes(self) -> Vec<u8> {
            let words = [
                self.task_force_id,
                self.fleet_id,
                self.owner_tag_db_id,
                self.flags,
                self.mission_enum,
                self.mission_state,
                self.location_province_id,
                self.strategic_region_id,
                self.home_base_province_id,
                self.repair_base_province_id,
                self.repair_progress_milli,
                self.retreat_state,
                self.sunk_convoys,
                self.convoy_route_context_id,
                self.source_status,
                self.reserved0,
            ];
            let mut row = Vec::with_capacity(ROW_LEN);
            for w in words {
                row.extend_from_slice(&w.to_le_bytes());
            }
            assert_eq!(row.len(), ROW_LEN);
            row
        }
    }

    /// Every live gate set, every carried word distinct and non-zero.
    fn full_row() -> Vec<u8> {
        RawRow {
            task_force_id: 617,
            fleet_id: 615,
            owner_tag_db_id: 47,
            flags: FLAG_HAS_MISSION
                | FLAG_HAS_MISSION_STATE
                | FLAG_HAS_LOCATION
                | FLAG_HAS_REGION
                | FLAG_HAS_HOME_BASE
                | FLAG_HAS_REPAIR_BASE
                | FLAG_HAS_SUNK_CONVOY_COUNT
                | FLAG_VALIDATED_DIRECT
                | FLAG_DERIVED_REGION,
            mission_enum: 3,
            mission_state: 4,
            location_province_id: 12_389,
            strategic_region_id: 31,
            home_base_province_id: 9_001,
            repair_base_province_id: 9_002,
            sunk_convoys: 2,
            source_status: LIVE_SOURCE_STATUS_MASK,
            ..RawRow::default()
        }
        .bytes()
    }

    fn supremacy_row(words: [u32; 8]) -> Vec<u8> {
        let mut row = Vec::with_capacity(TRAILER_ROW_LEN);
        for w in words {
            row.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(row.len(), TRAILER_ROW_LEN);
        row
    }

    fn full_supremacy_row() -> Vec<u8> {
        supremacy_row([
            18,
            44,
            448_057_205,
            448_057_206,
            76_245_200,
            448_057_204,
            448_057_203,
            97_816,
        ])
    }

    /// A whole payload: header, rows, and an `NSUP` trailer when non-empty.
    fn payload(
        status_flags: u32,
        changed_rows: u32,
        rows: &[Vec<u8>],
        supremacy: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&7u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&changed_rows.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed
        p.extend_from_slice(&0u32.to_le_bytes()); // validated_location_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // derived_region_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // fixture_required_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // convoy_sunk_count_rows
        p.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        assert_eq!(p.len(), HEADER_LEN);
        for row in rows {
            p.extend_from_slice(row);
        }
        if !supremacy.is_empty() {
            p.extend_from_slice(&TRAILER_MAGIC);
            p.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
            p.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
            p.extend_from_slice(&(supremacy.len() as u32).to_le_bytes());
            for row in supremacy {
                p.extend_from_slice(row);
            }
        }
        p
    }

    // --- Step 8.1: the schema is valid ---------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d56 schema is well-formed");
    }

    #[test]
    fn the_supremacy_wire_ids_scale_and_zero_quant_code_are_literal_pinned() {
        assert_eq!(NAVAL_SUPREMACY, 2);
        assert_eq!(field::SUPREMACY_REGION_ID, 20);
        assert_eq!(field::SUPREMACY_CONTROLLER_TAG_DB_ID, 21);
        assert_eq!(field::SUPREMACY_CURRENT, 22);
        assert_eq!(field::SUPREMACY_TARGET, 23);
        assert_eq!(field::SUPREMACY_BASE_TARGET, 24);
        assert_eq!(field::SUPREMACY_PREVIOUS, 25);
        assert_eq!(field::SUPREMACY_DECLINE_FROM, 26);
        assert_eq!(field::SUPREMACY_INDIVIDUAL_RATIO, 27);
        assert_eq!(FIXED_SCALE, 100_000.0);
        assert_eq!(FIXED_SCALE_RAW, 100_000);
        assert_eq!(SUPREMACY_QUANT, Quant::Fixed(5));
        assert_eq!(Quant::None.code(), 0);
        assert_eq!(Quant::Exact.code(), 0);
        assert_eq!(Quant::None.param(), None);
        assert_eq!(Quant::Exact.param(), None);
    }

    // --- Step 8.5: shape and key ---------------------------------------------

    #[test]
    fn the_task_force_table_is_keyed_by_the_task_force() {
        let c = task_force_collection();
        assert_eq!(c.shape, Shape::KeyedSet);
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by task_force_id alone");
        assert_eq!(keys[0].field_id, field::TASK_FORCE_ID);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "a task force id is a runtime instance identity, not content"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 15);
    }

    #[test]
    fn the_supremacy_sidecar_is_an_ordered_occurrence_collection() {
        // `WIRE_FORMAT.md` §7.3 names this array an ordered occurrence list; the
        // schema now carries exactly that shape with no fabricated key.
        let s = schema();
        assert_eq!(s.collections.len(), 2);
        assert_eq!(s.collections[0].collection_id, TASK_FORCES);
        assert_eq!(s.collections[1].collection_id, NAVAL_SUPREMACY);
        assert_eq!(s.collections[1].shape, Shape::OrderedOccurrenceList);
        assert!(s.collections[1].key_fields().is_empty());
        assert_eq!(
            s.collections[1]
                .fields
                .iter()
                .map(|field| (field.field_id, field.kind, field.quant))
                .collect::<Vec<_>>(),
            vec![
                (field::SUPREMACY_REGION_ID, Kind::ContentId, Quant::None),
                (
                    field::SUPREMACY_CONTROLLER_TAG_DB_ID,
                    Kind::ContentId,
                    Quant::None
                ),
                (
                    field::SUPREMACY_CURRENT,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
                (
                    field::SUPREMACY_TARGET,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
                (
                    field::SUPREMACY_BASE_TARGET,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
                (
                    field::SUPREMACY_PREVIOUS,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
                (
                    field::SUPREMACY_DECLINE_FROM,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
                (
                    field::SUPREMACY_INDIVIDUAL_RATIO,
                    Kind::Continuous,
                    SUPREMACY_QUANT
                ),
            ],
            "both ids and all six source-grid values are byte-shape decisions"
        );
    }

    #[test]
    fn a_duplicated_supremacy_row_is_valid_input_which_requires_the_occurrence_shape() {
        // The premise of the shape, tested rather than asserted: two entries with
        // the same `(region, controller)` and different values are accepted by the
        // decoder (`domain56_compact.rs:438`-`:449`), so any keyed reading of this
        // array must either lose one of them or key by position — and §7.3 says
        // position is not an identity here. Both rows validate and both are mapped.
        let a = supremacy_row([29, 22, 100_000, 0, 0, 0, 0, 1]);
        let b = supremacy_row([29, 22, 200_000, 0, 0, 0, 0, 2]);
        assert_ne!(a, b, "the value payloads differ");
        check_supremacy_row(&a, 0).expect("a duplicate pair is legal input");
        check_supremacy_row(&b, 1).expect("a duplicate pair is legal input");
        // A whole payload carrying both folds, and the fold counts two rows.
        let mut state = State::new();
        state
            .fold_payload(&schema(), &payload(0, 0, &[], &[a, b]), false, 0)
            .expect("folds");
        assert_eq!(state.naval_supremacy.len(), 2);
    }

    // --- Step 8.3: the four states, through the mapper ------------------------

    #[test]
    fn the_declared_capabilities_are_exactly_unknown_on_the_gated_fields() {
        // Absent would claim the concept does not apply to a task force; Null would
        // claim we know there is nothing. The collector clears these gates only
        // when the read failed or could not be validated (`lib.rs:515`-`:582`),
        // which is §3's "applicable, not captured".
        let c = task_force_collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        let unknown_capable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown_capable,
            vec![
                field::MISSION_ENUM,
                field::MISSION_STATE,
                field::LOCATION_PROVINCE_ID,
                field::STRATEGIC_REGION_ID,
                field::HOME_BASE_PROVINCE_ID,
                field::REPAIR_BASE_PROVINCE_ID,
            ],
            "exactly the six flag-gated fields"
        );
        // The occurrence collection declares no capability at all — every one of
        // its words is present on every row the decoder accepts — so this remains
        // the port's only collection with capabilities.
    }

    #[test]
    fn the_mapper_emits_values_for_a_full_row() {
        let c = task_force_collection();
        let r = record_from_task_force_row(&c, &full_row(), 0).expect("maps");
        for (id, expected) in [
            (field::FLEET_ID, Value::Unsigned(615)),
            (field::OWNER_TAG_DB_ID, Value::ContentId(47)),
            (field::MISSION_ENUM, Value::Enum(3)),
            (field::MISSION_STATE, Value::Enum(4)),
            (field::LOCATION_PROVINCE_ID, Value::ContentId(12_389)),
            (field::STRATEGIC_REGION_ID, Value::ContentId(31)),
            (field::HOME_BASE_PROVINCE_ID, Value::ContentId(9_001)),
            (field::REPAIR_BASE_PROVINCE_ID, Value::ContentId(9_002)),
            (field::SUNK_CONVOYS, Value::Unsigned(2)),
            (field::VALIDATED_DIRECT, Value::Bool(true)),
            (field::DERIVED_REGION, Value::Bool(true)),
            (field::SOURCE_UNAVAILABLE, Value::Bool(true)),
            (field::SOURCE_PARTIAL_DATA, Value::Bool(true)),
            (field::SOURCE_DELTA_DATA, Value::Bool(true)),
        ] {
            assert_eq!(r.values[index_of(&c, id)], expected, "field {id}");
        }
    }

    #[test]
    fn a_clear_gate_is_unknown_field_by_field() {
        // One gate clear at a time — the whole gate space, not one sampled case.
        let c = task_force_collection();
        let cases: [(u32, u32); 6] = [
            (FLAG_HAS_MISSION, field::MISSION_ENUM),
            (FLAG_HAS_MISSION_STATE, field::MISSION_STATE),
            (FLAG_HAS_LOCATION, field::LOCATION_PROVINCE_ID),
            (FLAG_HAS_REGION, field::STRATEGIC_REGION_ID),
            (FLAG_HAS_HOME_BASE, field::HOME_BASE_PROVINCE_ID),
            (FLAG_HAS_REPAIR_BASE, field::REPAIR_BASE_PROVINCE_ID),
        ];
        for (cleared, gated_id) in cases {
            let mut raw = RawRow {
                flags: (LIVE_ROW_FLAGS & !cleared),
                mission_enum: 3,
                mission_state: 4,
                location_province_id: 12_389,
                strategic_region_id: 31,
                home_base_province_id: 9_001,
                repair_base_province_id: 9_002,
                sunk_convoys: 2,
                ..RawRow::default()
            };
            // A gated-off field must pack a zero, so build the row that way.
            match cleared {
                FLAG_HAS_MISSION => raw.mission_enum = 0,
                FLAG_HAS_MISSION_STATE => raw.mission_state = 0,
                FLAG_HAS_LOCATION => raw.location_province_id = 0,
                FLAG_HAS_REGION => raw.strategic_region_id = 0,
                FLAG_HAS_HOME_BASE => raw.home_base_province_id = 0,
                FLAG_HAS_REPAIR_BASE => raw.repair_base_province_id = 0,
                _ => unreachable!(),
            }
            let r = record_from_task_force_row(&c, &raw.bytes(), 0)
                .unwrap_or_else(|e| panic!("maps with flags {:#x}: {e}", raw.flags));
            for f in c.value_fields() {
                assert_eq!(
                    r.values[index_of(&c, f.field_id)] == Value::Unknown,
                    f.field_id == gated_id,
                    "field {} under flags {:#x}",
                    f.field_id,
                    raw.flags
                );
            }
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Both zero cases the grammar has. `sunk_convoys` with its gate clear is
        // the measured zero the decoder requires the row to pack — the fidelity
        // gain over the fold, which folds Null there (`world_fold.rs:22189`).
        let c = task_force_collection();
        let raw = RawRow {
            flags: FLAG_HAS_MISSION | FLAG_HAS_MISSION_STATE | FLAG_VALIDATED_DIRECT,
            mission_enum: 0,
            mission_state: 0,
            ..RawRow::default()
        };
        let r = record_from_task_force_row(&c, &raw.bytes(), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::SUNK_CONVOYS)],
            Value::Unsigned(0),
            "a clear sunk-convoy gate is a measured zero, never Unknown"
        );
        // And enum code 0 under a SET gate is a real member ("HOLD" / "none"),
        // which is why the decoder uses `validate_optional_enum` for both
        // (`domain56_compact.rs:928`-`:946`).
        assert_eq!(r.values[index_of(&c, field::MISSION_ENUM)], Value::Enum(0));
        assert_eq!(r.values[index_of(&c, field::MISSION_STATE)], Value::Enum(0));
        // A zero supremacy value word is legal input too (only the two ids are
        // required non-zero), which the stop report's validator has to allow.
        check_supremacy_row(&supremacy_row([1, 1, 0, 0, 0, 0, 0, 0]), 0)
            .expect("zero-valued dominance entries are legal");
    }

    #[test]
    fn the_witness_booleans_track_their_bits_independently() {
        let c = task_force_collection();
        let cases: [(u32, u32, u32); 5] = [
            (FLAG_VALIDATED_DIRECT, 0, field::VALIDATED_DIRECT),
            (FLAG_DERIVED_REGION, 0, field::DERIVED_REGION),
            (0, STATUS_SOURCE_UNAVAILABLE, field::SOURCE_UNAVAILABLE),
            (0, STATUS_PARTIAL_DATA, field::SOURCE_PARTIAL_DATA),
            (0, STATUS_DELTA_DATA, field::SOURCE_DELTA_DATA),
        ];
        for (flag, status, id) in cases {
            let raw = RawRow {
                flags: flag,
                source_status: status,
                ..RawRow::default()
            };
            let r = record_from_task_force_row(&c, &raw.bytes(), 0).expect("maps");
            for f in c.value_fields() {
                if matches!(f.kind, Kind::Boolean) {
                    assert_eq!(
                        r.values[index_of(&c, f.field_id)],
                        Value::Bool(f.field_id == id),
                        "boolean {} with flag {flag:#x} status {status:#x}",
                        f.field_id
                    );
                }
            }
        }
    }

    #[test]
    fn the_unknown_state_survives_a_roundtrip() {
        let c = task_force_collection();
        let raw = RawRow {
            flags: FLAG_VALIDATED_DIRECT,
            ..RawRow::default()
        };
        let record = record_from_task_force_row(&c, &raw.bytes(), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        for id in [
            field::MISSION_ENUM,
            field::MISSION_STATE,
            field::LOCATION_PROVINCE_ID,
            field::STRATEGIC_REGION_ID,
            field::HOME_BASE_PROVINCE_ID,
            field::REPAIR_BASE_PROVINCE_ID,
        ] {
            assert_eq!(back.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
        assert_eq!(
            back.values[index_of(&c, field::SUNK_CONVOYS)],
            Value::Unsigned(0)
        );
    }

    // --- Step 8.2: byte-identical round trips ---------------------------------

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        for (c, record) in [(
            task_force_collection(),
            record_from_task_force_row(&task_force_collection(), &full_row(), 0).expect("maps"),
        )] {
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
            assert_eq!(back, record, "decode reproduces the record");
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
        }
    }

    #[test]
    fn a_fully_gated_off_row_roundtrips_byte_for_byte() {
        let c = task_force_collection();
        let record = record_from_task_force_row(&c, &RawRow::default().bytes(), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    // --- Step 8.6: every field's offset, and the scale ------------------------

    #[test]
    fn every_task_force_field_reads_its_own_offset() {
        // Sixteen distinct words, so a transposition of any pair fails — the same
        // failure class as a wrong scale (d62's adjacent `owner`/`actor`).
        let c = task_force_collection();
        let raw = RawRow {
            task_force_id: 1_000_001,
            fleet_id: 1_000_002,
            owner_tag_db_id: 2_048,
            flags: LIVE_ROW_FLAGS,
            mission_enum: 9,
            mission_state: 4,
            location_province_id: 1_000_000,
            strategic_region_id: 100_000,
            home_base_province_id: 999_999,
            repair_base_province_id: 999_998,
            sunk_convoys: 1_000_000,
            source_status: STATUS_PARTIAL_DATA,
            ..RawRow::default()
        };
        let row = raw.bytes();
        assert_eq!(key_from_task_force_row(&row).expect("keys"), key(&[1_000_001]));
        let r = record_from_task_force_row(&c, &row, 0).expect("maps");
        for (id, expected) in [
            (field::FLEET_ID, Value::Unsigned(1_000_002)),
            (field::OWNER_TAG_DB_ID, Value::ContentId(2_048)),
            (field::MISSION_ENUM, Value::Enum(9)),
            (field::MISSION_STATE, Value::Enum(4)),
            (field::LOCATION_PROVINCE_ID, Value::ContentId(1_000_000)),
            (field::STRATEGIC_REGION_ID, Value::ContentId(100_000)),
            (field::HOME_BASE_PROVINCE_ID, Value::ContentId(999_999)),
            (field::REPAIR_BASE_PROVINCE_ID, Value::ContentId(999_998)),
            (field::SUNK_CONVOYS, Value::Unsigned(1_000_000)),
            (field::VALIDATED_DIRECT, Value::Bool(true)),
            (field::DERIVED_REGION, Value::Bool(true)),
            (field::SOURCE_UNAVAILABLE, Value::Bool(false)),
            (field::SOURCE_PARTIAL_DATA, Value::Bool(true)),
            (field::SOURCE_DELTA_DATA, Value::Bool(false)),
        ] {
            assert_eq!(r.values[index_of(&c, id)], expected, "field {id}");
        }
    }

    #[test]
    fn the_supremacy_offsets_and_the_hundred_thousand_scale_are_pinned() {
        // The mapper depends on these offsets and the divisor, so pin them rather
        // than trusting the schema prose. Known fixed-point words are checked
        // against their exact readings at the fold's own divisor
        // (`world_fold.rs:9019`-`:9024`) — the check that would catch a
        // 100000-vs-32768 error.
        let row = supremacy_row([16, 2, 100_000, 150_000, 1, 67_962_800, 250_000, 87_173]);
        check_supremacy_row(&row, 0).expect("legal row");
        let read = |offset: usize| {
            u32::from_le_bytes([
                row[offset],
                row[offset + 1],
                row[offset + 2],
                row[offset + 3],
            ]) as f64
                / FIXED_SCALE
        };
        assert_eq!(read(supremacy_offset::CURRENT), 1.0);
        assert_eq!(read(supremacy_offset::TARGET), 1.5);
        assert_eq!(read(supremacy_offset::BASE_TARGET), 1.0 / 100_000.0);
        assert_eq!(read(supremacy_offset::PREVIOUS), 679.628);
        assert_eq!(read(supremacy_offset::DECLINE_FROM), 2.5);
        assert_eq!(read(supremacy_offset::INDIVIDUAL_RATIO), 0.87173);
        // And the two id words, which the validator does read.
        assert_eq!(u32_at(&row, supremacy_offset::REGION_ID), 16);
        assert_eq!(u32_at(&row, supremacy_offset::CONTROLLER_TAG_DB_ID), 2);
    }

    #[test]
    fn every_header_word_reads_its_own_offset() {
        // A transposed header word mis-slices every row behind it, and no
        // round-trip or replay assertion can see that (d31's rule).
        let mut p = Vec::new();
        p.extend_from_slice(&11u64.to_le_bytes());
        p.extend_from_slice(&2u32.to_le_bytes()); // row_count
        p.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        p.extend_from_slice(&STATUS_DELTA_DATA.to_le_bytes());
        p.extend_from_slice(&2u32.to_le_bytes()); // observed_task_forces
        p.extend_from_slice(&2u32.to_le_bytes()); // validated_location_rows
        p.extend_from_slice(&1u32.to_le_bytes()); // derived_region_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // fixture_required_rows
        p.extend_from_slice(&1u32.to_le_bytes()); // convoy_sunk_count_rows
        p.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        p.extend_from_slice(&full_row());
        p.extend_from_slice(&RawRow {
            task_force_id: 618,
            ..RawRow::default()
        }
        .bytes());
        p.extend_from_slice(&TRAILER_MAGIC);
        p.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        p.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(&full_supremacy_row());

        let split = split_payload(&p).expect("well-formed");
        assert_eq!(
            split.header,
            PayloadHeader {
                elapsed_wall_ms: 11,
                row_count: 2,
                changed_rows: 1,
                status_flags: STATUS_DELTA_DATA,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 0,
                convoy_sunk_count_rows: 1,
                schema_version: SCHEMA_VERSION,
                supremacy_row_count: 1,
            }
        );
        assert!(split.header.is_delta());
        assert_eq!(split.rows.len(), 2 * ROW_LEN);
        assert_eq!(split.supremacy_rows.len(), TRAILER_ROW_LEN);
        assert_eq!(split.rows[..ROW_LEN], full_row()[..]);
        assert_eq!(split.supremacy_rows, &full_supremacy_row()[..]);
    }

    // --- Step 8.4: quantization ------------------------------------------------

    #[test]
    fn the_task_force_collection_has_no_continuous_field_so_nothing_is_quantized() {
        // Step 8.4's byte-saving test does not apply to the task-force table: it
        // table declares no continuous field at all — every one of its words is an
        // id, an enum, a count or a flag. The domain's six continuous channels are
        // all in the supremacy occurrence collection; its `SUPREMACY_QUANT` is
        // proved lossless by `fixed_five_is_lossless_over_every_u32_raw` below.
        let c = task_force_collection();
        assert_eq!(
            c.value_fields()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
        assert!(c.value_fields().all(|f| !f.quant.is_quantized()));
        assert_eq!(SUPREMACY_QUANT, Quant::Fixed(5));
    }

    #[test]
    fn fixed_five_is_lossless_over_every_u32_raw() {
        // `Fixed(5)` is chosen because `10^5` is the source's own denominator, so
        // the claim to prove is `quantize(raw / 1e5) == raw` with no escape, over
        // the whole u32 range the decoder admits for the five point words — not
        // "the error is small".
        let q = Quant::Fixed(SCALE_DIGITS);
        let check = |raw: u32| {
            let v = raw as f64 / FIXED_SCALE;
            assert!(!escapes(v, q), "raw {raw} escaped fixed(5)");
            assert_eq!(quantize(v, q), Some(raw as i64), "raw {raw} did not survive");
        };
        for raw in [
            0u32,
            1,
            FIXED_SCALE_RAW,
            FIXED_SCALE_RAW - 1,
            935_064_683, // the capture's maximum
            u32::MAX,
            u32::MAX - 1,
        ] {
            check(raw);
        }
        // Exhaustive across the unit interval, where rounding ties live, and the
        // whole range of the bounded ratio word.
        for raw in 0..=FIXED_SCALE_RAW {
            check(raw);
        }
        // A stride over the whole u32 range, coprime with 10^5 so the samples do
        // not all land on the same residue.
        let mut raw: u64 = 0;
        while raw <= u32::MAX as u64 {
            check(raw as u32);
            raw += 10_007;
        }
    }

    #[test]
    fn no_field_is_derived() {
        for c in schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    #[test]
    fn no_field_is_a_content_id_key() {
        // The §13 retrofit sweep needs "checked, none" to be distinguishable from
        // "not checked". d56 HAS ContentId values (five of them across the two
        // collections) but no ContentId key, which is the lighter retrofit class.
        for c in schema().collections {
            for f in c.key_fields() {
                assert_ne!(f.kind, Kind::ContentId, "field {} is a ContentId key", f.field_id);
            }
        }
        let content_ids: usize = schema()
            .collections
            .iter()
            .map(|c| c.value_fields().filter(|f| f.kind == Kind::ContentId).count())
            .sum();
        // Five in the task-force collection and two in naval supremacy.
        assert_eq!(content_ids, 7, "the ContentId values the retrofit must sweep");
    }

    // --- Refusals: everything the decoder refuses ------------------------------

    #[test]
    fn the_mapper_rejects_every_row_the_decoder_rejects() {
        let c = task_force_collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1] {
            let row = vec![0u8; len];
            assert!(record_from_task_force_row(&c, &row, 0).is_err(), "length {len}");
            assert!(key_from_task_force_row(&row).is_err(), "length {len}");
        }
        // Invalid oob ids: zero and past the maximum, on both id words.
        for id in [0u32, MAX_OOB_ID + 1] {
            for raw in [
                RawRow {
                    task_force_id: id,
                    ..RawRow::default()
                },
                RawRow {
                    fleet_id: id,
                    ..RawRow::default()
                },
            ] {
                assert!(record_from_task_force_row(&c, &raw.bytes(), 0).is_err());
            }
            // And the bound itself is accepted, so it is not off by one.
            let ok = RawRow {
                task_force_id: MAX_OOB_ID,
                fleet_id: MAX_OOB_ID,
                ..RawRow::default()
            };
            assert!(record_from_task_force_row(&c, &ok.bytes(), 0).is_ok());
        }
        // owner_tag_db_id past its maximum, and the maximum itself accepted.
        assert!(record_from_task_force_row(
            &c,
            &RawRow {
                owner_tag_db_id: MAX_OWNER_TAG_DB_ID + 1,
                ..RawRow::default()
            }
            .bytes(),
            0
        )
        .is_err());
        assert!(record_from_task_force_row(
            &c,
            &RawRow {
                owner_tag_db_id: MAX_OWNER_TAG_DB_ID,
                ..RawRow::default()
            }
            .bytes(),
            0
        )
        .is_ok());
        // Every unknown flag bit, and every retired one — bit by bit, not one
        // sample. Bits 15..=31 are unknown; 6, 7, 8, 12, 13, 14 are retired; the
        // rest are live and must map when their word is packed legally.
        for bit in 0..32u32 {
            let flag = 1u32 << bit;
            let mut raw = RawRow {
                flags: FLAG_VALIDATED_DIRECT | flag,
                ..RawRow::default()
            };
            // A live max-validated gate requires a non-zero word; the two enum
            // gates accept the zero the default row already carries.
            match flag {
                FLAG_HAS_LOCATION => raw.location_province_id = 12_389,
                FLAG_HAS_REGION => raw.strategic_region_id = 31,
                FLAG_HAS_HOME_BASE => raw.home_base_province_id = 9_001,
                FLAG_HAS_REPAIR_BASE => raw.repair_base_province_id = 9_002,
                FLAG_HAS_SUNK_CONVOY_COUNT => raw.sunk_convoys = 2,
                _ => {}
            }
            let mapped = record_from_task_force_row(&c, &raw.bytes(), 0);
            if flag & LIVE_ROW_FLAGS != 0 {
                assert!(mapped.is_ok(), "live flag bit {bit} must map");
            } else {
                assert!(mapped.is_err(), "flag bit {bit} must be rejected");
            }
        }
        // Out-of-range present values, each at max + 1, and the maximum accepted.
        let cases: [(u32, fn(&mut RawRow, u32), u32); 5] = [
            (FLAG_HAS_MISSION, |r, v| r.mission_enum = v, MAX_MISSION_ENUM),
            (
                FLAG_HAS_MISSION_STATE,
                |r, v| r.mission_state = v,
                MAX_MISSION_STATE,
            ),
            (
                FLAG_HAS_LOCATION,
                |r, v| r.location_province_id = v,
                MAX_PROVINCE_ID,
            ),
            (
                FLAG_HAS_REGION,
                |r, v| r.strategic_region_id = v,
                MAX_REGION_ID,
            ),
            (
                FLAG_HAS_SUNK_CONVOY_COUNT,
                |r, v| r.sunk_convoys = v,
                MAX_SUNK_CONVOYS,
            ),
        ];
        for (flag, set, max) in cases {
            let mut over = RawRow {
                flags: FLAG_VALIDATED_DIRECT | flag,
                ..RawRow::default()
            };
            set(&mut over, max + 1);
            assert!(
                record_from_task_force_row(&c, &over.bytes(), 0).is_err(),
                "flag {flag:#x} value {} exceeds {max}",
                max + 1
            );
            let mut at = RawRow {
                flags: FLAG_VALIDATED_DIRECT | flag,
                ..RawRow::default()
            };
            set(&mut at, max);
            assert!(record_from_task_force_row(&c, &at.bytes(), 0).is_ok());
            // A gated-off word carrying a value is a malformed row, not data.
            let mut without = RawRow {
                flags: FLAG_VALIDATED_DIRECT,
                ..RawRow::default()
            };
            set(&mut without, 1);
            assert!(
                record_from_task_force_row(&c, &without.bytes(), 0).is_err(),
                "flag {flag:#x} word set without its gate"
            );
        }
        // A present id word of zero, where zero is NOT a member (the four ids).
        for flag in [
            FLAG_HAS_LOCATION,
            FLAG_HAS_REGION,
            FLAG_HAS_HOME_BASE,
            FLAG_HAS_REPAIR_BASE,
            FLAG_HAS_SUNK_CONVOY_COUNT,
        ] {
            let raw = RawRow {
                flags: FLAG_VALIDATED_DIRECT | flag,
                ..RawRow::default()
            };
            assert!(
                record_from_task_force_row(&c, &raw.bytes(), 0).is_err(),
                "flag {flag:#x} set with a zero word"
            );
        }
        // The three retired words and reserved0: non-zero is a rejected row, which
        // is what makes them constants of the grammar rather than fields.
        for raw in [
            RawRow {
                repair_progress_milli: 1,
                ..RawRow::default()
            },
            RawRow {
                retreat_state: 1,
                ..RawRow::default()
            },
            RawRow {
                convoy_route_context_id: 1,
                ..RawRow::default()
            },
            RawRow {
                reserved0: 1,
                ..RawRow::default()
            },
        ] {
            assert!(record_from_task_force_row(&c, &raw.bytes(), 0).is_err());
        }
        // source_status: every bit outside 8..=10 is refused, and each of those
        // three alone is accepted.
        for bit in 0..32u32 {
            let raw = RawRow {
                source_status: 1u32 << bit,
                ..RawRow::default()
            };
            let mapped = record_from_task_force_row(&c, &raw.bytes(), 0);
            assert_eq!(
                mapped.is_ok(),
                (1u32 << bit) & LIVE_SOURCE_STATUS_MASK != 0,
                "source_status bit {bit}"
            );
        }
    }

    #[test]
    fn the_supremacy_validator_rejects_every_row_the_decoder_rejects() {
        // The fold decodes the trailer before it folds anything: a row this accepts
        // that the decoder rejects would let the harness measure a payload
        // production refuses.
        for len in [0usize, 1, TRAILER_ROW_LEN - 1, TRAILER_ROW_LEN + 1] {
            assert!(check_supremacy_row(&vec![0u8; len], 0).is_err(), "length {len}");
        }
        // Zero and out-of-range ids, and a ratio past 1.0.
        for row in [
            supremacy_row([0, 1, 0, 0, 0, 0, 0, 0]),
            supremacy_row([MAX_REGION_ID + 1, 1, 0, 0, 0, 0, 0, 0]),
            supremacy_row([1, 0, 0, 0, 0, 0, 0, 0]),
            supremacy_row([1, MAX_OWNER_TAG_DB_ID + 1, 0, 0, 0, 0, 0, 0]),
            supremacy_row([1, 1, 0, 0, 0, 0, 0, FIXED_SCALE_RAW + 1]),
        ] {
            assert!(check_supremacy_row(&row, 0).is_err());
        }
        // The bounds themselves are legal, and the five point words are unbounded
        // u32 — only the ratio is capped.
        check_supremacy_row(
            &supremacy_row([
                MAX_REGION_ID,
                MAX_OWNER_TAG_DB_ID,
                u32::MAX,
                u32::MAX,
                u32::MAX,
                u32::MAX,
                u32::MAX,
                FIXED_SCALE_RAW,
            ]),
            0,
        )
        .expect("the bounds themselves are legal");
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over a collection with no value fields — or one whose
        // ids are another domain's — must not get a silently empty or placeholder
        // record under this domain's field ids.
        let tf = task_force_collection();
        let mut empty = tf.clone();
        empty.collection_id = 99;
        empty.fields.clear();
        assert!(record_from_task_force_row(&empty, &full_row(), 0).is_err());
        let mut unknown_field = tf.clone();
        unknown_field.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_task_force_row(&unknown_field, &full_row(), 0).is_err());
    }

    // --- split_payload --------------------------------------------------------

    #[test]
    fn split_payload_enforces_the_decoders_structural_rules() {
        let good = payload(0, 1, &[full_row()], &[full_supremacy_row()]);
        assert!(split_payload(&good).is_ok());
        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // A body shorter than row_count * ROW_LEN.
        assert!(split_payload(&good[..HEADER_LEN + ROW_LEN - 1]).is_err());
        // Wrong schema version, in both directions.
        for version in [0u32, 2, u32::MAX] {
            let mut p = good.clone();
            p[40..44].copy_from_slice(&version.to_le_bytes());
            assert!(split_payload(&p).is_err(), "schema version {version}");
        }
        // changed_rows > row_count, and changed_rows < row_count accepted: d56 is
        // NOT a `changed_rows == row_count` grammar.
        let mut over = good.clone();
        over[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&over).is_err());
        let mut under = good.clone();
        under[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&under).is_ok(), "a partial changed_rows is legal");
        // fixture_required_rows must be zero.
        let mut fixtures = good.clone();
        fixtures[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&fixtures).is_err());
        // Retired fixture status bits in the header.
        for bit in [16u32, 17, 18, 19] {
            let mut p = good.clone();
            p[16..20].copy_from_slice(&(1u32 << bit).to_le_bytes());
            assert!(split_payload(&p).is_err(), "retired status bit {bit}");
        }
        // Header counters above row_count.
        for offset in [24usize, 28] {
            let mut p = good.clone();
            p[offset..offset + 4].copy_from_slice(&2u32.to_le_bytes());
            assert!(split_payload(&p).is_err(), "counter at {offset}");
        }
        // A row_count whose byte length overflows `usize`.
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
        // No trailer at all is legal — that is how an empty sidecar is written.
        let bare = payload(0, 1, &[full_row()], &[]);
        let split = split_payload(&bare).expect("no trailer is legal");
        assert_eq!(split.header.supremacy_row_count, 0);
        assert!(split.supremacy_rows.is_empty());
    }

    #[test]
    fn split_payload_enforces_the_trailer_walk() {
        let base = payload(0, 1, &[full_row()], &[]);
        // Unknown magic, at every length a stub tail can have.
        for tail in [
            vec![0u8],
            vec![0u8; 3],
            b"XSUP".to_vec(),
            b"NSU".to_vec(),
        ] {
            let mut p = base.clone();
            p.extend_from_slice(&tail);
            assert!(split_payload(&p).is_err(), "tail {tail:?}");
        }
        // A truncated NSUP header.
        for len in 4..TRAILER_HEADER_LEN {
            let mut p = base.clone();
            let mut trailer = Vec::new();
            trailer.extend_from_slice(&TRAILER_MAGIC);
            trailer.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
            trailer.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
            trailer.extend_from_slice(&0u32.to_le_bytes());
            p.extend_from_slice(&trailer[..len]);
            assert!(split_payload(&p).is_err(), "trailer header len {len}");
        }
        // Wrong trailer version or header length.
        for (version, header_len) in [(0u16, 12u16), (2, 12), (1, 8), (1, 16)] {
            let mut p = base.clone();
            p.extend_from_slice(&TRAILER_MAGIC);
            p.extend_from_slice(&version.to_le_bytes());
            p.extend_from_slice(&header_len.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            assert!(
                split_payload(&p).is_err(),
                "trailer version {version} header_len {header_len}"
            );
        }
        // Row count past the cap, and a body shorter than the declared rows.
        let mut over = base.clone();
        over.extend_from_slice(&TRAILER_MAGIC);
        over.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        over.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        over.extend_from_slice(&((MAX_NAVAL_SUPREMACY_ROWS + 1) as u32).to_le_bytes());
        assert!(split_payload(&over).is_err());
        let mut short = base.clone();
        short.extend_from_slice(&TRAILER_MAGIC);
        short.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        short.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        short.extend_from_slice(&2u32.to_le_bytes());
        short.extend_from_slice(&full_supremacy_row());
        assert!(split_payload(&short).is_err());
        // A duplicate trailer, and trailing bytes after a complete one.
        let one = payload(0, 1, &[full_row()], &[full_supremacy_row()]);
        let mut duplicate = one.clone();
        duplicate.extend_from_slice(&TRAILER_MAGIC);
        duplicate.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        duplicate.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        duplicate.extend_from_slice(&0u32.to_le_bytes());
        let err = split_payload(&duplicate).expect_err("duplicate trailer");
        assert!(err.contains("duplicate"), "{err}");
        let mut trailing = one.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err());
        // The cap itself is legal as a declared count (with the rows to match it
        // this would be a 640 KiB payload, so only the count is exercised here).
        let mut at_cap = base.clone();
        at_cap.extend_from_slice(&TRAILER_MAGIC);
        at_cap.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        at_cap.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        at_cap.extend_from_slice(&(MAX_NAVAL_SUPREMACY_ROWS as u32).to_le_bytes());
        let err = split_payload(&at_cap).expect_err("no rows behind the count");
        assert!(err.contains("trailer len"), "{err}");
    }

    // --- fold_payload ---------------------------------------------------------

    #[test]
    fn the_fold_full_replaces_task_forces_and_the_sidecar() {
        let schema = schema();
        let mut state = State::new();
        let first = payload(
            0,
            1,
            &[full_row()],
            &[full_supremacy_row(), supremacy_row([2, 3, 1, 1, 1, 1, 1, 1])],
        );
        assert_eq!(
            state
                .fold_payload(&schema, &first, false, 0)
                .expect("folds"),
            FoldOutcome::Folded
        );
        assert_eq!(state.task_forces.len(), 1);
        assert_eq!(state.naval_supremacy.len(), 2);
        assert!(state.baseline_seen);

        // A non-delta frame clears stale rows (`world_fold.rs:22142`).
        let second = payload(
            0,
            1,
            &[RawRow {
                task_force_id: 900,
                ..RawRow::default()
            }
            .bytes()],
            &[],
        );
        state
            .fold_payload(&schema, &second, false, 0)
            .expect("folds");
        assert_eq!(state.task_forces.len(), 1);
        assert_eq!(
            state.task_forces.keys().next(),
            Some(&key(&[900])),
            "the previous task force is gone"
        );
        assert_eq!(
            state.naval_supremacy.len(), 0,
            "the sidecar is full-replaced even by an empty trailer"
        );

        // A delta frame upserts and does NOT clear (no tombstone contract), and
        // still full-replaces the sidecar (`:22193`-`:22203`).
        let third = payload(
            STATUS_DELTA_DATA,
            1,
            &[RawRow {
                task_force_id: 901,
                ..RawRow::default()
            }
            .bytes()],
            &[full_supremacy_row()],
        );
        state
            .fold_payload(&schema, &third, false, 0)
            .expect("folds");
        assert_eq!(state.task_forces.len(), 2, "a delta frame never clears");
        assert_eq!(state.naval_supremacy.len(), 1);
    }

    #[test]
    fn the_fold_excludes_a_failure_hour_and_a_delta_before_baseline() {
        let schema = schema();
        // Failure status: the accumulator is retained, nothing folded.
        let mut state = State::new();
        state
            .fold_payload(&schema, &payload(0, 1, &[full_row()], &[]), false, 0)
            .expect("folds");
        let before = state.task_forces.clone();
        assert_eq!(
            state
                .fold_payload(
                    &schema,
                    &payload(
                        0,
                        1,
                        &[RawRow {
                            task_force_id: 900,
                            ..RawRow::default()
                        }
                        .bytes()],
                        &[],
                    ),
                    true,
                    0,
                )
                .expect("classified"),
            FoldOutcome::FailureStatus
        );
        assert_eq!(state.task_forces, before, "accumulator retained");
        assert!(state.baseline_seen, "a failed hour sets no baseline");

        // Delta before baseline, on a fresh accumulator.
        let mut fresh = State::new();
        assert_eq!(
            fresh
                .fold_payload(
                    &schema,
                    &payload(STATUS_DELTA_DATA, 1, &[full_row()], &[full_supremacy_row()]),
                    false,
                    0,
                )
                .expect("classified"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(fresh.task_forces.is_empty());
        assert_eq!(
            fresh.naval_supremacy.len(), 0,
            "the sidecar is not replaced on a non-folding exit either"
        );
        assert!(!fresh.baseline_seen);
    }

    #[test]
    fn a_malformed_payload_is_an_error_even_under_a_failure_status() {
        // The fold decodes before it classifies (`world_fold.rs:22120`-`:22127`),
        // so a bad payload refuses the stream whatever its status word says.
        let schema = schema();
        let mut state = State::new();
        let mut bad = payload(0, 1, &[full_row()], &[]);
        bad[40..44].copy_from_slice(&2u32.to_le_bytes()); // schema version
        assert!(state.fold_payload(&schema, &bad, true, 0).is_err());
        assert!(state.fold_payload(&schema, &bad, false, 0).is_err());
    }

    #[test]
    fn a_duplicate_task_force_id_refuses_the_frame() {
        // `domain56_compact.rs:269`-`:274`: a rejected frame, never a
        // last-writer-wins overwrite of the first row.
        let schema = schema();
        let mut state = State::new();
        let dup = payload(0, 2, &[full_row(), full_row()], &[]);
        assert!(state.fold_payload(&schema, &dup, false, 0).is_err());
        assert!(state.task_forces.is_empty(), "nothing was written");
    }

    #[test]
    fn duplicate_supremacy_occurrences_are_retained_in_source_order() {
        // Three rows, two of them byte-identical: the fold keeps all three in its
        // `Vec` (`world_fold.rs:22595`-`:22609`), and the decoder says a
        // byte-identical repeat "can legitimately occur for zero/steady-state
        // entries" (`domain56_compact.rs:446`-`:449`). Any map keyed by the row's
        // own words would hold one. The count this port tracks holds three, which
        // §7.3's shape retains the rows themselves, not merely their count.
        let schema = schema();
        let rows = [
            supremacy_row([9, 1, 5, 0, 0, 0, 0, 0]),
            supremacy_row([2, 7, 6, 0, 0, 0, 0, 0]),
            supremacy_row([9, 1, 5, 0, 0, 0, 0, 0]),
        ];
        let mut state = State::new();
        state
            .fold_payload(&schema, &payload(0, 0, &[], &rows), false, 0)
            .expect("folds");
        assert_eq!(state.naval_supremacy.len(), 3, "a repeated row is not collapsed");
        assert_eq!(state.naval_supremacy[0], state.naval_supremacy[2]);
        assert_ne!(state.naval_supremacy[0], state.naval_supremacy[1]);
        assert_eq!(state.states().len(), 2);
    }

    #[test]
    fn the_supremacy_mapper_carries_all_eight_words_on_the_source_grid() {
        let collection = schema().collections[1].clone();
        let row = supremacy_row([16, 2, 100_000, 150_000, 1, 67_962_800, 250_000, 87_173]);
        let mapped = record_from_supremacy_row(&collection, &row, 0).expect("maps");
        assert_eq!(
            mapped.values,
            vec![
                Value::ContentId(16),
                Value::ContentId(2),
                Value::Continuous(1.0),
                Value::Continuous(1.5),
                Value::Continuous(0.000_01),
                Value::Continuous(679.628),
                Value::Continuous(2.5),
                Value::Continuous(0.871_73),
            ]
        );
    }
}
