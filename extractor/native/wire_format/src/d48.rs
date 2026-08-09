//! d48 `construction_buildings_state_damage_live` — the construction queue,
//! per-location building levels/damage, per-state resources, and the RLC1
//! railway-construction trailer.
//!
//! Derived from `world_fold.rs:23754`-`:24091`
//! (`fold_construction_buildings_state_damage`), its accumulator type at
// @world-fold-authority domain=48 functions=fold_construction_buildings_state_damage lines=23754-24091 sha256=9d8154f0e46b7fe29b1894a2a39c562b69563431d16b20dbdb6161fc3d5e869b closure=d4e9a53edf16bf22320e4b1c1b688fb1bc1c55652f3d7cfaa333bcab30f60d1e
//! `:2123`-`:2131` and its key helpers at `:25681`-`:25696`; the row grammar is
//! the lane crate `extractor/native/lanes/construction_buildings_state_damage_live/src/lib.rs`
//! (`decode_lane_frame` at `:1118`, `gate_decoded_frame` at `:1413`, the row
//! readers at `:1583`-`:1633` and the row validators at `:1635`-`:1839`); the
//! collector read that decides every gate is
//! `lanes/construction_buildings_state_damage_live/src/collector_module.rs` and
//! `extractor/native/collector/src/attach_log.rs:41560`-`:41645`.
//! There is no dossier under `extractor/documents/domains/` for d48, which
//! `domains/UNDOCUMENTED_DOMAINS.md` says is expected; the fold and the lane are
//! the whole basis.
//!
//! # Payload grammar
//!
//! `CBSD` v2, a 64-byte header (`lib.rs:12`-`:19`, decoded at `:1118`-`:1168`)
//! followed by three fixed-length row tables and an optional `RLC1` trailer:
//!
//! | offset | width | header word | check |
//! |---|---|---|---|
//! | `+0`  | 4×u8 | magic `CBSD`         | `lib.rs:1122` |
//! | `+4`  | u16  | version `2`          | `:1125` |
//! | `+6`  | u16  | header_len `64`      | `:1128` |
//! | `+8`  | u16  | domain_id `48`       | `:1132` |
//! | `+10` | u16  | reserved, must be 0  | `:1135` |
//! | `+12` | u32  | `status_flags`       | the frame mode and the failure class |
//! | `+16` | i32  | `game_hour_raw`      | read, never used by the fold (`:20374` overwrites it with 0) |
//! | `+20` | u32  | `construction_count` | `:1140` |
//! | `+24` | u32  | `building_count`     | `:1141` |
//! | `+28` | u32  | `resource_count`     | `:1142` |
//! | `+32` | u16  | construction row_len `52` | `:1143` |
//! | `+34` | u16  | building row_len `40`     | `:1144` |
//! | `+36` | u16  | resource row_len `16`     | `:1145` |
//! | `+38` | u16  | reserved, must be 0  | `:1149` |
//! | `+40` | u32  | `observed_country_roots`  | diagnostic only |
//! | `+44` | u32  | `observed_state_roots`    | diagnostic only |
//! | `+48` | u32  | `observed_province_roots` | diagnostic only |
//! | `+52` | u32  | `source_flags`       | `SOURCE_FLAG_*`, `lib.rs:26`-`:30` |
//! | `+56` | u32  | reserved, must be 0  | `:1156` |
//! | `+60` | u32  | reserved, must be 0  | `:1156` |
//!
//! **There is no `changed_rows` word.** The recipe's `changed_rows == row_count`
//! invariant has no counterpart here: the header carries three independent table
//! counts and nothing that the fold could compare them against, so nothing of
//! that class is enforced (`split_payload`).
//!
//! Construction row, 52 bytes (`lib.rs:1517`-`:1552` writes it, `:1583` reads it):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`   | raw | key; non-zero and `<= 2048` (`:1636`-`:1644`) |
//! | `+4`  | u32 | `line_id`     | raw | key; bit 31 is the DELETED tombstone bit (`:153`, `:1845`) |
//! | `+8`  | u32 | `template_id` | raw | `1<<0` (`:129`) |
//! | `+12` | i32 | `active_factories` | raw | none; `0..=100_000` (`:1679`) |
//! | `+16` | i32 | `cost_raw`    | **raw, no scale** | none; `>= 0` (`:1685`) |
//! | `+20` | i32 | `produced_raw`| **raw, no scale** | none; `>= 0` (`:1685`) |
//! | `+24` | u32 | `priority`    | raw | none; the country's production-vector index (`:640`) |
//! | `+28` | u32 | `target_state_id` | raw | `1<<1` (`:130`) |
//! | `+32` | u32 | `flags`       | — | — |
//! | `+36` | u32 | `created_date_raw` | raw hour-count | `1<<2` (`:136`) |
//! | `+40` | u32 | `target_location_id` | raw | `1<<3` (`:137`) |
//! | `+44` | u32 | `target_scope` | — | `1<<4` (`:138`); `1`=state, `2`=province |
//! | `+48` | u32 | `repair_levels_remaining` | raw | `1<<5` (`:139`) |
//!
//! Building row, 40 bytes (`lib.rs:1554`-`:1574`, read at `:1612`):
//! `scope` u32 `+0`, `location_id` u32 `+4`, `state_id` u32 `+8`, `type_id` u32
//! `+12` (bit 31 DELETED), `level` u32 `+16`, `healthy_level` u32 `+20` gated by
//! `1<<0`, `flags` u32 `+24`, reserved u32 `+28` (must be 0), `partial_health_raw`
//! u64 `+32` gated by `1<<2`. `BUILDING_FLAG_DAMAGED` (`1<<1`) is **derived**:
//! the encoder sets it exactly when `healthy_level < level` (`:1556`-`:1560`) and
//! the decoder refuses it without `HEALTHY_PRESENT` (`:1226`), so it is not a
//! field — see "Derived values that are not fields" below.
//!
//! Resource row, 16 bytes (`lib.rs:1576`, read at `:1627`): `state_id` u32 `+0`,
//! `resource_id` u32 `+4` (bit 31 DELETED), `amount_raw` i32 `+8` **raw, no
//! scale**, reserved u32 `+12` (must be 0).
//!
//! `RLC1` trailer (`lib.rs:1079` writes it, `:1277` decodes it): a 24-byte
//! header — magic `+0`, version `1` `+4`, header_len `24` `+6`, `row_count` u32
//! `+8`, `route_count` u32 `+12`, row_len `64` `+16`, reserved u16 `+18`,
//! reserved u32 `+20` — then `row_count` 64-byte rows, then `route_count` u32
//! province ids. Row: `tag_db_id +0`, `line_id +4`, `queue_index +8`,
//! `active_factories i32 +12`, `cost_raw i32 +16`, `produced_raw i32 +20`,
//! `created_date_raw u32 +24`, `base_level u32 +28`, `target_level u32 +32`,
//! `current_segment_index i32 +36`, `remaining_hours_raw i64 +40` (**fixed-5,
//! carried raw**), `route_start u32 +48`, `route_len u32 +52`, `flags u32 +56`
//! (bit 0 = completion), reserved u32 `+60`.
//!
//! # Frame relation: baseline + delta, with a THREE-WAY accumulator seed
//!
//! This is a **fifth shape** beside the recipe's four (step 1), and it is
//! recorded as a recipe gap. `world_fold.rs::fold_construction_buildings_state_damage` seeds the working
//! state from one of three places, chosen by the frame's mode:
//!
//! * **delta frame** (`COMPACT_STATUS_DELTA_DATA` set) — seed from the **delta
//!   baseline** `domain48_delta_state`, *not* the projected state. The collector
//!   computed the delta against its own `tracker.last_*`, which a tolerated
//!   partial never advanced, so applying it onto anything else re-introduces the
//!   partial's transient rows. Seeding a delta from the projected state is the
//!   named defect the fold's ten-line comment exists to prevent.
//! * **tolerated-partial frame** — seed from the projected state
//!   `domain48_state` and sparse-merge, retaining omitted rows.
//! * **clean frame** — replace from empty.
//!
//! The two accumulators then advance differently (`world_fold.rs::fold_construction_buildings_state_damage`): the
//! projected state always takes this frame, while the delta baseline advances
//! only on a clean or delta frame. [`Domain48Fold`] holds both and
//! [`Domain48Fold::apply`] takes [`FrameMode`] — the frame mode **and** the
//! partial flag — as a typed input, so neither is re-derived by a caller.
//!
//! **The partial arm is structurally unreachable for d48 today, and that is
//! producer evidence, not a licence to drop it.** `expected_partial` comes from
//! `compact_contract::expected_partial_tolerance_applies`, whose table lists
//! exactly domains 26, 27, 28 and 54 (`compact_contract/src/lib.rs:136`-`:158`).
//! Domain 48 is not in it, so `COMPACT_STATUS_PARTIAL_DATA` on a d48 record is an
//! ordinary failure status (`COMPACT_STATUS_FAILURE_MASK`, `:110`-`:118`) and the
//! hour is excluded. The arm is implemented and tested here because the fold
//! implements it and the recipe forbids refusing what the fold accepts; the
//! measurement reports it as a path the capture never exercised.
//!
//! # The frame-level source flag that decides one field for every row
//!
//! `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE` (`lib.rs:30`) is a
//! **frame** bit, and `world_fold.rs::fold_construction_buildings_state_damage` lets it decide
//! `target_state_id` for every construction row in the projected state —
//! including rows retained from an earlier frame that carried a real value. This
//! is the d17 `WarSegment` pattern (recipe step 7's typed-input table), second
//! measured instance, so it is a named mapper parameter and never re-derived.
//!
//! The collector sets the bit exactly when at least one construction row's
//! target join failed, and then clears the **whole promoted tuple** —
//! `target_state_id`, `target_location_id`, `target_scope` — on every row
//! (`collector_module.rs:164`-`:206`). So a clear bit means every live row
//! carries the join, and a set bit means no row does. The gate enforces that
//! correspondence within a frame (`lib.rs:1443`-`:1466`).
//!
//! # Four-state semantics
//!
//! The fold answers value-vs-nothing only: it emits JSON `Null` for
//! `template_id`/`healthy_level` (`option_u32_json`, `world_fold.rs::option_u32_json`) and
//! omits the key for `created_date_raw`, `target_location_id`, `target_scope`,
//! `repair_levels_remaining` and `partial_health_raw`. §3's three-way choice is
//! made here, per field, from the collector read:
//!
//! * `template_id` — `None` when the line's construction sub-object pointer is
//!   not a heap pointer, or the id read fails, or the id is out of range
//!   (`lib.rs:615`-`:620`). Two of those three are unreadable memory and one is a
//!   missing relation, and nothing in the payload separates them → the recipe's
//!   **tie-break: Unknown**, recorded in the open questions.
//! * `target_state_id` — **Unknown** when the frame bit is set (the join was not
//!   captured this frame) or the row carries no value. A row can only carry no
//!   value because the frame that admitted it was itself badged, so both cases
//!   are §3's "applicable, not captured".
//! * `target_location_id`, `target_scope` — **Unknown** when absent, for the same
//!   reason: they are the other two thirds of the one promoted tuple the
//!   collector clears together. Note the fold gates these two on the row's own
//!   option only, **not** on the frame bit (`world_fold.rs::fold_construction_buildings_state_damage`), so a
//!   badged frame still projects them for a retained row; this mapper mirrors
//!   that asymmetry exactly and reports it as a fold observation.
//! * `repair_levels_remaining` — **Unknown** when absent: the read failed or the
//!   value was out of range (`lib.rs:625`-`:627`). A line that is not a repair
//!   reads `0` with the flag **set** — a real Zero, never an empty state.
//! * `created_date_raw` — **Unknown** when absent: a torn or implausible raw
//!   hour-count is badged absent rather than emitted as a fabricated 0
//!   (`lib.rs:628`-`:632`, `world_fold.rs::fold_construction_buildings_state_damage` says so in prose).
//! * `healthy_level` — **Unknown** when absent: at least one `CBuilding` at this
//!   location/type had an unreadable or out-of-range healthy word, which makes
//!   the summed level incomplete (`lib.rs:942`-`:944`, `:1874`-`:1895`).
//! * `partial_health_raw` — **Unknown** when absent, for the same accumulator
//!   reason plus a rejected inconsistent reading (`lib.rs:945`-`:948`).
//! * `current_segment_index` (RLC1) — **Null** at `-1`. That is not a failed
//!   read: it is the engine's explicit no-current-edge sentinel, documented as
//!   such at `lib.rs:315`-`:317`, so the relation genuinely points at nothing.
//!   The mapping is bijective over the values the decoder admits
//!   (`-1..route_len-1`, `:1354`-`:1355`), so nothing is lost.
//! * Everything else is a real value, including a real Zero.
//!
//! **No field here is `Absent`.** Every one of these concepts applies to every
//! row of its collection; there is no row form and no kind discriminator that
//! makes one inapplicable, so nothing declares `.optional()`.
//!
//! # Derived values that are not fields
//!
//! Per the recipe's step-2 rule, a value the fold hard-enforces as a function of
//! carried state stays off the wire and is enforced in the mapper instead:
//!
//! * `damaged` (`domain48_building_rows_by_type`, `world_fold.rs::domain48_building_rows_by_type`)
//!   `== healthy_level < level`.
//! * `is_repair` (`:20424`) `== repair_levels_remaining > 0`.
//! * `remaining_hours` (`:20479`) `== remaining_hours_raw / 100000.0`.
//! * `token` (`world_fold.rs::domain48_building_rows_by_type`) is a building-registry lookup on `type_id`.
//! * RLC1 `route_len` is the cardinality of this row's [`RAILWAY_ROUTE`] rows.
//!
//! # No continuous field, and therefore no quantization
//!
//! Every measured quantity in d48 is a raw fixed-point **integer** that the fold
//! emits verbatim (`cost_raw`, `produced_raw`, `amount_raw`, `partial_health_raw`,
//! `remaining_hours_raw`, `created_date_raw`) and renders derived floats from.
//! Carrying the integer is exact for every value the decoder admits, needs no
//! precision band, and keeps the raw word downstream consumers read. Recipe step
//! 4 governs `Continuous` fields; d48 declares none. In particular
//! `remaining_hours_raw` is an `i64` over a 100000 denominator whose value range
//! is only bounded below (`lib.rs:1356`), so a `Fixed(5)` continuous encoding
//! would have carried d36's `2^51` reproduction band for no gain.
//!
//! # No derived field (§6)
//!
//! Nothing here changes deterministically with the clock. `created_date_raw` is a
//! *fixed* creation stamp, not a countdown, and `remaining_hours_raw` moves with
//! construction progress rather than with the tick. The mappers still take
//! `frame_tick` so every domain's boundary has one shape.

use std::collections::BTreeMap;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 48;

pub const CONSTRUCTION: u32 = 1;
pub const BUILDINGS: u32 = 2;
pub const STATE_RESOURCES: u32 = 3;
pub const RAILWAY_CONSTRUCTION: u32 = 4;
pub const RAILWAY_ROUTE: u32 = 5;

pub const COLLECTION_COUNT: usize = 5;
pub const CONSTRUCTION_INDEX: usize = 0;
pub const BUILDINGS_INDEX: usize = 1;
pub const STATE_RESOURCES_INDEX: usize = 2;
pub const RAILWAY_CONSTRUCTION_INDEX: usize = 3;
pub const RAILWAY_ROUTE_INDEX: usize = 4;

/// Field ids are stable and append-only; §11 forbids reusing an id for a
/// different field. One concept keeps one id across collections (d26's and
/// d83's convention), which is why the railway table reuses `TAG_DB_ID`,
/// `LINE_ID`, `ACTIVE_FACTORIES`, `COST_RAW`, `PRODUCED_RAW` and
/// `CREATED_DATE_RAW`.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const LINE_ID: u32 = 2;
    pub const PRIORITY: u32 = 3;
    pub const TEMPLATE_ID: u32 = 4;
    pub const ACTIVE_FACTORIES: u32 = 5;
    pub const COST_RAW: u32 = 6;
    pub const PRODUCED_RAW: u32 = 7;
    pub const TARGET_STATE_ID: u32 = 8;
    pub const TARGET_LOCATION_ID: u32 = 9;
    pub const TARGET_SCOPE: u32 = 10;
    pub const REPAIR_LEVELS_REMAINING: u32 = 11;
    pub const CREATED_DATE_RAW: u32 = 12;
    pub const SCOPE: u32 = 13;
    pub const LOCATION_ID: u32 = 14;
    pub const STATE_ID: u32 = 15;
    pub const TYPE_ID: u32 = 16;
    pub const LEVEL: u32 = 17;
    pub const HEALTHY_LEVEL: u32 = 18;
    pub const PARTIAL_HEALTH_RAW: u32 = 19;
    pub const RESOURCE_ID: u32 = 20;
    pub const AMOUNT_RAW: u32 = 21;
    pub const QUEUE_INDEX: u32 = 22;
    pub const BASE_LEVEL: u32 = 23;
    pub const TARGET_LEVEL: u32 = 24;
    pub const COMPLETION_FLAG: u32 = 25;
    pub const CURRENT_SEGMENT_INDEX: u32 = 26;
    pub const REMAINING_HOURS_RAW: u32 = 27;
    pub const ROUTE_ORDINAL: u32 = 28;
    pub const ROUTE_PROVINCE_ID: u32 = 29;
}

/// `target_scope` codes. §4 rejects a code at or above the cardinality, so the
/// wire uses a dense `0..2` rather than the payload's `1`/`2` words.
pub mod scope_code {
    pub const STATE: u32 = 0;
    pub const PROVINCE: u32 = 1;
    pub const CARDINALITY: u32 = 2;
}

// --- Payload constants, from the lane crate ----------------------------------
//
// d48 is not admitted to the production-codec dependency surface and the
// library may not depend on its capture-lane crate, so the constants the fold's
// checks are written against are restated here with their source line and the
// measurement bin asserts each one against `compact_contract::build_profile`'s
// own value, so neither copy can drift unnoticed.

/// `lib.rs:10`-`:11`.
pub const MAGIC: [u8; 4] = *b"CBSD";
pub const VERSION: u16 = 2;
/// `lib.rs:12`-`:15`.
pub const HEADER_LEN: usize = 64;
pub const CONSTRUCTION_ROW_LEN: usize = 52;
pub const BUILDING_ROW_LEN: usize = 40;
pub const RESOURCE_ROW_LEN: usize = 16;
/// `lib.rs:16`-`:19`.
pub const RAILWAY_TRAILER_MAGIC: [u8; 4] = *b"RLC1";
pub const RAILWAY_TRAILER_VERSION: u16 = 1;
pub const RAILWAY_TRAILER_HEADER_LEN: usize = 24;
pub const RAILWAY_TRAILER_ROW_LEN: usize = 64;

/// `lib.rs:26`-`:30`.
pub const SOURCE_FLAG_DLL_NATIVE: u32 = 1;
pub const SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE: u32 = 1 << 4;

/// `lib.rs:129`-`:147`. An unknown bit in a construction row's flag word is a
/// forward-incompatible payload and fails closed (`:1176`).
pub const CONSTRUCTION_FLAG_TEMPLATE_PRESENT: u32 = 1;
pub const CONSTRUCTION_FLAG_TARGET_STATE_PRESENT: u32 = 1 << 1;
pub const CONSTRUCTION_FLAG_CREATED_DATE_PRESENT: u32 = 1 << 2;
pub const CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT: u32 = 1 << 3;
pub const CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT: u32 = 1 << 4;
pub const CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT: u32 = 1 << 5;
pub const CONSTRUCTION_ROW_KNOWN_FLAGS: u32 = CONSTRUCTION_FLAG_TEMPLATE_PRESENT
    | CONSTRUCTION_FLAG_TARGET_STATE_PRESENT
    | CONSTRUCTION_FLAG_CREATED_DATE_PRESENT
    | CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT
    | CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT
    | CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT;

/// `lib.rs:148`-`:152`. `DAMAGED` is derived by the encoder, never a field.
pub const BUILDING_FLAG_HEALTHY_PRESENT: u32 = 1;
pub const BUILDING_FLAG_DAMAGED: u32 = 1 << 1;
pub const BUILDING_FLAG_PARTIAL_HEALTH_PRESENT: u32 = 1 << 2;
pub const BUILDING_ROW_KNOWN_FLAGS: u32 =
    BUILDING_FLAG_HEALTHY_PRESENT | BUILDING_FLAG_DAMAGED | BUILDING_FLAG_PARTIAL_HEALTH_PRESENT;

/// `lib.rs:153` — deletion rides in the identity word's top bit, not a flag word.
pub const ROW_ID_DELETED: u32 = 1 << 31;

/// `build_profile.rs:1175`-`:1176`, restated per the note above.
pub const SCOPE_STATE: u32 = 1;
pub const SCOPE_PROVINCE: u32 = 2;
/// `build_profile.rs:1177`-`:1178`.
pub const MAX_STATE_ID: u32 = 65_535;
pub const MAX_PROVINCE_ID: u32 = 65_535;
/// `build_profile.rs:45`.
pub const MAX_COMPACT_OWNER_TAG_DB_ID: u32 = 2_048;
/// `build_profile.rs:1137`.
pub const BUILDING_LEVEL_SANE_MAX: u32 = 1_000;
/// `build_profile.rs:1170`.
pub const MAX_CONSTRUCTION_TEMPLATE_ID: u32 = 10_000_000;
/// `build_profile.rs:1173`.
pub const MAX_ACTIVE_FACTORIES: i32 = 100_000;
/// `build_profile.rs:1120`.
pub const STATE_RESOURCE_MAX_ABS_RAW: i32 = 100_000_000;
/// `build_profile.rs:1118`.
pub const STATE_RESOURCE_NO_ID: u32 = u32::MAX;
/// `build_profile.rs:1135`.
pub const CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL: u64 = 10_000_000;
// These last two are the lane's own constants rather than `build_profile`'s, so
// the measurement bin's drift guard cannot machine-check them: `wire_format`
// depends on `compact_contract` and deliberately not on the lane crate. They are
// therefore hand-checked against their source and cited exactly.
/// `lib.rs:93` — the fail-closed ceiling on a `created_date` raw hour-count.
pub const MAX_CONSTRUCTION_CREATED_DATE_RAW: u32 = 100_000_000;
/// `lib.rs:110`.
pub const MAX_RAILWAY_ROUTE_PROVINCES: u32 = 4_096;

pub fn schema() -> Schema {
    use field as f;

    // --- Construction queue, keyed as the fold keys it (`world_fold.rs::domain48_construction_key`:
    // `(tag_db_id, line_id & !ROW_ID_DELETED)`).
    let construction = Collection {
        collection_id: CONSTRUCTION,
        shape: Shape::KeyedSet,
        fields: vec![
            // A country tag is content-defined (§5): a manifest must resolve it
            // for it to keep its meaning under another mod set. d63's key and
            // d20's `owner` are classed the same way. This is a ContentId KEY —
            // flagged separately in the open questions, because a manifest remap
            // reorders keys and rewrites every baseline's byte layout.
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            // A runtime instance identity created during play, not content: the
            // engine's own `CBuildingProductionLine` id (`lib.rs:599`).
            Field::new(f::LINE_ID, Kind::Unsigned).key(1),
            // The country's production-vector index (`lib.rs:640`) and the only
            // order witness the fold's sort reads (`world_fold.rs::fold_construction_buildings_state_damage` sorts
            // by `(tag_db_id, priority, line_id)`). Recipe step 5's first
            // ordinal case: a carried field already ranks the array, so the
            // order is a total function of carried fields and the keyed set
            // stands. Required and concrete.
            Field::new(f::PRIORITY, Kind::Unsigned),
            // A building template is defined by mod content, so a manifest must
            // resolve it: ContentId.
            Field::new(f::TEMPLATE_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::ACTIVE_FACTORIES, Kind::Unsigned),
            // Raw fixed-point words the fold emits verbatim
            // (`world_fold.rs::fold_construction_buildings_state_damage`), never divided.
            Field::new(f::COST_RAW, Kind::Unsigned),
            Field::new(f::PRODUCED_RAW, Kind::Unsigned),
            Field::new(f::TARGET_STATE_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::TARGET_LOCATION_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::TARGET_SCOPE, Kind::Enum)
                .enum_card(scope_code::CARDINALITY)
                .unknown_capable(),
            Field::new(f::REPAIR_LEVELS_REMAINING, Kind::Unsigned).unknown_capable(),
            Field::new(f::CREATED_DATE_RAW, Kind::Unsigned).unknown_capable(),
        ],
    };

    // --- Buildings, keyed as the fold keys them (`world_fold.rs::domain48_building_key`:
    // `(scope, location_id, state_id, type_id & !ROW_ID_DELETED)`).
    let buildings = Collection {
        collection_id: BUILDINGS,
        shape: Shape::KeyedSet,
        fields: vec![
            // A key component may only be Unsigned/Signed/ContentId
            // (`schema.rs:29`-`:31`), so the scope discriminator rides as its
            // raw engine word rather than as the Enum the construction table's
            // `target_scope` uses. It is an engine constant, not content.
            Field::new(f::SCOPE, Kind::Unsigned).key(0),
            // State id or province id depending on `scope` — content either way.
            Field::new(f::LOCATION_ID, Kind::ContentId).key(1),
            Field::new(f::STATE_ID, Kind::ContentId).key(2),
            // A building type is defined by mod content. ContentId KEY.
            Field::new(f::TYPE_ID, Kind::ContentId).key(3),
            Field::new(f::LEVEL, Kind::Unsigned),
            Field::new(f::HEALTHY_LEVEL, Kind::Unsigned).unknown_capable(),
            Field::new(f::PARTIAL_HEALTH_RAW, Kind::Unsigned).unknown_capable(),
        ],
    };

    // --- State resources, keyed as the fold keys them (`world_fold.rs::domain48_resource_identity_key`).
    let resources = Collection {
        collection_id: STATE_RESOURCES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            // A resource is defined by mod content. ContentId KEY.
            Field::new(f::RESOURCE_ID, Kind::ContentId).key(1),
            // Raw, emitted verbatim by the fold (`world_fold.rs::fold_construction_buildings_state_damage`).
            Field::new(f::AMOUNT_RAW, Kind::Unsigned),
        ],
    };

    // --- RLC1 railway construction. The fold full-replaces this table on every
    // non-partial frame (`world_fold.rs::fold_construction_buildings_state_damage`); full replacement is
    // recipe shape 1 and a keyed set carries it, because the identity persists.
    //
    // KEY DECISION, recorded with its evidence (recipe step 5). The trailer
    // enforces a strictly increasing `(tag_db_id, queue_index, line_id)`
    // (`lib.rs:1360`-`:1364`), which is an *ordering* key, not the row's
    // identity: `queue_index` is the country's production-vector position
    // (`lib.rs:688`, `:761`) and shifts whenever a line ahead of it completes.
    // Keying by it would churn every railway row of every such hour and would
    // contradict step 5's requirement that a key persist across hours as the
    // same thing. `line_id` is the engine's own line id, read from the line
    // object and required non-zero (`lib.rs:753`-`:757`), so the identity is
    // `(tag_db_id, line_id)` and `queue_index` rides beside it as a required
    // value field — exactly what d11's production lines do, and the same reading
    // d32 made of a loadout line. The trailer does not enforce that identity, so
    // [`decode_frame`] refuses a trailer that repeats one; see its docs.
    let railway = Collection {
        collection_id: RAILWAY_CONSTRUCTION,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::LINE_ID, Kind::Unsigned).key(1),
            Field::new(f::QUEUE_INDEX, Kind::Unsigned),
            Field::new(f::ACTIVE_FACTORIES, Kind::Unsigned),
            Field::new(f::COST_RAW, Kind::Unsigned),
            Field::new(f::PRODUCED_RAW, Kind::Unsigned),
            Field::new(f::CREATED_DATE_RAW, Kind::Unsigned),
            Field::new(f::BASE_LEVEL, Kind::Unsigned),
            Field::new(f::TARGET_LEVEL, Kind::Unsigned),
            Field::new(f::COMPLETION_FLAG, Kind::Boolean),
            // `-1` is the engine's explicit no-current-edge sentinel
            // (`lib.rs:315`-`:317`) — §3's null, not a failed read and not a
            // magic number to carry as a Signed word.
            Field::new(f::CURRENT_SEGMENT_INDEX, Kind::Unsigned).nullable(),
            // Fixed-5 over 100000 (`lib.rs:316`-`:317`), carried as the raw
            // integer the fold emits (`world_fold.rs::fold_construction_buildings_state_damage`); the fold's
            // `remaining_hours` float beside it is derived and stays off the wire.
            Field::new(f::REMAINING_HOURS_RAW, Kind::Unsigned),
        ],
    };

    // --- The railway route, a child collection of the row above.
    //
    // §4 declares no array kind, so the route rides as its own keyed set with
    // the position in the key — recipe step 5's third ordinal case, for rows
    // that have no identity of their own. The position persists across hours as
    // the same thing here: a line's route is the path it was planned on, and
    // `current_segment_index` is an index into exactly this array
    // (`lib.rs:1354`-`:1355`), so segment k of line L is the same segment next
    // hour. The parent id leads the key, d23's and d83's child convention.
    let route = Collection {
        collection_id: RAILWAY_ROUTE,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::LINE_ID, Kind::Unsigned).key(1),
            Field::new(f::ROUTE_ORDINAL, Kind::Unsigned).key(2),
            Field::new(f::ROUTE_PROVINCE_ID, Kind::ContentId),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![construction, buildings, resources, railway, route],
        event_part: None,
    }
}

// --- Frame mode --------------------------------------------------------------

/// The two typed inputs `world_fold.rs::fold_construction_buildings_state_damage` selects the accumulator
/// seed with. Both come from the caller — the delta bit from the payload status
/// word, the partial flag from `compact_contract`'s own tolerance predicate —
/// so neither is re-derived inside the fold boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameMode {
    /// `COMPACT_STATUS_DELTA_DATA` set on the payload status word (`:20277`).
    pub delta: bool,
    /// `compact_expected_partial_tolerance_applies` (`:20244`). Always false for
    /// domain 48 under today's tolerance table — see the module docs.
    pub expected_partial: bool,
}

// --- The payload header ------------------------------------------------------

/// The 64-byte `CBSD` header (`lib.rs:1118`-`:1158`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub status_flags: u32,
    pub game_hour_raw: i32,
    pub construction_count: u32,
    pub building_count: u32,
    pub resource_count: u32,
    pub observed_country_roots: u32,
    pub observed_state_roots: u32,
    pub observed_province_roots: u32,
    pub source_flags: u32,
}

impl PayloadHeader {
    /// Whether this frame badges the construction target-state join unavailable
    /// (`world_fold.rs::fold_construction_buildings_state_damage`).
    pub fn target_state_unavailable(&self) -> bool {
        self.source_flags & SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE != 0
    }
}

/// Read and check the 64-byte header alone, without touching the body.
///
/// A caller that only needs to *classify* a record — the status word for the
/// frame mode and the failure predicate — must be able to do it without paying
/// for the rows, which is why this is separate from [`split_payload`].
pub fn peek_header(payload: &[u8]) -> Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "construction_buildings_state_damage payload {} shorter than the \
             {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if payload[0..4] != MAGIC {
        return Err("construction_buildings_state_damage magic mismatch".into());
    }
    if u16_at(payload, 4) != VERSION {
        return Err("construction_buildings_state_damage version mismatch".into());
    }
    let header_len = u16_at(payload, 6) as usize;
    if header_len != HEADER_LEN {
        return Err(format!(
            "construction_buildings_state_damage unexpected header_len {header_len}"
        ));
    }
    if u16_at(payload, 8) != DOMAIN as u16 {
        return Err("construction_buildings_state_damage domain id mismatch".into());
    }
    if u16_at(payload, 10) != 0 {
        return Err("construction_buildings_state_damage schema reserved field is nonzero".into());
    }
    if u16_at(payload, 32) as usize != CONSTRUCTION_ROW_LEN
        || u16_at(payload, 34) as usize != BUILDING_ROW_LEN
        || u16_at(payload, 36) as usize != RESOURCE_ROW_LEN
    {
        return Err("construction_buildings_state_damage row length mismatch".into());
    }
    if u16_at(payload, 38) != 0 {
        return Err("construction_buildings_state_damage row reserved field is nonzero".into());
    }
    if u32_at(payload, 56) != 0 || u32_at(payload, 60) != 0 {
        return Err("construction_buildings_state_damage header reserved tail is nonzero".into());
    }
    Ok(PayloadHeader {
        status_flags: u32_at(payload, 12),
        game_hour_raw: i32_at(payload, 16),
        construction_count: u32_at(payload, 20),
        building_count: u32_at(payload, 24),
        resource_count: u32_at(payload, 28),
        observed_country_roots: u32_at(payload, 40),
        observed_state_roots: u32_at(payload, 44),
        observed_province_roots: u32_at(payload, 48),
        source_flags: u32_at(payload, 52),
    })
}

/// The four body slices, after [`peek_header`]'s checks and the fold's own
/// length arithmetic (`lib.rs:1159`-`:1168`).
///
/// The three row counts are attacker-shaped words from a stream on disk, so
/// every multiplication is overflow-checked. The trailing slice is whatever
/// follows the three tables: empty, or an `RLC1` trailer, and
/// [`decode_railway_trailer`] decides which.
///
/// Note what is **not** checked: there is no `changed_rows` word in this header
/// and therefore no `changed_rows`-versus-`row_count` invariant of any kind.
/// The fold's base-length check is `<`, not `==` (`lib.rs:1163`) — the trailer
/// is what legitimately follows — and the exact total length is then pinned by
/// the trailer's own `!=` check (`:1312`) or, without a trailer, by the empty
/// remainder.
#[allow(clippy::type_complexity)]
pub fn split_payload(
    payload: &[u8],
) -> Result<(PayloadHeader, &[u8], &[u8], &[u8], &[u8]), String> {
    let header = peek_header(payload)?;
    let construction_len = (header.construction_count as usize)
        .checked_mul(CONSTRUCTION_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "construction_buildings_state_damage construction_count {} overflows",
                header.construction_count
            )
        })?;
    let building_len = (header.building_count as usize)
        .checked_mul(BUILDING_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "construction_buildings_state_damage building_count {} overflows",
                header.building_count
            )
        })?;
    let resource_len = (header.resource_count as usize)
        .checked_mul(RESOURCE_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "construction_buildings_state_damage resource_count {} overflows",
                header.resource_count
            )
        })?;
    let base_len = HEADER_LEN
        .checked_add(construction_len)
        .and_then(|v| v.checked_add(building_len))
        .and_then(|v| v.checked_add(resource_len))
        .ok_or_else(|| "construction_buildings_state_damage body length overflows".to_string())?;
    if payload.len() < base_len {
        return Err(format!(
            "construction_buildings_state_damage payload length {} shorter than base {base_len}",
            payload.len()
        ));
    }
    let c0 = HEADER_LEN;
    let b0 = c0 + construction_len;
    let r0 = b0 + building_len;
    let t0 = r0 + resource_len;
    Ok((
        header,
        &payload[c0..b0],
        &payload[b0..r0],
        &payload[r0..t0],
        &payload[t0..],
    ))
}

// --- Rows --------------------------------------------------------------------

/// One construction row, parsed once and carrying its own bytes so the mapper
/// and the accumulator share a single derivation of the layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructionRow {
    pub raw: [u8; CONSTRUCTION_ROW_LEN],
    pub tag_db_id: u32,
    /// With the DELETED bit already masked off — this is the key component.
    pub line_id: u32,
    pub deleted: bool,
    pub template_id: Option<u32>,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub priority: u32,
    pub target_state_id: Option<u32>,
    pub target_location_id: Option<u32>,
    pub target_scope: Option<u32>,
    pub repair_levels_remaining: Option<u32>,
    pub created_date_raw: Option<u32>,
}

/// Parse and validate one construction row exactly as the lane does: the
/// decoder's per-row structural checks (`lib.rs:1172`-`:1216`) followed by
/// `validate_construction_row` (`:1635`-`:1741`). Every rejection the fold makes
/// is an `Err` here; no 52-byte input panics.
pub fn parse_construction_row(row: &[u8]) -> Result<ConstructionRow, String> {
    if row.len() != CONSTRUCTION_ROW_LEN {
        return Err(format!(
            "construction row length {} != {CONSTRUCTION_ROW_LEN}",
            row.len()
        ));
    }
    let flags = u32_at(row, 32);
    if flags & !CONSTRUCTION_ROW_KNOWN_FLAGS != 0 {
        return Err(format!("construction row has unknown flags {flags:#x}"));
    }
    let line_word = u32_at(row, 4);
    let deleted = line_word & ROW_ID_DELETED != 0;
    // `lib.rs:1181`-`:1192`: a tombstone's payload words must be inert.
    if deleted
        && (u32_at(row, 8) != u32::MAX
            || u32_at(row, 28) != 0
            || u32_at(row, 36) != 0
            || u32_at(row, 40) != 0
            || u32_at(row, 44) != 0
            || u32_at(row, 48) != 0)
    {
        return Err("construction row tombstone has live fields".into());
    }
    // `lib.rs:1193`-`:1214`: an ungated word must be zero-packed.
    if flags & CONSTRUCTION_FLAG_CREATED_DATE_PRESENT == 0 && u32_at(row, 36) != 0 {
        return Err("construction row created_date reserved field is nonzero".into());
    }
    for (flag, offset, name) in [
        (CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT, 40, "target_location"),
        (CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT, 44, "target_scope"),
        (CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT, 48, "repair_levels"),
    ] {
        if flags & flag == 0 && u32_at(row, offset) != 0 {
            return Err(format!(
                "construction row {name} reserved field is nonzero"
            ));
        }
    }

    let parsed = ConstructionRow {
        raw: row.try_into().expect("length checked above"),
        tag_db_id: u32_at(row, 0),
        line_id: line_word & !ROW_ID_DELETED,
        deleted,
        template_id: (flags & CONSTRUCTION_FLAG_TEMPLATE_PRESENT != 0).then(|| u32_at(row, 8)),
        active_factories: i32_at(row, 12),
        cost_raw: i32_at(row, 16),
        produced_raw: i32_at(row, 20),
        priority: u32_at(row, 24),
        target_state_id: (flags & CONSTRUCTION_FLAG_TARGET_STATE_PRESENT != 0)
            .then(|| u32_at(row, 28)),
        target_location_id: (flags & CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT != 0)
            .then(|| u32_at(row, 40)),
        target_scope: (flags & CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT != 0)
            .then(|| u32_at(row, 44)),
        repair_levels_remaining: (flags & CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT != 0)
            .then(|| u32_at(row, 48)),
        created_date_raw: (flags & CONSTRUCTION_FLAG_CREATED_DATE_PRESENT != 0)
            .then(|| u32_at(row, 36)),
    };

    // `validate_construction_row`, `lib.rs:1635`-`:1741`.
    if parsed.tag_db_id == 0 {
        return Err("construction row has zero tag_db_id".into());
    }
    if parsed.tag_db_id > MAX_COMPACT_OWNER_TAG_DB_ID {
        return Err(format!(
            "construction row invalid tag_db_id {}",
            parsed.tag_db_id
        ));
    }
    if parsed.deleted {
        if parsed.line_id == 0 {
            return Err("construction row has zero deleted line_id".into());
        }
        if parsed.template_id.is_some()
            || parsed.active_factories != 0
            || parsed.cost_raw != 0
            || parsed.produced_raw != 0
            || parsed.priority != 0
            || parsed.target_state_id.is_some()
            || parsed.target_location_id.is_some()
            || parsed.target_scope.is_some()
            || parsed.repair_levels_remaining.is_some()
            || parsed.created_date_raw.is_some()
        {
            return Err("construction row tombstone has live fields".into());
        }
        return Ok(parsed);
    }
    if parsed.line_id == 0 {
        return Err("construction row has zero line_id".into());
    }
    if parsed.template_id == Some(0) {
        return Err("construction row has zero template_id".into());
    }
    if let Some(template_id) = parsed.template_id {
        if template_id > MAX_CONSTRUCTION_TEMPLATE_ID {
            return Err(format!(
                "construction row invalid template_id {template_id}"
            ));
        }
    }
    if !(0..=MAX_ACTIVE_FACTORIES).contains(&parsed.active_factories) {
        return Err(format!(
            "construction row invalid active_factories {}",
            parsed.active_factories
        ));
    }
    if parsed.cost_raw < 0 || parsed.produced_raw < 0 {
        return Err("construction row has negative progress fields".into());
    }
    if let Some(state_id) = parsed.target_state_id {
        if state_id == 0 || state_id > MAX_STATE_ID {
            return Err(format!(
                "construction row invalid target_state_id {state_id}"
            ));
        }
    }
    if parsed.target_location_id.is_some() != parsed.target_scope.is_some() {
        return Err("construction row target_location_id/target_scope presence mismatch".into());
    }
    if let (Some(location_id), Some(scope)) = (parsed.target_location_id, parsed.target_scope) {
        if location_id == 0 || location_id > MAX_PROVINCE_ID {
            return Err(format!(
                "construction row invalid target_location_id {location_id}"
            ));
        }
        if scope != SCOPE_STATE && scope != SCOPE_PROVINCE {
            return Err(format!("construction row invalid target_scope {scope}"));
        }
        if scope == SCOPE_STATE && parsed.target_state_id != Some(location_id) {
            return Err("construction row state target location/state mismatch".into());
        }
        if scope == SCOPE_PROVINCE && parsed.target_state_id.is_none() {
            return Err("construction row province target lacks target_state_id".into());
        }
    }
    if let Some(repair_levels) = parsed.repair_levels_remaining {
        if repair_levels > BUILDING_LEVEL_SANE_MAX {
            return Err(format!(
                "construction row invalid repair_levels_remaining {repair_levels}"
            ));
        }
    }
    if let Some(created_date_raw) = parsed.created_date_raw {
        if created_date_raw == 0 || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW {
            return Err(format!(
                "construction row invalid created_date_raw {created_date_raw}"
            ));
        }
    }
    Ok(parsed)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildingRow {
    pub raw: [u8; BUILDING_ROW_LEN],
    pub scope: u32,
    pub location_id: u32,
    pub state_id: u32,
    /// With the DELETED bit already masked off — this is the key component.
    pub type_id: u32,
    pub deleted: bool,
    pub level: u32,
    pub healthy_level: Option<u32>,
    pub partial_health_raw: Option<u64>,
}

/// `lib.rs:1221`-`:1248` then `validate_building_row` (`:1743`-`:1805`).
pub fn parse_building_row(row: &[u8]) -> Result<BuildingRow, String> {
    if row.len() != BUILDING_ROW_LEN {
        return Err(format!(
            "building row length {} != {BUILDING_ROW_LEN}",
            row.len()
        ));
    }
    let flags = u32_at(row, 24);
    if flags & !BUILDING_ROW_KNOWN_FLAGS != 0 {
        return Err(format!("building row has unknown flags {flags:#x}"));
    }
    if flags & BUILDING_FLAG_DAMAGED != 0 && flags & BUILDING_FLAG_HEALTHY_PRESENT == 0 {
        return Err("building row damaged flag set without healthy level".into());
    }
    if u32_at(row, 28) != 0 {
        return Err("building row reserved field is nonzero".into());
    }
    if flags & BUILDING_FLAG_PARTIAL_HEALTH_PRESENT == 0 && u64_at(row, 32) != 0 {
        return Err("building row partial_health reserved field is nonzero".into());
    }
    let type_word = u32_at(row, 12);
    let deleted = type_word & ROW_ID_DELETED != 0;
    if deleted && (u32_at(row, 20) != u32::MAX || u64_at(row, 32) != 0) {
        return Err("building row tombstone has live fields".into());
    }

    let parsed = BuildingRow {
        raw: row.try_into().expect("length checked above"),
        scope: u32_at(row, 0),
        location_id: u32_at(row, 4),
        state_id: u32_at(row, 8),
        type_id: type_word & !ROW_ID_DELETED,
        deleted,
        level: u32_at(row, 16),
        healthy_level: (flags & BUILDING_FLAG_HEALTHY_PRESENT != 0).then(|| u32_at(row, 20)),
        partial_health_raw: (flags & BUILDING_FLAG_PARTIAL_HEALTH_PRESENT != 0)
            .then(|| u64_at(row, 32)),
    };

    if parsed.scope != SCOPE_STATE && parsed.scope != SCOPE_PROVINCE {
        return Err(format!("building row invalid scope {}", parsed.scope));
    }
    if parsed.location_id == 0 {
        return Err("building row has zero location_id".into());
    }
    if parsed.scope == SCOPE_STATE && parsed.location_id != parsed.state_id {
        return Err("building row state-scope location/state mismatch".into());
    }
    if parsed.scope == SCOPE_PROVINCE && parsed.location_id > MAX_PROVINCE_ID {
        return Err(format!(
            "building row invalid province_id {}",
            parsed.location_id
        ));
    }
    if parsed.state_id == 0 || parsed.state_id > MAX_STATE_ID {
        return Err(format!("building row invalid state_id {}", parsed.state_id));
    }
    if parsed.type_id == 0 {
        return Err("building row has zero type_id".into());
    }
    if parsed.deleted {
        if parsed.level != 0 || parsed.healthy_level.is_some() || parsed.partial_health_raw.is_some()
        {
            return Err("building row tombstone has live fields".into());
        }
        return Ok(parsed);
    }
    if parsed.level > BUILDING_LEVEL_SANE_MAX {
        return Err(format!("building row invalid level {}", parsed.level));
    }
    if let Some(healthy) = parsed.healthy_level {
        if healthy > parsed.level {
            return Err(format!(
                "building row healthy_level {healthy} > level {}",
                parsed.level
            ));
        }
    }
    if let Some(partial) = parsed.partial_health_raw {
        let max = u64::from(parsed.level).saturating_mul(CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL);
        if partial > max {
            return Err(format!(
                "building row partial_health_raw {partial} > maximum {max}"
            ));
        }
        if parsed.healthy_level == Some(parsed.level) && partial != 0 {
            return Err("building row undamaged building has nonzero partial_health_raw".into());
        }
    }
    Ok(parsed)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRow {
    pub raw: [u8; RESOURCE_ROW_LEN],
    pub state_id: u32,
    /// With the DELETED bit already masked off — this is the key component.
    pub resource_id: u32,
    pub deleted: bool,
    pub amount_raw: i32,
}

/// `lib.rs:1251`-`:1258` then `validate_resource_row` (`:1807`-`:1839`).
pub fn parse_resource_row(row: &[u8]) -> Result<ResourceRow, String> {
    if row.len() != RESOURCE_ROW_LEN {
        return Err(format!(
            "resource row length {} != {RESOURCE_ROW_LEN}",
            row.len()
        ));
    }
    if u32_at(row, 12) != 0 {
        return Err("resource row reserved field is nonzero".into());
    }
    let resource_word = u32_at(row, 4);
    let parsed = ResourceRow {
        raw: row.try_into().expect("length checked above"),
        state_id: u32_at(row, 0),
        resource_id: resource_word & !ROW_ID_DELETED,
        deleted: resource_word & ROW_ID_DELETED != 0,
        amount_raw: i32_at(row, 8),
    };
    if parsed.state_id == 0 || parsed.state_id > MAX_STATE_ID {
        return Err(format!("resource row invalid state_id {}", parsed.state_id));
    }
    if parsed.resource_id == 0 || parsed.resource_id == STATE_RESOURCE_NO_ID {
        return Err(format!(
            "resource row invalid resource_id {}",
            parsed.resource_id
        ));
    }
    if parsed.deleted {
        if parsed.amount_raw != 0 {
            return Err("resource row tombstone has live fields".into());
        }
        return Ok(parsed);
    }
    if !(0..=STATE_RESOURCE_MAX_ABS_RAW).contains(&parsed.amount_raw) {
        return Err(format!(
            "resource row invalid amount_raw {}",
            parsed.amount_raw
        ));
    }
    Ok(parsed)
}

/// One RLC1 row plus the slice of the trailer's shared province array it names.
/// There is no tombstone form: the trailer is a complete set, full-replaced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RailwayRow {
    pub raw: [u8; RAILWAY_TRAILER_ROW_LEN],
    pub tag_db_id: u32,
    pub line_id: u32,
    pub queue_index: u32,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub created_date_raw: u32,
    pub base_level: u32,
    pub target_level: u32,
    pub completion_flag: bool,
    pub current_segment_index: i32,
    pub remaining_hours_raw: i64,
    pub route_province_ids: Vec<u32>,
}

/// Decode the `RLC1` trailer exactly as `decode_railway_trailer` does
/// (`lib.rs:1277`-`:1394`), returning `None` for an empty trailing slice.
///
/// **One deliberate divergence, per the recipe's step-7 rule.** The trailer
/// enforces a strictly increasing `(tag_db_id, queue_index, line_id)`
/// (`:1360`-`:1364`), which does not make `(tag_db_id, line_id)` unique: rows
/// `(1, 0, 5)` and `(1, 1, 5)` both pass. This schema's railway identity is
/// `(tag_db_id, line_id)` (see [`schema`]), and a keyed set cannot hold two rows
/// under one key — mirroring the fold would silently keep one line and drop the
/// other, which is the data-loss defect this port exists to prevent. So a
/// trailer that repeats one identity is **refused**.
///
/// * *Shape limit*: `Shape::KeyedSet` is the only implemented shape, and the
///   alternative — putting `queue_index` in the key — churns every railway row
///   of every hour a line ahead of it completes.
/// * *Producer evidence*: the collector reads `line_id` from each distinct
///   `CBuildingProductionLine` object in one country's production vector
///   (`lib.rs:688`-`:761`); a duplicate would need the engine to hold one line
///   id in two vector slots. Measured: zero duplicates over the whole capture.
/// * A test pins the refusal.
pub fn decode_railway_trailer(trailer: &[u8]) -> Result<Option<Vec<RailwayRow>>, String> {
    if trailer.is_empty() {
        return Ok(None);
    }
    if trailer.len() < RAILWAY_TRAILER_HEADER_LEN {
        return Err("RLC1 trailer truncated".into());
    }
    if trailer[0..4] != RAILWAY_TRAILER_MAGIC {
        return Err("unknown D48 trailer magic".into());
    }
    if u16_at(trailer, 4) != RAILWAY_TRAILER_VERSION
        || u16_at(trailer, 6) as usize != RAILWAY_TRAILER_HEADER_LEN
        || u16_at(trailer, 16) as usize != RAILWAY_TRAILER_ROW_LEN
        || u16_at(trailer, 18) != 0
        || u32_at(trailer, 20) != 0
    {
        return Err("RLC1 header mismatch".into());
    }
    let row_count = u32_at(trailer, 8) as usize;
    let route_count = u32_at(trailer, 12) as usize;
    if route_count > row_count.saturating_mul(MAX_RAILWAY_ROUTE_PROVINCES as usize) {
        return Err("RLC1 route-count ceiling exceeded".into());
    }
    let rows_len = row_count
        .checked_mul(RAILWAY_TRAILER_ROW_LEN)
        .ok_or_else(|| "RLC1 row length overflow".to_string())?;
    let routes_len = route_count
        .checked_mul(4)
        .ok_or_else(|| "RLC1 route length overflow".to_string())?;
    let expected = RAILWAY_TRAILER_HEADER_LEN
        .checked_add(rows_len)
        .and_then(|v| v.checked_add(routes_len))
        .ok_or_else(|| "RLC1 total length overflow".to_string())?;
    if trailer.len() != expected {
        return Err(format!("RLC1 length {} != expected {expected}", trailer.len()));
    }
    let route_base = RAILWAY_TRAILER_HEADER_LEN + rows_len;
    let mut rows = Vec::with_capacity(row_count);
    let mut expected_route_start = 0usize;
    let mut previous_key: Option<(u32, u32, u32)> = None;
    let mut identities: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
    for index in 0..row_count {
        let offset = RAILWAY_TRAILER_HEADER_LEN + index * RAILWAY_TRAILER_ROW_LEN;
        let row = &trailer[offset..offset + RAILWAY_TRAILER_ROW_LEN];
        let tag_db_id = u32_at(row, 0);
        let line_id = u32_at(row, 4);
        let queue_index = u32_at(row, 8);
        let active_factories = i32_at(row, 12);
        let cost_raw = i32_at(row, 16);
        let produced_raw = i32_at(row, 20);
        let created_date_raw = u32_at(row, 24);
        let base_level = u32_at(row, 28);
        let target_level = u32_at(row, 32);
        let current_segment_index = i32_at(row, 36);
        let remaining_hours_raw = i64_at(row, 40);
        let route_start = u32_at(row, 48) as usize;
        let route_len = u32_at(row, 52) as usize;
        let flags = u32_at(row, 56);
        if u32_at(row, 60) != 0 || flags & !1 != 0 {
            return Err(format!("RLC1 row {index} flags/reserved invalid"));
        }
        if tag_db_id == 0
            || line_id == 0
            || !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories)
            || cost_raw < 0
            || produced_raw < 0
            || created_date_raw == 0
            || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW
            || base_level > BUILDING_LEVEL_SANE_MAX
            || target_level < base_level
            || target_level > BUILDING_LEVEL_SANE_MAX
            || !(2..=MAX_RAILWAY_ROUTE_PROVINCES as usize).contains(&route_len)
            || route_start != expected_route_start
            || route_start.saturating_add(route_len) > route_count
            || current_segment_index < -1
            || current_segment_index >= route_len as i32 - 1
            || remaining_hours_raw < 0
        {
            return Err(format!("RLC1 row {index} invariant failed"));
        }
        let ordering_key = (tag_db_id, queue_index, line_id);
        if previous_key.is_some_and(|previous| previous >= ordering_key) {
            return Err(format!("RLC1 row {index} is not canonically ordered"));
        }
        previous_key = Some(ordering_key);
        // The divergence documented above. `tag_db_id` is not part of the
        // trailer's uniqueness contract either, so the check is over the whole
        // identity rather than within a country's run.
        if !identities.insert((tag_db_id, line_id)) {
            return Err(format!(
                "RLC1 row {index} repeats railway identity (tag_db_id {tag_db_id}, \
                 line_id {line_id}); a keyed set cannot hold two rows under one key"
            ));
        }
        let mut route_province_ids = Vec::with_capacity(route_len);
        for route_index in 0..route_len {
            let province_id = u32_at(trailer, route_base + (route_start + route_index) * 4);
            if province_id == 0 || province_id > MAX_PROVINCE_ID {
                return Err(format!("RLC1 row {index} route province invalid"));
            }
            route_province_ids.push(province_id);
        }
        expected_route_start += route_len;
        rows.push(RailwayRow {
            raw: row.try_into().expect("64-byte slice"),
            tag_db_id,
            line_id,
            queue_index,
            active_factories,
            cost_raw,
            produced_raw,
            created_date_raw,
            base_level,
            target_level,
            completion_flag: flags & 1 != 0,
            current_segment_index,
            remaining_hours_raw,
            route_province_ids,
        });
    }
    if expected_route_start != route_count {
        return Err("RLC1 unreferenced route tail".into());
    }
    Ok(Some(rows))
}

/// One decoded d48 payload, the shape `decode_domain48_compact` returns
/// (`decoder_gate.rs:28`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub header: PayloadHeader,
    pub construction: Vec<ConstructionRow>,
    pub buildings: Vec<BuildingRow>,
    pub resources: Vec<ResourceRow>,
    /// `None` is a legacy payload without an `RLC1` trailer; `Some(empty)` is an
    /// authoritative complete snapshot proving there are no live rail lines
    /// (`lib.rs:365`-`:367`).
    pub railway: Option<Vec<RailwayRow>>,
}

/// Decode a whole d48 payload, exactly as `decode_lane_frame` does.
///
/// **Position matters (recipe step 9).** `fold_construction_buildings_state_damage`
/// calls this FIRST, before it looks at the status word, and returns `Err` on
/// failure (`world_fold.rs::fold_construction_buildings_state_damage`) — so a payload this rejects is a
/// **stream rejection**, not an hour to skip, whatever its status. That is the
/// d83 shape, not d63's, and copying d63's order here would have excluded
/// malformed failure-status records instead of refusing them. The production
/// authority reads it the same way: it decodes every d48 DATA record regardless
/// of status (`domain_data_authority.rs:7958`-`:7972`).
pub fn decode_frame(payload: &[u8]) -> Result<Frame, String> {
    let (header, construction_bytes, building_bytes, resource_bytes, trailer) =
        split_payload(payload)?;
    let mut construction = Vec::with_capacity(header.construction_count as usize);
    for (index, row) in construction_bytes
        .chunks_exact(CONSTRUCTION_ROW_LEN)
        .enumerate()
    {
        construction.push(parse_construction_row(row).map_err(|e| format!("row {index}: {e}"))?);
    }
    let mut buildings = Vec::with_capacity(header.building_count as usize);
    for (index, row) in building_bytes.chunks_exact(BUILDING_ROW_LEN).enumerate() {
        buildings.push(parse_building_row(row).map_err(|e| format!("row {index}: {e}"))?);
    }
    let mut resources = Vec::with_capacity(header.resource_count as usize);
    for (index, row) in resource_bytes.chunks_exact(RESOURCE_ROW_LEN).enumerate() {
        resources.push(parse_resource_row(row).map_err(|e| format!("row {index}: {e}"))?);
    }
    let railway = decode_railway_trailer(trailer)?;
    Ok(Frame {
        header,
        construction,
        buildings,
        resources,
        railway,
    })
}

/// The lane gate's verdict (`lib.rs:1413`-`:1504`).
///
/// A failed gate is an **excluded hour**, not a stream rejection: the fold
/// returns `Ok(())` and folds nothing (`world_fold.rs::fold_construction_buildings_state_damage`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GateReport {
    pub ok: bool,
    pub errors: Vec<String>,
    pub live_construction_rows: usize,
    pub construction_rows_with_target_state: usize,
    pub damaged_building_rows: usize,
}

/// `gate_decoded_frame`, `lib.rs:1413`-`:1504`, rule for rule.
pub fn gate_frame(frame: &Frame) -> GateReport {
    use std::collections::BTreeSet;
    let mut errors = Vec::new();
    // `COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_SOURCE_UNAVAILABLE`,
    // `lib.rs:21`-`:22`, `:1415`. Both are also in the global failure mask, so
    // this arm never decides an hour the status check has not already excluded —
    // it is reproduced because the gate is the fold's authority, not an
    // optimisation.
    const BLOCKING: u32 = (1 << 6) | (1 << 8);
    if frame.header.status_flags & BLOCKING != 0 {
        errors.push(format!(
            "blocking status_flags set: {:#x}",
            frame.header.status_flags
        ));
    }
    if frame.header.source_flags & SOURCE_FLAG_DLL_NATIVE == 0 {
        errors.push("source_flags missing DLL-native bit".into());
    }
    // `lib.rs:1425`: the payload's own two mode bits, not the caller's
    // `FrameMode` — the gate runs before the tolerance table is consulted.
    const STATUS_PARTIAL_DATA: u32 = 1 << 9;
    const STATUS_DELTA_DATA: u32 = 1 << 10;
    let clean_full_frame = frame.header.status_flags & (STATUS_PARTIAL_DATA | STATUS_DELTA_DATA) == 0;
    if clean_full_frame && frame.buildings.is_empty() {
        errors.push("no building rows emitted".into());
    }
    if clean_full_frame && frame.resources.is_empty() {
        errors.push("no state resource rows emitted".into());
    }
    let live_construction_rows = frame.construction.iter().filter(|r| !r.deleted).count();
    let construction_rows_with_target_state = frame
        .construction
        .iter()
        .filter(|r| !r.deleted && r.target_state_id.is_some())
        .count();
    let target_state_unavailable = frame.header.target_state_unavailable();
    if live_construction_rows != 0 {
        if construction_rows_with_target_state == 0 && !target_state_unavailable {
            errors.push(
                "construction target_state_id absent without target-state-unavailable source flag"
                    .into(),
            );
        }
        if target_state_unavailable && construction_rows_with_target_state > 0 {
            errors.push(
                "construction target_state_id emitted while target-state-unavailable source flag \
                 is set"
                    .into(),
            );
        }
        if construction_rows_with_target_state > 0
            && construction_rows_with_target_state != live_construction_rows
        {
            errors.push(
                "construction target_state_id partially emitted; fill every construction row or \
                 badge unavailable"
                    .into(),
            );
        }
    }
    let mut seen_construction = BTreeSet::new();
    for (index, row) in frame.construction.iter().enumerate() {
        if !seen_construction.insert((row.tag_db_id, row.line_id)) {
            errors.push(format!("construction row {index} duplicates tag/line"));
        }
    }
    // NOTE, reported as a fold observation: the gate's building uniqueness is
    // over `(scope, location_id, type_id)` (`lib.rs:1476`) while the fold's
    // accumulator key is `(scope, location_id, state_id, type_id)`
    // (`world_fold.rs::domain48_building_key`). For a state-scope row the two coincide, because
    // `location_id == state_id` is enforced (`lib.rs:1750`). For a province-scope
    // row whose owning state changed, the accumulator gains a second entry under
    // the new key while the old one persists. This mirrors the gate exactly
    // rather than tightening it, because refusing what the fold accepts is the
    // failure this recipe exists to prevent; the accumulator consequence is
    // reproduced faithfully by keying on all four components.
    let mut seen_buildings = BTreeSet::new();
    let mut damaged_building_rows = 0usize;
    for (index, row) in frame.buildings.iter().enumerate() {
        if !seen_buildings.insert((row.scope, row.location_id, row.type_id)) {
            errors.push(format!(
                "building row {index} duplicates scope/location/type"
            ));
        }
        if !row.deleted && row.healthy_level.is_some_and(|healthy| healthy < row.level) {
            damaged_building_rows += 1;
        }
    }
    let mut seen_resources = BTreeSet::new();
    for (index, row) in frame.resources.iter().enumerate() {
        if !seen_resources.insert((row.state_id, row.resource_id)) {
            errors.push(format!("resource row {index} duplicates state/resource"));
        }
    }
    GateReport {
        ok: errors.is_empty(),
        errors,
        live_construction_rows,
        construction_rows_with_target_state,
        damaged_building_rows,
    }
}

// --- Keys --------------------------------------------------------------------

/// `world_fold.rs::domain48_construction_key`: `(tag_db_id, line_id & !ROW_ID_DELETED)`.
pub fn key_from_construction_row(row: &[u8]) -> Result<Key, String> {
    let parsed = parse_construction_row(row)?;
    Ok(construction_key(parsed.tag_db_id, parsed.line_id))
}

pub fn construction_key(tag_db_id: u32, line_id: u32) -> Key {
    key(&[tag_db_id as u64, line_id as u64])
}

/// `world_fold.rs::domain48_building_key`: `(scope, location_id, state_id, type_id & !ROW_ID_DELETED)`.
pub fn key_from_building_row(row: &[u8]) -> Result<Key, String> {
    let parsed = parse_building_row(row)?;
    Ok(building_key(
        parsed.scope,
        parsed.location_id,
        parsed.state_id,
        parsed.type_id,
    ))
}

pub fn building_key(scope: u32, location_id: u32, state_id: u32, type_id: u32) -> Key {
    key(&[
        scope as u64,
        location_id as u64,
        state_id as u64,
        type_id as u64,
    ])
}

/// `world_fold.rs::domain48_resource_identity_key`: `(state_id, resource_id & !ROW_ID_DELETED)`.
pub fn key_from_resource_row(row: &[u8]) -> Result<Key, String> {
    let parsed = parse_resource_row(row)?;
    Ok(resource_key(parsed.state_id, parsed.resource_id))
}

pub fn resource_key(state_id: u32, resource_id: u32) -> Key {
    key(&[state_id as u64, resource_id as u64])
}

/// The railway identity decided in [`schema`]: `(tag_db_id, line_id)`.
pub fn key_from_railway_row(row: &[u8]) -> Result<Key, String> {
    if row.len() != RAILWAY_TRAILER_ROW_LEN {
        return Err(format!(
            "RLC1 row length {} != {RAILWAY_TRAILER_ROW_LEN}",
            row.len()
        ));
    }
    Ok(railway_key(u32_at(row, 0), u32_at(row, 4)))
}

pub fn railway_key(tag_db_id: u32, line_id: u32) -> Key {
    key(&[tag_db_id as u64, line_id as u64])
}

/// A route entry's key. The parent id leads, d23's and d83's child convention
/// (`d23.rs:576`, `d83.rs:627`), so a caller that already parsed the parent does
/// not parse it twice; the ordinal is the array position, generated at this
/// boundary and counted in exactly one place.
pub fn key_from_railway_route_row(parent: (u32, u32), ordinal: u32) -> Key {
    key(&[parent.0 as u64, parent.1 as u64, ordinal as u64])
}

// --- The boundary mapping ----------------------------------------------------

/// One compact row of the `construction` table, as the collector emits it today,
/// onto its stored Record.
///
/// `row` is the raw 52-byte row slice (the caller has already consumed the table
/// header via [`split_payload`]); d48 has exactly one construction row form, so
/// length selects nothing and a wrong length is rejected. `frame_tick` is §6's
/// derived-field input; d48 declares no derived field, so it is taken and
/// ignored — every domain's mapper has the same shape.
///
/// `target_state_unavailable` is the frame's
/// `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE` bit
/// (`world_fold.rs::fold_construction_buildings_state_damage`), the d17-shaped typed input: it decides
/// `target_state_id` for **every** row of the frame, including rows the
/// accumulator retained from an earlier frame, and it is never re-derived here.
///
/// A tombstone row is a **removal**, not a record: mapping one would publish a
/// zero-valued row under a live key, so it is refused.
pub fn record_from_construction_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
    target_state_unavailable: bool,
) -> Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != CONSTRUCTION {
        return Err(format!(
            "record_from_construction_row called with collection {}",
            collection.collection_id
        ));
    }
    let parsed = parse_construction_row(row)?;
    if parsed.deleted {
        return Err("construction tombstone is a removal, not a record".into());
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::PRIORITY => Value::Unsigned(parsed.priority as u64),
            field::TEMPLATE_ID => unknown_or(parsed.template_id.map(|v| Value::ContentId(v as u64))),
            field::ACTIVE_FACTORIES => Value::Unsigned(parsed.active_factories as u64),
            field::COST_RAW => Value::Unsigned(parsed.cost_raw as u64),
            field::PRODUCED_RAW => Value::Unsigned(parsed.produced_raw as u64),
            field::TARGET_STATE_ID => {
                if target_state_unavailable {
                    Value::Unknown
                } else {
                    unknown_or(parsed.target_state_id.map(|v| Value::ContentId(v as u64)))
                }
            }
            // The fold gates these two on the row's own option ONLY — not on the
            // frame bit (`world_fold.rs::fold_construction_buildings_state_damage`). Mirrored exactly.
            field::TARGET_LOCATION_ID => {
                unknown_or(parsed.target_location_id.map(|v| Value::ContentId(v as u64)))
            }
            field::TARGET_SCOPE => match parsed.target_scope {
                Some(SCOPE_STATE) => Value::Enum(scope_code::STATE),
                Some(SCOPE_PROVINCE) => Value::Enum(scope_code::PROVINCE),
                // `parse_construction_row` has already refused any other code,
                // so this arm is unreachable; it is an `Err` rather than an
                // `unreachable!` because a panic in a public mapper is a defect
                // class of its own.
                Some(other) => return Err(format!("construction row invalid target_scope {other}")),
                None => Value::Unknown,
            },
            field::REPAIR_LEVELS_REMAINING => unknown_or(
                parsed
                    .repair_levels_remaining
                    .map(|v| Value::Unsigned(v as u64)),
            ),
            field::CREATED_DATE_RAW => {
                unknown_or(parsed.created_date_raw.map(|v| Value::Unsigned(v as u64)))
            }
            other => return Err(no_mapping("construction", other)),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact row of the `buildings` table onto its stored Record. See
/// [`record_from_construction_row`] for the shared contract.
pub fn record_from_building_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != BUILDINGS {
        return Err(format!(
            "record_from_building_row called with collection {}",
            collection.collection_id
        ));
    }
    let parsed = parse_building_row(row)?;
    if parsed.deleted {
        return Err("building tombstone is a removal, not a record".into());
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::LEVEL => Value::Unsigned(parsed.level as u64),
            field::HEALTHY_LEVEL => unknown_or(parsed.healthy_level.map(|v| Value::Unsigned(v as u64))),
            field::PARTIAL_HEALTH_RAW => {
                unknown_or(parsed.partial_health_raw.map(Value::Unsigned))
            }
            other => return Err(no_mapping("buildings", other)),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact row of the `state_resources` table onto its stored Record.
pub fn record_from_resource_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != STATE_RESOURCES {
        return Err(format!(
            "record_from_resource_row called with collection {}",
            collection.collection_id
        ));
    }
    let parsed = parse_resource_row(row)?;
    if parsed.deleted {
        return Err("resource tombstone is a removal, not a record".into());
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::AMOUNT_RAW => Value::Unsigned(parsed.amount_raw as u64),
            other => return Err(no_mapping("state_resources", other)),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `RLC1` row onto its stored Record. The route it names is a separate
/// collection; see [`record_from_railway_route_row`].
pub fn record_from_railway_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != RAILWAY_CONSTRUCTION {
        return Err(format!(
            "record_from_railway_row called with collection {}",
            collection.collection_id
        ));
    }
    if row.len() != RAILWAY_TRAILER_ROW_LEN {
        return Err(format!(
            "RLC1 row length {} != {RAILWAY_TRAILER_ROW_LEN}",
            row.len()
        ));
    }
    // Every invariant `decode_railway_trailer` (`lib.rs:1338`-`:1359`) applies
    // that ONE ROW can decide on its own is re-applied here, so this public
    // entry point is fail-closed on exactly the rows the trailer would refuse.
    // The two the trailer checks that are NOT row-local are deliberately absent:
    // `route_start == expected_route_start` and
    // `route_start + route_len <= route_count` are relative to the trailer's
    // shared province array and its running cursor, so a single row carries no
    // way to decide them. `decode_railway_trailer` is the full authority and is
    // what the accumulator goes through, so nothing in a built state can reach
    // this function and fail.
    let flags = u32_at(row, 56);
    if u32_at(row, 60) != 0 || flags & !1 != 0 {
        return Err("RLC1 row flags/reserved invalid".into());
    }
    let tag_db_id = u32_at(row, 0);
    let line_id = u32_at(row, 4);
    if tag_db_id == 0 || line_id == 0 {
        return Err("RLC1 row has a zero identity component".into());
    }
    let active_factories = i32_at(row, 12);
    if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories) {
        return Err(format!(
            "RLC1 row invalid active_factories {active_factories}"
        ));
    }
    let cost_raw = i32_at(row, 16);
    let produced_raw = i32_at(row, 20);
    if cost_raw < 0 || produced_raw < 0 {
        return Err("RLC1 row has negative progress fields".into());
    }
    let created_date_raw = u32_at(row, 24);
    if created_date_raw == 0 || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW {
        return Err(format!(
            "RLC1 row invalid created_date_raw {created_date_raw}"
        ));
    }
    let base_level = u32_at(row, 28);
    let target_level = u32_at(row, 32);
    if base_level > BUILDING_LEVEL_SANE_MAX
        || target_level > BUILDING_LEVEL_SANE_MAX
        || target_level < base_level
    {
        return Err(format!(
            "RLC1 row invalid level range {base_level}..={target_level}"
        ));
    }
    let route_len = u32_at(row, 52) as usize;
    if !(2..=MAX_RAILWAY_ROUTE_PROVINCES as usize).contains(&route_len) {
        return Err(format!("RLC1 row invalid route_len {route_len}"));
    }
    let current_segment_index = i32_at(row, 36);
    if current_segment_index < -1 || current_segment_index >= route_len as i32 - 1 {
        return Err(format!(
            "RLC1 row invalid current_segment_index {current_segment_index} for route_len \
             {route_len}"
        ));
    }
    let remaining_hours_raw = i64_at(row, 40);
    if remaining_hours_raw < 0 {
        return Err(format!(
            "RLC1 row negative remaining_hours_raw {remaining_hours_raw}"
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::QUEUE_INDEX => Value::Unsigned(u32_at(row, 8) as u64),
            field::ACTIVE_FACTORIES => Value::Unsigned(active_factories as u64),
            field::COST_RAW => Value::Unsigned(cost_raw as u64),
            field::PRODUCED_RAW => Value::Unsigned(produced_raw as u64),
            field::CREATED_DATE_RAW => Value::Unsigned(created_date_raw as u64),
            field::BASE_LEVEL => Value::Unsigned(base_level as u64),
            field::TARGET_LEVEL => Value::Unsigned(target_level as u64),
            field::COMPLETION_FLAG => Value::Bool(flags & 1 != 0),
            // `-1` is the engine's no-current-edge sentinel (`lib.rs:315`).
            field::CURRENT_SEGMENT_INDEX => {
                if current_segment_index < 0 {
                    Value::Null
                } else {
                    Value::Unsigned(current_segment_index as u64)
                }
            }
            field::REMAINING_HOURS_RAW => Value::Unsigned(remaining_hours_raw as u64),
            other => return Err(no_mapping("railway_construction", other)),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One province of one railway line's route onto its stored Record. `entry` is
/// the 4-byte province id word from the trailer's shared route array.
pub fn record_from_railway_route_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != RAILWAY_ROUTE {
        return Err(format!(
            "record_from_railway_route_row called with collection {}",
            collection.collection_id
        ));
    }
    if entry.len() != 4 {
        return Err(format!("RLC1 route entry length {} != 4", entry.len()));
    }
    let province_id = u32_at(entry, 0);
    if province_id == 0 || province_id > MAX_PROVINCE_ID {
        return Err(format!("RLC1 route province invalid {province_id}"));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::ROUTE_PROVINCE_ID => Value::ContentId(province_id as u64),
            other => return Err(no_mapping("railway_route", other)),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

fn unknown_or(value: Option<Value>) -> Value {
    value.unwrap_or(Value::Unknown)
}

/// A collection this mapper does not know how to fill. Filling it with a
/// placeholder would publish invented data under a field id that means something
/// else.
fn no_mapping(table: &str, field_id: u32) -> String {
    format!("construction_buildings_state_damage {table} has no mapping for field id {field_id}")
}

// --- The accumulator ---------------------------------------------------------

type ConstructionKey = (u32, u32);
type BuildingKey = (u32, u32, u32, u32);
type ResourceKey = (u32, u32);

/// `Domain48FoldState`, `world_fold.rs:2123`-`:2131`.
///
/// Rows are kept as the collector emitted them so the mapper's fixed
/// `&[u8]` signature and this accumulator share one derivation of the layout,
/// and so that a frame-level flag change re-derives every construction record
/// exactly as the fold re-derives its whole JSON projection each frame.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Domain48State {
    pub source_flags: u32,
    pub construction: BTreeMap<ConstructionKey, ConstructionRow>,
    pub buildings: BTreeMap<BuildingKey, BuildingRow>,
    pub resources: BTreeMap<ResourceKey, ResourceRow>,
    /// `None` until an `RLC1` trailer has ever been seen (`world_fold.rs::fold_construction_buildings_state_damage`).
    pub railway: Option<Vec<RailwayRow>>,
}

impl Domain48State {
    pub fn target_state_unavailable(&self) -> bool {
        self.source_flags & SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE != 0
    }

    pub fn rows(&self) -> usize {
        self.construction.len()
            + self.buildings.len()
            + self.resources.len()
            + self
                .railway
                .as_ref()
                .map_or(0, |rows| rows.len() + rows.iter().map(|r| r.route_province_ids.len()).sum::<usize>())
    }

    /// The five keyed states this accumulator represents, in schema order.
    ///
    /// **`railway == None` is refused here, and that is a recorded divergence.**
    /// A keyed set has no way to say "this collection's membership was not
    /// measured" — that is §8 gap normalisation, which has no implementation
    /// (`lib.rs`) — so emitting zero railway rows for a state that has never
    /// seen an `RLC1` trailer would publish "measured, no rail lines", which is
    /// a false statement rather than a missing one. Producer evidence that this
    /// cannot happen on a modern stream: the collector always emits the trailer
    /// (`collector_module.rs:190` sets `railway_construction_rows: Some(...)`
    /// unconditionally), so a `None` requires a legacy pre-`RLC1` payload. A
    /// test pins the refusal.
    pub fn to_states(&self, schema: &Schema, frame_tick: u64) -> Result<Vec<KeyedState>, String> {
        if schema.collections.len() != COLLECTION_COUNT {
            return Err(format!(
                "construction_buildings_state_damage schema has {} collections, expected \
                 {COLLECTION_COUNT}",
                schema.collections.len()
            ));
        }
        let unavailable = self.target_state_unavailable();
        let mut construction = KeyedState::new();
        for (key, row) in &self.construction {
            construction.insert(
                construction_key(key.0, key.1),
                record_from_construction_row(
                    &schema.collections[CONSTRUCTION_INDEX],
                    &row.raw,
                    frame_tick,
                    unavailable,
                )?,
            );
        }
        let mut buildings = KeyedState::new();
        for (key, row) in &self.buildings {
            buildings.insert(
                building_key(key.0, key.1, key.2, key.3),
                record_from_building_row(
                    &schema.collections[BUILDINGS_INDEX],
                    &row.raw,
                    frame_tick,
                )?,
            );
        }
        let mut resources = KeyedState::new();
        for (key, row) in &self.resources {
            resources.insert(
                resource_key(key.0, key.1),
                record_from_resource_row(
                    &schema.collections[STATE_RESOURCES_INDEX],
                    &row.raw,
                    frame_tick,
                )?,
            );
        }
        let mut railway = KeyedState::new();
        let mut route = KeyedState::new();
        let Some(rows) = self.railway.as_ref() else {
            return Err(
                "construction_buildings_state_damage state has never observed an RLC1 trailer; a \
                 keyed set cannot express an unmeasured collection (WIRE_FORMAT.md §8)"
                    .into(),
            );
        };
        for row in rows {
            let parent = (row.tag_db_id, row.line_id);
            if railway
                .insert(
                    railway_key(parent.0, parent.1),
                    record_from_railway_row(
                        &schema.collections[RAILWAY_CONSTRUCTION_INDEX],
                        &row.raw,
                        frame_tick,
                    )?,
                )
                .is_some()
            {
                // `decode_railway_trailer` already refuses this; the insert is
                // where a silent overwrite would happen, so the check belongs
                // here too.
                return Err(format!(
                    "construction_buildings_state_damage state repeats railway identity \
                     (tag_db_id {}, line_id {})",
                    parent.0, parent.1
                ));
            }
            for (ordinal, province_id) in row.route_province_ids.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| "RLC1 route ordinal overflows u32".to_string())?;
                route.insert(
                    key_from_railway_route_row(parent, ordinal),
                    record_from_railway_route_row(
                        &schema.collections[RAILWAY_ROUTE_INDEX],
                        &province_id.to_le_bytes(),
                        frame_tick,
                    )?,
                );
            }
        }
        Ok(vec![construction, buildings, resources, railway, route])
    }
}

/// The recorder's two d48 accumulators (`world_fold.rs:1571`, `:1580`) and the
/// three-way seed that reads them.
///
/// The recorder holds each as an `Option` and seeds with `unwrap_or_default()`
/// (`world_fold.rs::fold_construction_buildings_state_damage`), which is indistinguishable from holding a default, so
/// these are plain values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Domain48Fold {
    /// `domain48_state` — always reflects the last folded frame, including a
    /// tolerated partial's sparse merge.
    pub projected: Domain48State,
    /// `domain48_delta_state` — the baseline a delta is computed against, which
    /// a tolerated-partial frame must never advance.
    pub delta_baseline: Domain48State,
}

impl Domain48Fold {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one d48 frame, exactly as `fold_construction_buildings_state_damage`
    /// does once its four non-folding exits are behind it
    /// (`world_fold.rs::fold_construction_buildings_state_damage`).
    ///
    /// **The caller applies the exclusions first**, in the fold's own order:
    /// a decode failure (a stream rejection, [`decode_frame`]), a failure
    /// status, a failed gate, and a delta before any baseline. Calling this on
    /// an excluded record would fold an hour the recorder drops.
    ///
    /// **Error contract: all-or-nothing.** The fold builds its whole working
    /// state before assigning either accumulator, and its one post-seed error
    /// path (an invalid building scope, `world_fold.rs::fold_construction_buildings_state_damage`) fires before the
    /// assignment, so neither accumulator is touched by a frame this rejects.
    /// This mirrors that; a harness that assumed the other contract would
    /// disagree with the recorder on exactly the frames that fail.
    ///
    /// Implemented as [`Domain48Fold::project`] followed by
    /// [`Domain48Fold::commit`] so that [`fold_payload`], which has a second
    /// fallible step after the fold, can put its own check between the two.
    pub fn apply(&mut self, frame: &Frame, mode: FrameMode) -> Result<(), String> {
        let state = self.project(frame, mode)?;
        self.commit(state, mode);
        Ok(())
    }

    /// Build the state this frame folds to, **without touching either
    /// accumulator**. This is the whole of the recorder's fold body
    /// (`world_fold.rs::fold_construction_buildings_state_damage`) up to but not including its two
    /// assignments.
    pub fn project(&self, frame: &Frame, mode: FrameMode) -> Result<Domain48State, String> {
        // `world_fold.rs::fold_construction_buildings_state_damage` — the three-way seed.
        let mut state = if mode.delta {
            self.delta_baseline.clone()
        } else if mode.expected_partial {
            self.projected.clone()
        } else {
            Domain48State::default()
        };
        state.source_flags = frame.header.source_flags;

        for row in &frame.construction {
            let key = (row.tag_db_id, row.line_id);
            if row.deleted {
                state.construction.remove(&key);
            } else {
                state.construction.insert(key, row.clone());
            }
        }
        for row in &frame.buildings {
            let key = (row.scope, row.location_id, row.state_id, row.type_id);
            if row.deleted {
                state.buildings.remove(&key);
            } else {
                state.buildings.insert(key, row.clone());
            }
        }
        for row in &frame.resources {
            let key = (row.state_id, row.resource_id);
            if row.deleted {
                state.resources.remove(&key);
            } else {
                state.resources.insert(key, row.clone());
            }
        }
        // `world_fold.rs::fold_construction_buildings_state_damage`: RLC1 is a complete set, full-replaced
        // rather than merged into the row-delta grammar — and a tolerated-partial
        // frame must NOT replace it, because a partial cannot prove absence.
        if !mode.expected_partial {
            if let Some(rows) = frame.railway.as_ref() {
                state.railway = Some(rows.clone());
            }
        }

        // `world_fold.rs::fold_construction_buildings_state_damage`: an invalid building scope is an `Err`
        // after the state was built and before it is stored. `parse_building_row`
        // has already refused any other scope, so this cannot fire; it is here
        // because the fold has it, and a scope that reached this point would be
        // a defect in the parser rather than in the payload.
        for row in state.buildings.values() {
            if row.scope != SCOPE_STATE && row.scope != SCOPE_PROVINCE {
                return Err(format!(
                    "domain48 building row has invalid scope {}",
                    row.scope
                ));
            }
        }

        Ok(state)
    }

    /// Store a projected state into both accumulators, on the recorder's own
    /// advance conditions (`world_fold.rs::fold_construction_buildings_state_damage`): the delta baseline
    /// advances only on a clean or delta frame, while the projected state always
    /// takes this frame. Infallible by construction — every way this fold can
    /// fail has already happened in [`Domain48Fold::project`].
    pub fn commit(&mut self, state: Domain48State, mode: FrameMode) {
        if !mode.expected_partial {
            self.delta_baseline = state.clone();
        }
        self.projected = state;
    }
}

/// The recipe's fourth public boundary: fold one payload into the hour's five
/// keyed states, through the same code path the tests and the harness share.
///
/// This is the whole grammar in one place — the three-way seed, the row-delta
/// merge, the RLC1 full replacement and the frame-flag-driven construction
/// projection — because a harness that re-derived any of it could drift from the
/// recorder without anything failing.
///
/// **Error contract: all-or-nothing, over BOTH of its steps.** The fold itself
/// is all-or-nothing ([`Domain48Fold::apply`]), but this boundary adds a second
/// fallible step after it — [`Domain48State::to_states`], which refuses a state
/// that has never observed an `RLC1` trailer — so committing between the two
/// would leave both accumulators advanced behind an `Err`. It therefore projects
/// first, checks representability second, and commits last. Costs nothing
/// extra: `project` was already building a fresh state, and `commit` takes it by
/// value.
///
/// **What the `Err` from the second step means is narrower than a fold
/// rejection, and the distinction matters.** The recorder *accepts* a legacy
/// trailer-less frame — it folds it, stores it, and simply projects no railway
/// rows (`world_fold.rs::fold_construction_buildings_state_damage`). So an `Err` here does not say "the
/// recorder rejected this frame"; it says "the resulting state is not
/// expressible as keyed sets", which makes the **stream** unmeasurable rather
/// than the frame invalid. A caller must abort, not skip the hour and continue —
/// and `bench_real_d48` does exactly that, refusing the stream in pass 1. Use
/// [`Domain48Fold::apply`] directly where recorder-exact accumulator semantics
/// are wanted without the wire-representability check.
pub fn fold_payload(
    schema: &Schema,
    fold: &mut Domain48Fold,
    frame: &Frame,
    mode: FrameMode,
    frame_tick: u64,
) -> Result<Vec<KeyedState>, String> {
    let state = fold.project(frame, mode)?;
    let states = state.to_states(schema, frame_tick)?;
    fold.commit(state, mode);
    Ok(states)
}

// --- Little-endian readers ---------------------------------------------------
//
// Callers reach these only after a length check, so the slices are in bounds.

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    u32_at(b, off) as i32
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    u64_at(b, off) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::diff;
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn s() -> Schema {
        schema()
    }

    fn c(index: usize) -> Collection {
        s().collections[index].clone()
    }

    fn value_of(collection: &Collection, record: &Record, field_id: u32) -> Value {
        let index = collection
            .value_fields()
            .position(|f| f.field_id == field_id)
            .expect("field is in the collection");
        record.values[index]
    }

    // --- Raw row builders, packed exactly as the lane's writers pack them
    // (`lib.rs:1517`, `:1554`, `:1576`, `:1079`). ------------------------------

    #[allow(clippy::too_many_arguments)]
    fn construction_row(
        tag_db_id: u32,
        line_word: u32,
        template_id: Option<u32>,
        active_factories: i32,
        cost_raw: i32,
        produced_raw: i32,
        priority: u32,
        target_state_id: Option<u32>,
        created_date_raw: Option<u32>,
        target_location_id: Option<u32>,
        target_scope: Option<u32>,
        repair_levels_remaining: Option<u32>,
    ) -> Vec<u8> {
        let mut flags = 0u32;
        if template_id.is_some() {
            flags |= CONSTRUCTION_FLAG_TEMPLATE_PRESENT;
        }
        if target_state_id.is_some() {
            flags |= CONSTRUCTION_FLAG_TARGET_STATE_PRESENT;
        }
        if created_date_raw.is_some() {
            flags |= CONSTRUCTION_FLAG_CREATED_DATE_PRESENT;
        }
        if target_location_id.is_some() {
            flags |= CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT;
        }
        if target_scope.is_some() {
            flags |= CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT;
        }
        if repair_levels_remaining.is_some() {
            flags |= CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT;
        }
        let mut row = Vec::with_capacity(CONSTRUCTION_ROW_LEN);
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&line_word.to_le_bytes());
        row.extend_from_slice(&template_id.unwrap_or(u32::MAX).to_le_bytes());
        row.extend_from_slice(&active_factories.to_le_bytes());
        row.extend_from_slice(&cost_raw.to_le_bytes());
        row.extend_from_slice(&produced_raw.to_le_bytes());
        row.extend_from_slice(&priority.to_le_bytes());
        row.extend_from_slice(&target_state_id.unwrap_or(0).to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&created_date_raw.unwrap_or(0).to_le_bytes());
        row.extend_from_slice(&target_location_id.unwrap_or(0).to_le_bytes());
        row.extend_from_slice(&target_scope.unwrap_or(0).to_le_bytes());
        row.extend_from_slice(&repair_levels_remaining.unwrap_or(0).to_le_bytes());
        assert_eq!(row.len(), CONSTRUCTION_ROW_LEN);
        row
    }

    /// The reference construction row: every optional field present, a province
    /// target, a repair line and a plausible creation date.
    fn full_construction_row() -> Vec<u8> {
        construction_row(
            7,
            1_234,
            Some(12_790),
            3,
            500_000,
            250_000,
            2,
            Some(42),
            Some(60_000_000),
            Some(2_068),
            Some(SCOPE_PROVINCE),
            Some(4),
        )
    }

    /// The same row with every gate clear — the four-state case.
    fn gated_off_construction_row() -> Vec<u8> {
        construction_row(7, 1_234, None, 0, 0, 0, 0, None, None, None, None, None)
    }

    fn construction_tombstone_row(tag_db_id: u32, line_id: u32) -> Vec<u8> {
        construction_row(
            tag_db_id,
            line_id | ROW_ID_DELETED,
            None,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
        )
    }

    fn building_row(
        scope: u32,
        location_id: u32,
        state_id: u32,
        type_word: u32,
        level: u32,
        healthy_level: Option<u32>,
        partial_health_raw: Option<u64>,
    ) -> Vec<u8> {
        let mut flags = 0u32;
        if let Some(healthy) = healthy_level {
            flags |= BUILDING_FLAG_HEALTHY_PRESENT;
            if healthy < level {
                flags |= BUILDING_FLAG_DAMAGED;
            }
        }
        if partial_health_raw.is_some() {
            flags |= BUILDING_FLAG_PARTIAL_HEALTH_PRESENT;
        }
        let mut row = Vec::with_capacity(BUILDING_ROW_LEN);
        row.extend_from_slice(&scope.to_le_bytes());
        row.extend_from_slice(&location_id.to_le_bytes());
        row.extend_from_slice(&state_id.to_le_bytes());
        row.extend_from_slice(&type_word.to_le_bytes());
        row.extend_from_slice(&level.to_le_bytes());
        row.extend_from_slice(&healthy_level.unwrap_or(u32::MAX).to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        row.extend_from_slice(&partial_health_raw.unwrap_or(0).to_le_bytes());
        assert_eq!(row.len(), BUILDING_ROW_LEN);
        row
    }

    fn full_building_row() -> Vec<u8> {
        building_row(SCOPE_PROVINCE, 2_068, 42, 0x1234, 5, Some(3), Some(7_000_000))
    }

    fn resource_row(state_id: u32, resource_word: u32, amount_raw: i32) -> Vec<u8> {
        let mut row = Vec::with_capacity(RESOURCE_ROW_LEN);
        row.extend_from_slice(&state_id.to_le_bytes());
        row.extend_from_slice(&resource_word.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(row.len(), RESOURCE_ROW_LEN);
        row
    }

    struct Railway {
        tag_db_id: u32,
        line_id: u32,
        queue_index: u32,
        current_segment_index: i32,
        route: Vec<u32>,
    }

    fn railway(tag_db_id: u32, line_id: u32, queue_index: u32) -> Railway {
        Railway {
            tag_db_id,
            line_id,
            queue_index,
            current_segment_index: 1,
            route: vec![11, 22, 33],
        }
    }

    fn railway_row_bytes(r: &Railway, route_start: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(RAILWAY_TRAILER_ROW_LEN);
        row.extend_from_slice(&r.tag_db_id.to_le_bytes());
        row.extend_from_slice(&r.line_id.to_le_bytes());
        row.extend_from_slice(&r.queue_index.to_le_bytes());
        row.extend_from_slice(&2i32.to_le_bytes()); // active_factories
        row.extend_from_slice(&10i32.to_le_bytes()); // cost_raw
        row.extend_from_slice(&5i32.to_le_bytes()); // produced_raw
        row.extend_from_slice(&60_000_000u32.to_le_bytes()); // created_date_raw
        row.extend_from_slice(&1u32.to_le_bytes()); // base_level
        row.extend_from_slice(&3u32.to_le_bytes()); // target_level
        row.extend_from_slice(&r.current_segment_index.to_le_bytes());
        row.extend_from_slice(&1_500_000i64.to_le_bytes()); // remaining_hours_raw
        row.extend_from_slice(&route_start.to_le_bytes());
        row.extend_from_slice(&(r.route.len() as u32).to_le_bytes());
        row.extend_from_slice(&1u32.to_le_bytes()); // flags: completion
        row.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(row.len(), RAILWAY_TRAILER_ROW_LEN);
        row
    }

    fn railway_trailer(lines: &[Railway]) -> Vec<u8> {
        let route_count: usize = lines.iter().map(|r| r.route.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(&RAILWAY_TRAILER_MAGIC);
        out.extend_from_slice(&RAILWAY_TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(&(RAILWAY_TRAILER_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(lines.len() as u32).to_le_bytes());
        out.extend_from_slice(&(route_count as u32).to_le_bytes());
        out.extend_from_slice(&(RAILWAY_TRAILER_ROW_LEN as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut start = 0u32;
        for line in lines {
            out.extend_from_slice(&railway_row_bytes(line, start));
            start += line.route.len() as u32;
        }
        for line in lines {
            for province in &line.route {
                out.extend_from_slice(&province.to_le_bytes());
            }
        }
        out
    }

    struct Payload {
        status_flags: u32,
        source_flags: u32,
        game_hour_raw: i32,
        construction: Vec<Vec<u8>>,
        buildings: Vec<Vec<u8>>,
        resources: Vec<Vec<u8>>,
        railway: Option<Vec<Railway>>,
    }

    /// `COMPACT_STATUS_PARTIAL_DATA` / `COMPACT_STATUS_DELTA_DATA`,
    /// `compact_contract/src/lib.rs`. Restated here for the same reason as the
    /// rest of the payload constants; the measurement bin asserts both.
    const STATUS_PARTIAL_DATA: u32 = 1 << 9;
    const STATUS_DELTA_DATA: u32 = 1 << 10;

    impl Payload {
        fn clean() -> Self {
            Self {
                status_flags: 0,
                source_flags: SOURCE_FLAG_DLL_NATIVE,
                game_hour_raw: 5,
                construction: vec![full_construction_row()],
                buildings: vec![full_building_row()],
                resources: vec![resource_row(42, 9, 1_234)],
                railway: Some(vec![railway(7, 99, 1)]),
            }
        }

        fn bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&MAGIC);
            out.extend_from_slice(&VERSION.to_le_bytes());
            out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
            out.extend_from_slice(&(DOMAIN as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&self.status_flags.to_le_bytes());
            out.extend_from_slice(&self.game_hour_raw.to_le_bytes());
            out.extend_from_slice(&(self.construction.len() as u32).to_le_bytes());
            out.extend_from_slice(&(self.buildings.len() as u32).to_le_bytes());
            out.extend_from_slice(&(self.resources.len() as u32).to_le_bytes());
            out.extend_from_slice(&(CONSTRUCTION_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&(BUILDING_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&(RESOURCE_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&11u32.to_le_bytes()); // observed_country_roots
            out.extend_from_slice(&22u32.to_le_bytes()); // observed_state_roots
            out.extend_from_slice(&33u32.to_le_bytes()); // observed_province_roots
            out.extend_from_slice(&self.source_flags.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            assert_eq!(out.len(), HEADER_LEN);
            for row in &self.construction {
                out.extend_from_slice(row);
            }
            for row in &self.buildings {
                out.extend_from_slice(row);
            }
            for row in &self.resources {
                out.extend_from_slice(row);
            }
            if let Some(lines) = &self.railway {
                out.extend_from_slice(&railway_trailer(lines));
            }
            out
        }
    }

    fn roundtrips(collection: &Collection, record: &Record) {
        let mut bytes = Vec::new();
        record.encode(collection, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(collection, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(&back, record, "decode reproduces the record");
        let mut reencoded = Vec::new();
        back.encode(collection, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    // --- Step 8.1: the schema is well-formed ---------------------------------

    #[test]
    fn the_schema_is_valid() {
        s().validate(grid().log2())
            .expect("d48 schema is well-formed");
    }

    // --- Step 8.5: shape and key --------------------------------------------

    #[test]
    fn every_collection_is_a_keyed_set_with_the_folds_own_key() {
        let schema = s();
        assert_eq!(schema.collections.len(), COLLECTION_COUNT);
        for collection in &schema.collections {
            assert_eq!(
                collection.shape,
                Shape::KeyedSet,
                "collection {} is a keyed set",
                collection.collection_id
            );
            for k in collection.key_fields() {
                assert!(
                    k.kind.legal_as_key(),
                    "field {} is a legal key kind",
                    k.field_id
                );
                assert!(!k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none());
            }
        }

        // `world_fold.rs::domain48_construction_key`.
        let construction = c(CONSTRUCTION_INDEX);
        let keys: Vec<(u32, Kind)> = construction
            .key_fields()
            .iter()
            .map(|f| (f.field_id, f.kind))
            .collect();
        assert_eq!(
            keys,
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::LINE_ID, Kind::Unsigned)
            ]
        );

        // `world_fold.rs::domain48_building_key` — four components, not the gate's three.
        let buildings = c(BUILDINGS_INDEX);
        let keys: Vec<(u32, Kind)> = buildings
            .key_fields()
            .iter()
            .map(|f| (f.field_id, f.kind))
            .collect();
        assert_eq!(
            keys,
            vec![
                (field::SCOPE, Kind::Unsigned),
                (field::LOCATION_ID, Kind::ContentId),
                (field::STATE_ID, Kind::ContentId),
                (field::TYPE_ID, Kind::ContentId),
            ]
        );

        // `world_fold.rs::domain48_resource_identity_key`.
        let resources = c(STATE_RESOURCES_INDEX);
        let keys: Vec<(u32, Kind)> = resources
            .key_fields()
            .iter()
            .map(|f| (f.field_id, f.kind))
            .collect();
        assert_eq!(
            keys,
            vec![
                (field::STATE_ID, Kind::ContentId),
                (field::RESOURCE_ID, Kind::ContentId)
            ]
        );

        // The recorded railway decision: identity, not position.
        let railway = c(RAILWAY_CONSTRUCTION_INDEX);
        let keys: Vec<u32> = railway.key_fields().iter().map(|f| f.field_id).collect();
        assert_eq!(keys, vec![field::TAG_DB_ID, field::LINE_ID]);
        assert!(
            railway
                .value_fields()
                .any(|f| f.field_id == field::QUEUE_INDEX),
            "the production-vector position rides beside the key as a value field"
        );

        // The route's positional key, parent first.
        let route = c(RAILWAY_ROUTE_INDEX);
        let keys: Vec<u32> = route.key_fields().iter().map(|f| f.field_id).collect();
        assert_eq!(
            keys,
            vec![field::TAG_DB_ID, field::LINE_ID, field::ROUTE_ORDINAL]
        );
    }

    #[test]
    fn the_declared_capabilities_are_exactly_what_the_mappers_emit() {
        // Absent is never emitted anywhere in d48: every concept applies to
        // every row of its collection, so nothing declares `.optional()`.
        for collection in &s().collections {
            assert_eq!(
                collection.optional_count(),
                0,
                "collection {} declares no optional field",
                collection.collection_id
            );
        }

        let construction = c(CONSTRUCTION_INDEX);
        let unknown: Vec<u32> = construction
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown,
            vec![
                field::TEMPLATE_ID,
                field::TARGET_STATE_ID,
                field::TARGET_LOCATION_ID,
                field::TARGET_SCOPE,
                field::REPAIR_LEVELS_REMAINING,
                field::CREATED_DATE_RAW,
            ]
        );
        assert!(construction.value_fields().all(|f| !f.nullable));

        let buildings = c(BUILDINGS_INDEX);
        let unknown: Vec<u32> = buildings
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown,
            vec![field::HEALTHY_LEVEL, field::PARTIAL_HEALTH_RAW]
        );
        assert!(buildings.value_fields().all(|f| !f.nullable));

        // `state_resources` and `railway_route` declare NO capability at all:
        // `amount_raw` and the route province are ungated words the decoder
        // requires to be in range, so neither can reach an empty state. Step
        // 8.3's per-capability cases therefore do not apply to them, which is
        // recorded here rather than silently missing.
        for index in [STATE_RESOURCES_INDEX, RAILWAY_ROUTE_INDEX] {
            let collection = c(index);
            assert!(collection
                .value_fields()
                .all(|f| !f.nullable && !f.unknown_capable && !f.optional));
        }

        // The railway table declares exactly one capability, and it is Null:
        // `-1` is the engine's explicit no-current-edge sentinel, not a failed
        // read (`lib.rs:315`).
        let railway = c(RAILWAY_CONSTRUCTION_INDEX);
        let nullable: Vec<u32> = railway
            .value_fields()
            .filter(|f| f.nullable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(nullable, vec![field::CURRENT_SEGMENT_INDEX]);
        assert!(railway.value_fields().all(|f| !f.unknown_capable));
    }

    #[test]
    fn the_content_ids_are_enumerated_for_the_section_13_retrofit_sweep() {
        // The §5 assignment now exists, but every one of these remains a raw
        // cast until this mapper receives the file-global resolver.
        // Asserting the list is what lets the sweep tell "checked" from "not
        // checked" (d35's `no_field_is_a_content_id`, inverted).
        let mut value_ids = Vec::new();
        let mut key_ids = Vec::new();
        for collection in &s().collections {
            for f in &collection.fields {
                if matches!(f.kind, Kind::ContentId) {
                    if f.is_key() {
                        key_ids.push((collection.collection_id, f.field_id));
                    } else {
                        value_ids.push((collection.collection_id, f.field_id));
                    }
                }
            }
        }
        // ContentId KEYS are the heavier retrofit class: a manifest remap
        // reorders keys and rewrites every baseline's byte layout.
        assert_eq!(
            key_ids,
            vec![
                (CONSTRUCTION, field::TAG_DB_ID),
                (BUILDINGS, field::LOCATION_ID),
                (BUILDINGS, field::STATE_ID),
                (BUILDINGS, field::TYPE_ID),
                (STATE_RESOURCES, field::STATE_ID),
                (STATE_RESOURCES, field::RESOURCE_ID),
                (RAILWAY_CONSTRUCTION, field::TAG_DB_ID),
                (RAILWAY_ROUTE, field::TAG_DB_ID),
            ]
        );
        assert_eq!(
            value_ids,
            vec![
                (CONSTRUCTION, field::TEMPLATE_ID),
                (CONSTRUCTION, field::TARGET_STATE_ID),
                (CONSTRUCTION, field::TARGET_LOCATION_ID),
                (RAILWAY_ROUTE, field::ROUTE_PROVINCE_ID),
            ]
        );
    }

    // --- Step 8.4: quantization ---------------------------------------------

    #[test]
    fn no_field_is_continuous_so_no_quantization_is_declared() {
        // Step 4 governs `Continuous` fields and d48 declares none: every
        // measured quantity here is a raw fixed-point integer the fold emits
        // verbatim (`cost_raw`, `produced_raw`, `amount_raw`,
        // `partial_health_raw`, `remaining_hours_raw`), so carrying the integer
        // is exact for every value the decoder admits and needs no precision
        // band at all. The "quantization saves bytes" comparison therefore has
        // nothing to compare.
        for collection in &s().collections {
            for f in collection.fields.iter() {
                assert!(
                    !matches!(f.kind, Kind::Continuous),
                    "field {} is not continuous",
                    f.field_id
                );
                assert_eq!(
                    f.quant,
                    Quant::None,
                    "field {} declares no quant",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers. `created_date_raw` is a
        // fixed creation stamp and `remaining_hours_raw` moves with construction
        // progress, not with the tick, so `frame_tick` never enters a value.
        for collection in &s().collections {
            assert!(collection.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    // --- Step 8.2: byte-identical round trips --------------------------------

    #[test]
    fn a_full_row_of_every_collection_roundtrips_byte_for_byte() {
        let schema = s();
        roundtrips(
            &schema.collections[CONSTRUCTION_INDEX],
            &record_from_construction_row(
                &schema.collections[CONSTRUCTION_INDEX],
                &full_construction_row(),
                0,
                false,
            )
            .expect("maps"),
        );
        roundtrips(
            &schema.collections[BUILDINGS_INDEX],
            &record_from_building_row(&schema.collections[BUILDINGS_INDEX], &full_building_row(), 0)
                .expect("maps"),
        );
        roundtrips(
            &schema.collections[STATE_RESOURCES_INDEX],
            &record_from_resource_row(
                &schema.collections[STATE_RESOURCES_INDEX],
                &resource_row(42, 9, 1_234),
                0,
            )
            .expect("maps"),
        );
        let line = railway(7, 99, 1);
        roundtrips(
            &schema.collections[RAILWAY_CONSTRUCTION_INDEX],
            &record_from_railway_row(
                &schema.collections[RAILWAY_CONSTRUCTION_INDEX],
                &railway_row_bytes(&line, 0),
                0,
            )
            .expect("maps"),
        );
        roundtrips(
            &schema.collections[RAILWAY_ROUTE_INDEX],
            &record_from_railway_route_row(
                &schema.collections[RAILWAY_ROUTE_INDEX],
                &42u32.to_le_bytes(),
                0,
            )
            .expect("maps"),
        );
    }

    #[test]
    fn a_fully_gated_off_construction_row_roundtrips_byte_for_byte() {
        let collection = c(CONSTRUCTION_INDEX);
        let record =
            record_from_construction_row(&collection, &gated_off_construction_row(), 0, false)
                .expect("maps");
        roundtrips(&collection, &record);
    }

    #[test]
    fn a_null_current_segment_roundtrips_byte_for_byte() {
        let collection = c(RAILWAY_CONSTRUCTION_INDEX);
        let mut line = railway(7, 99, 1);
        line.current_segment_index = -1;
        let record =
            record_from_railway_row(&collection, &railway_row_bytes(&line, 0), 0).expect("maps");
        assert_eq!(
            value_of(&collection, &record, field::CURRENT_SEGMENT_INDEX),
            Value::Null
        );
        roundtrips(&collection, &record);
    }

    // --- Step 8.3 + 8.6: every field's offset, and every declared state ------

    #[test]
    fn every_construction_field_reads_its_own_offset() {
        let collection = c(CONSTRUCTION_INDEX);
        // Distinct values at every offset, so a transposition cannot pass.
        let row = construction_row(
            7,                 // +0  tag_db_id (key)
            1_234,             // +4  line_id (key)
            Some(555),         // +8  template_id
            3,                 // +12 active_factories
            500_000,           // +16 cost_raw
            250_000,           // +20 produced_raw
            2,                 // +24 priority
            Some(42),          // +28 target_state_id
            Some(60_000_000),  // +36 created_date_raw
            Some(42),          // +40 target_location_id
            Some(SCOPE_STATE), // +44 target_scope (state => location == state)
            Some(4),           // +48 repair_levels_remaining
        );
        let record = record_from_construction_row(&collection, &row, 0, false).expect("maps");
        assert_eq!(
            value_of(&collection, &record, field::PRIORITY),
            Value::Unsigned(2)
        );
        assert_eq!(
            value_of(&collection, &record, field::TEMPLATE_ID),
            Value::ContentId(555)
        );
        assert_eq!(
            value_of(&collection, &record, field::ACTIVE_FACTORIES),
            Value::Unsigned(3)
        );
        assert_eq!(
            value_of(&collection, &record, field::COST_RAW),
            Value::Unsigned(500_000)
        );
        assert_eq!(
            value_of(&collection, &record, field::PRODUCED_RAW),
            Value::Unsigned(250_000)
        );
        assert_eq!(
            value_of(&collection, &record, field::TARGET_STATE_ID),
            Value::ContentId(42)
        );
        assert_eq!(
            value_of(&collection, &record, field::TARGET_LOCATION_ID),
            Value::ContentId(42)
        );
        assert_eq!(
            value_of(&collection, &record, field::TARGET_SCOPE),
            Value::Enum(scope_code::STATE)
        );
        assert_eq!(
            value_of(&collection, &record, field::REPAIR_LEVELS_REMAINING),
            Value::Unsigned(4)
        );
        assert_eq!(
            value_of(&collection, &record, field::CREATED_DATE_RAW),
            Value::Unsigned(60_000_000)
        );
        assert_eq!(
            key_from_construction_row(&row).expect("key"),
            construction_key(7, 1_234)
        );

        // The province scope code, so the Enum is exercised at both codes.
        let province =
            record_from_construction_row(&collection, &full_construction_row(), 0, false)
                .expect("maps");
        assert_eq!(
            value_of(&collection, &province, field::TARGET_SCOPE),
            Value::Enum(scope_code::PROVINCE)
        );
        assert_eq!(
            value_of(&collection, &province, field::TARGET_LOCATION_ID),
            Value::ContentId(2_068)
        );
    }

    #[test]
    fn a_real_zero_is_a_value_and_a_clear_gate_is_unknown() {
        let collection = c(CONSTRUCTION_INDEX);
        // Real zeros under SET gates — a line that is not a repair reads 0 with
        // the flag set (`lib.rs:625`-`:627`), which is a measured Zero.
        let zeros = construction_row(
            7,
            1_234,
            Some(555),
            0,
            0,
            0,
            0,
            Some(42),
            Some(1),
            Some(42),
            Some(SCOPE_STATE),
            Some(0),
        );
        let record = record_from_construction_row(&collection, &zeros, 0, false).expect("maps");
        for id in [
            field::PRIORITY,
            field::ACTIVE_FACTORIES,
            field::COST_RAW,
            field::PRODUCED_RAW,
            field::REPAIR_LEVELS_REMAINING,
        ] {
            assert_eq!(
                value_of(&collection, &record, id),
                Value::Unsigned(0),
                "field {id} is a real zero, never an empty state"
            );
        }

        // Every clear gate is Unknown — never Null, never Absent.
        let record =
            record_from_construction_row(&collection, &gated_off_construction_row(), 0, false)
                .expect("maps");
        for id in [
            field::TEMPLATE_ID,
            field::TARGET_STATE_ID,
            field::TARGET_LOCATION_ID,
            field::TARGET_SCOPE,
            field::REPAIR_LEVELS_REMAINING,
            field::CREATED_DATE_RAW,
        ] {
            assert_eq!(
                value_of(&collection, &record, id),
                Value::Unknown,
                "field {id} is Unknown on a clear gate"
            );
        }
    }

    #[test]
    fn the_frame_flag_makes_target_state_unknown_for_every_row() {
        // The d17-shaped input: a frame bit decides one field's answer for every
        // row, INCLUDING a row the accumulator retained with a real value
        // (`world_fold.rs::fold_construction_buildings_state_damage`).
        let collection = c(CONSTRUCTION_INDEX);
        let row = full_construction_row();
        let available = record_from_construction_row(&collection, &row, 0, false).expect("maps");
        assert_eq!(
            value_of(&collection, &available, field::TARGET_STATE_ID),
            Value::ContentId(42)
        );
        let unavailable = record_from_construction_row(&collection, &row, 0, true).expect("maps");
        assert_eq!(
            value_of(&collection, &unavailable, field::TARGET_STATE_ID),
            Value::Unknown
        );
        // ...and the fold gates the other two thirds of the promoted tuple on
        // the row's own option only, not on the frame bit
        // (`world_fold.rs::fold_construction_buildings_state_damage`). Mirrored, not tidied.
        assert_eq!(
            value_of(&collection, &unavailable, field::TARGET_LOCATION_ID),
            Value::ContentId(2_068)
        );
        assert_eq!(
            value_of(&collection, &unavailable, field::TARGET_SCOPE),
            Value::Enum(scope_code::PROVINCE)
        );
    }

    #[test]
    fn every_building_field_reads_its_own_offset() {
        let collection = c(BUILDINGS_INDEX);
        let row = building_row(
            SCOPE_PROVINCE,
            2_068,
            42,
            0x1234,
            5,
            Some(3),
            Some(7_000_000),
        );
        let record = record_from_building_row(&collection, &row, 0).expect("maps");
        assert_eq!(
            value_of(&collection, &record, field::LEVEL),
            Value::Unsigned(5)
        );
        assert_eq!(
            value_of(&collection, &record, field::HEALTHY_LEVEL),
            Value::Unsigned(3)
        );
        assert_eq!(
            value_of(&collection, &record, field::PARTIAL_HEALTH_RAW),
            Value::Unsigned(7_000_000)
        );
        assert_eq!(
            key_from_building_row(&row).expect("key"),
            building_key(SCOPE_PROVINCE, 2_068, 42, 0x1234)
        );

        // Both clear gates are Unknown: an incomplete per-type accumulation is a
        // read failure, not a relation pointing at nothing (`lib.rs:1874`-`:1895`).
        let gated = building_row(SCOPE_STATE, 42, 42, 0x1234, 5, None, None);
        let record = record_from_building_row(&collection, &gated, 0).expect("maps");
        assert_eq!(
            value_of(&collection, &record, field::HEALTHY_LEVEL),
            Value::Unknown
        );
        assert_eq!(
            value_of(&collection, &record, field::PARTIAL_HEALTH_RAW),
            Value::Unknown
        );
        // A real zero under a set gate stays a zero.
        let undamaged = building_row(SCOPE_STATE, 42, 42, 0x1234, 5, Some(5), Some(0));
        let record = record_from_building_row(&collection, &undamaged, 0).expect("maps");
        assert_eq!(
            value_of(&collection, &record, field::PARTIAL_HEALTH_RAW),
            Value::Unsigned(0)
        );
    }

    #[test]
    fn every_resource_and_railway_field_reads_its_own_offset() {
        let schema = s();
        let collection = &schema.collections[STATE_RESOURCES_INDEX];
        let row = resource_row(42, 9, 1_234);
        let record = record_from_resource_row(collection, &row, 0).expect("maps");
        assert_eq!(
            value_of(collection, &record, field::AMOUNT_RAW),
            Value::Unsigned(1_234)
        );
        assert_eq!(key_from_resource_row(&row).expect("key"), resource_key(42, 9));

        let collection = &schema.collections[RAILWAY_CONSTRUCTION_INDEX];
        let line = railway(7, 99, 3);
        let row = railway_row_bytes(&line, 0);
        let record = record_from_railway_row(collection, &row, 0).expect("maps");
        assert_eq!(
            value_of(collection, &record, field::QUEUE_INDEX),
            Value::Unsigned(3)
        );
        assert_eq!(
            value_of(collection, &record, field::ACTIVE_FACTORIES),
            Value::Unsigned(2)
        );
        assert_eq!(
            value_of(collection, &record, field::COST_RAW),
            Value::Unsigned(10)
        );
        assert_eq!(
            value_of(collection, &record, field::PRODUCED_RAW),
            Value::Unsigned(5)
        );
        assert_eq!(
            value_of(collection, &record, field::CREATED_DATE_RAW),
            Value::Unsigned(60_000_000)
        );
        assert_eq!(
            value_of(collection, &record, field::BASE_LEVEL),
            Value::Unsigned(1)
        );
        assert_eq!(
            value_of(collection, &record, field::TARGET_LEVEL),
            Value::Unsigned(3)
        );
        assert_eq!(
            value_of(collection, &record, field::COMPLETION_FLAG),
            Value::Bool(true)
        );
        assert_eq!(
            value_of(collection, &record, field::CURRENT_SEGMENT_INDEX),
            Value::Unsigned(1)
        );
        assert_eq!(
            value_of(collection, &record, field::REMAINING_HOURS_RAW),
            Value::Unsigned(1_500_000)
        );
        assert_eq!(key_from_railway_row(&row).expect("key"), railway_key(7, 99));

        let collection = &schema.collections[RAILWAY_ROUTE_INDEX];
        let record =
            record_from_railway_route_row(collection, &4_321u32.to_le_bytes(), 0).expect("maps");
        assert_eq!(
            value_of(collection, &record, field::ROUTE_PROVINCE_ID),
            Value::ContentId(4_321)
        );
    }

    #[test]
    fn every_header_word_reads_its_own_offset() {
        // A transposed header word mis-slices every row behind it, and no
        // round-trip or replay assertion can see that (recipe step 8.6).
        let mut payload = Payload::clean();
        payload.status_flags = STATUS_DELTA_DATA;
        payload.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        payload.game_hour_raw = -7;
        payload.construction = vec![
            construction_row(7, 1, None, 0, 0, 0, 0, None, None, None, None, None),
            construction_row(7, 2, None, 0, 0, 0, 0, None, None, None, None, None),
        ];
        payload.buildings = vec![
            building_row(SCOPE_STATE, 42, 42, 0x11, 1, None, None),
            building_row(SCOPE_STATE, 42, 42, 0x22, 1, None, None),
            building_row(SCOPE_STATE, 43, 43, 0x33, 1, None, None),
        ];
        payload.resources = vec![resource_row(42, 9, 1)];
        let bytes = payload.bytes();
        let header = peek_header(&bytes).expect("header");
        assert_eq!(header.status_flags, STATUS_DELTA_DATA);
        assert_eq!(header.game_hour_raw, -7);
        assert_eq!(header.construction_count, 2);
        assert_eq!(header.building_count, 3);
        assert_eq!(header.resource_count, 1);
        assert_eq!(header.observed_country_roots, 11);
        assert_eq!(header.observed_state_roots, 22);
        assert_eq!(header.observed_province_roots, 33);
        assert_eq!(
            header.source_flags,
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
        );
        assert!(header.target_state_unavailable());

        // And the slices the counts select.
        let (_, construction, buildings, resources, trailer) =
            split_payload(&bytes).expect("splits");
        assert_eq!(construction.len(), 2 * CONSTRUCTION_ROW_LEN);
        assert_eq!(buildings.len(), 3 * BUILDING_ROW_LEN);
        assert_eq!(resources.len(), RESOURCE_ROW_LEN);
        assert_eq!(&trailer[0..4], &RAILWAY_TRAILER_MAGIC);
    }

    #[test]
    fn the_header_rejects_every_word_the_fold_checks() {
        let good = Payload::clean().bytes();
        for (offset, byte) in [
            (0usize, b'X'), // magic
            (4, 9),         // version
            (6, 65),        // header_len
            (8, 47),        // domain id
            (10, 1),        // schema reserved
            (32, 51),       // construction row_len
            (34, 41),       // building row_len
            (36, 15),       // resource row_len
            (38, 1),        // row reserved
            (56, 1),        // header reserved tail
            (60, 1),        // header reserved tail
        ] {
            let mut bad = good.clone();
            bad[offset] = byte;
            assert!(
                peek_header(&bad).is_err(),
                "a corrupt word at +{offset} must be refused"
            );
        }
        assert!(
            peek_header(&good[..HEADER_LEN - 1]).is_err(),
            "a short header is refused"
        );
    }

    // --- Derived values that stay off the wire -------------------------------

    #[test]
    fn the_derived_values_are_functions_of_carried_state_and_are_not_fields() {
        let schema = s();
        // `damaged` == healthy_level < level (`world_fold.rs::domain48_building_rows_by_type`).
        let collection = &schema.collections[BUILDINGS_INDEX];
        for (level, healthy, damaged) in [(5u32, 3u32, true), (5, 5, false), (1, 0, true)] {
            let row = building_row(SCOPE_STATE, 42, 42, 0x1234, level, Some(healthy), Some(0));
            let parsed = parse_building_row(&row).expect("parses");
            assert_eq!(parsed.healthy_level.expect("present") < parsed.level, damaged);
            let record = record_from_building_row(collection, &row, 0).expect("maps");
            assert_eq!(
                value_of(collection, &record, field::HEALTHY_LEVEL)
                    != value_of(collection, &record, field::LEVEL),
                damaged,
                "damaged is recoverable from the two carried fields alone"
            );
        }

        // `is_repair` == repair_levels_remaining > 0 (`world_fold.rs::fold_construction_buildings_state_damage`).
        let collection = &schema.collections[CONSTRUCTION_INDEX];
        for (levels, is_repair) in [(0u32, false), (4, true)] {
            let row = construction_row(
                7,
                1,
                Some(555),
                0,
                0,
                0,
                0,
                Some(42),
                None,
                Some(42),
                Some(SCOPE_STATE),
                Some(levels),
            );
            let record = record_from_construction_row(collection, &row, 0, false).expect("maps");
            assert_eq!(
                value_of(collection, &record, field::REPAIR_LEVELS_REMAINING)
                    != Value::Unsigned(0),
                is_repair
            );
        }

        // `remaining_hours` == remaining_hours_raw / 100000 (`world_fold.rs::fold_construction_buildings_state_damage`).
        let collection = &schema.collections[RAILWAY_CONSTRUCTION_INDEX];
        let line = railway(7, 99, 1);
        let record =
            record_from_railway_row(collection, &railway_row_bytes(&line, 0), 0).expect("maps");
        let Value::Unsigned(raw) = value_of(collection, &record, field::REMAINING_HOURS_RAW) else {
            panic!("remaining_hours_raw is an unsigned word");
        };
        assert_eq!(raw as f64 / 100_000.0, 15.0);
    }

    // --- The three-way accumulator seed --------------------------------------

    fn clean_mode() -> FrameMode {
        FrameMode {
            delta: false,
            expected_partial: false,
        }
    }
    fn delta_mode() -> FrameMode {
        FrameMode {
            delta: true,
            expected_partial: false,
        }
    }
    fn partial_mode() -> FrameMode {
        FrameMode {
            delta: false,
            expected_partial: true,
        }
    }

    fn frame_of(payload: &Payload) -> Frame {
        decode_frame(&payload.bytes()).expect("decodes")
    }

    #[test]
    fn a_clean_frame_replaces_the_world_from_empty() {
        let mut fold = Domain48Fold::new();
        let mut first = Payload::clean();
        first.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        first.construction = vec![construction_row(
            7, 1, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        fold.apply(&frame_of(&first), clean_mode()).expect("folds");
        assert_eq!(fold.projected.construction.len(), 1);

        let mut second = Payload::clean();
        second.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        second.construction = vec![construction_row(
            7, 2, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        fold.apply(&frame_of(&second), clean_mode()).expect("folds");
        assert_eq!(
            fold.projected
                .construction
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![(7, 2)],
            "a clean frame discards the prior state (`world_fold.rs:20328`)"
        );
    }

    #[test]
    fn a_delta_frame_upserts_and_a_tombstone_removes() {
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        assert!(fold.projected.construction.contains_key(&(7, 1_234)));

        let mut delta = Payload::clean();
        delta.status_flags = STATUS_DELTA_DATA;
        delta.construction = vec![construction_tombstone_row(7, 1_234)];
        delta.buildings = vec![];
        delta.resources = vec![];
        fold.apply(&frame_of(&delta), delta_mode()).expect("folds");
        assert!(
            !fold.projected.construction.contains_key(&(7, 1_234)),
            "the DELETED bit in the identity word removes the row"
        );
        assert_eq!(
            fold.projected.buildings.len(),
            1,
            "a delta leaves untouched tables alone"
        );
    }

    #[test]
    fn a_delta_seeds_from_the_delta_baseline_not_the_projected_state() {
        // This is the named defect the fold's ten-line comment
        // (`world_fold.rs::fold_construction_buildings_state_damage`) exists to prevent: a tolerated
        // partial sparse-merges a phantom row into the PROJECTED state without
        // advancing the DELTA BASELINE, so a delta seeded from the projected
        // state would re-introduce it.
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");

        // A tolerated partial adds a row and must not advance the baseline.
        let mut partial = Payload::clean();
        partial.status_flags = STATUS_PARTIAL_DATA;
        partial.construction = vec![construction_row(
            7, 4_242, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        partial.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        partial.buildings = vec![];
        partial.resources = vec![];
        fold.apply(&frame_of(&partial), partial_mode())
            .expect("folds");
        assert!(
            fold.projected.construction.contains_key(&(7, 4_242)),
            "the partial sparse-merged into the projected state"
        );
        assert!(
            !fold.delta_baseline.construction.contains_key(&(7, 4_242)),
            "and must NOT have advanced the delta baseline (`world_fold.rs:20595`)"
        );
        assert!(
            fold.projected
                .buildings
                .contains_key(&(SCOPE_PROVINCE, 2_068, 42, 0x1234)),
            "a partial retains rows it omitted"
        );

        // The next delta seeds from the baseline, so the phantom is gone.
        let mut delta = Payload::clean();
        delta.status_flags = STATUS_DELTA_DATA;
        delta.construction = vec![];
        delta.buildings = vec![];
        delta.resources = vec![];
        fold.apply(&frame_of(&delta), delta_mode()).expect("folds");
        assert!(
            !fold.projected.construction.contains_key(&(7, 4_242)),
            "seeding the delta from the projected state is the defect; the baseline is the seed"
        );
        assert!(
            fold.projected.construction.contains_key(&(7, 1_234)),
            "and the baseline's own rows survive"
        );
    }

    #[test]
    fn a_tolerated_partial_must_not_replace_the_railway_trailer() {
        // `world_fold.rs::fold_construction_buildings_state_damage`: a partial cannot prove absence.
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        assert_eq!(fold.projected.railway.as_ref().map(Vec::len), Some(1));

        let mut partial = Payload::clean();
        partial.status_flags = STATUS_PARTIAL_DATA;
        partial.railway = Some(Vec::new());
        fold.apply(&frame_of(&partial), partial_mode())
            .expect("folds");
        assert_eq!(
            fold.projected.railway.as_ref().map(Vec::len),
            Some(1),
            "the last complete railway snapshot is retained"
        );

        // A clean or delta frame does replace it.
        let mut delta = Payload::clean();
        delta.status_flags = STATUS_DELTA_DATA;
        delta.railway = Some(Vec::new());
        fold.apply(&frame_of(&delta), delta_mode()).expect("folds");
        assert_eq!(fold.projected.railway.as_ref().map(Vec::len), Some(0));
    }

    #[test]
    fn a_frame_without_a_trailer_retains_only_what_its_seed_carried() {
        // `railway_construction_rows == None` is a legacy payload; the fold's
        // `if let Some(rows)` leaves the SEEDED set alone (`:20360`), so what
        // survives depends entirely on which of the three seeds the frame took.
        //
        // A delta seeds from the delta baseline, so the last complete snapshot
        // survives a trailer-less frame.
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        let mut legacy_delta = Payload::clean();
        legacy_delta.status_flags = STATUS_DELTA_DATA;
        legacy_delta.railway = None;
        fold.apply(&frame_of(&legacy_delta), delta_mode())
            .expect("folds");
        assert_eq!(fold.projected.railway.as_ref().map(Vec::len), Some(1));

        // A CLEAN frame seeds from empty, so a trailer-less clean frame leaves
        // the railway set unmeasured — `None`, not an empty set. That is the
        // fold's own behaviour and it is exactly why `to_states` refuses to
        // publish it as "measured, no rail lines".
        let mut legacy_clean = Payload::clean();
        legacy_clean.railway = None;
        fold.apply(&frame_of(&legacy_clean), clean_mode())
            .expect("folds");
        assert_eq!(fold.projected.railway, None);
        assert!(fold.projected.to_states(&s(), 0).is_err());
    }

    #[test]
    fn a_state_that_never_saw_a_trailer_is_refused_rather_than_published_as_empty() {
        // The recorded §8 divergence: a keyed set cannot say "not measured", so
        // publishing zero railway rows would state something false.
        let schema = s();
        let mut fold = Domain48Fold::new();
        let mut legacy = Payload::clean();
        legacy.railway = None;
        fold.apply(&frame_of(&legacy), clean_mode()).expect("folds");
        assert!(fold.projected.railway.is_none());
        let err = fold
            .projected
            .to_states(&schema, 0)
            .expect_err("an unmeasured collection is refused");
        assert!(err.contains("RLC1"), "{err}");
    }

    #[test]
    fn a_rejected_frame_leaves_both_accumulators_untouched() {
        // The fold's error contract is all-or-nothing (`world_fold.rs::fold_construction_buildings_state_damage`).
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        let before = fold.clone();
        let mut bad = Payload::clean();
        bad.construction = vec![construction_row(
            0, 1, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        assert!(
            decode_frame(&bad.bytes()).is_err(),
            "a zero tag_db_id is refused at decode, before anything is folded"
        );
        assert_eq!(fold, before);
    }

    // --- The RLC1 identity divergence ----------------------------------------

    #[test]
    fn a_trailer_repeating_one_railway_identity_is_refused() {
        // The recorded d32-shaped divergence: the trailer's canonical order is
        // `(tag_db_id, queue_index, line_id)` (`lib.rs:1360`), so two rows can
        // legally share `(tag_db_id, line_id)`. This schema's identity cannot
        // hold both, and mirroring would silently drop one.
        let mut payload = Payload::clean();
        payload.railway = Some(vec![railway(7, 99, 0), railway(7, 99, 1)]);
        let err = decode_frame(&payload.bytes()).expect_err("refused");
        assert!(err.contains("repeats railway identity"), "{err}");

        // Two lines that differ in `line_id` are fine, in queue order.
        let mut ok = Payload::clean();
        ok.railway = Some(vec![railway(7, 98, 0), railway(7, 99, 1)]);
        let frame = decode_frame(&ok.bytes()).expect("accepted");
        assert_eq!(frame.railway.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn the_route_ordinal_reproduces_the_array_element_for_element() {
        let schema = s();
        let mut payload = Payload::clean();
        let mut line = railway(7, 99, 0);
        line.route = vec![101, 102, 103, 104];
        payload.railway = Some(vec![line]);
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&payload), clean_mode()).expect("folds");
        let states = fold.projected.to_states(&schema, 0).expect("states");
        let route = &states[RAILWAY_ROUTE_INDEX];
        assert_eq!(route.len(), 4);
        for (ordinal, province) in [101u64, 102, 103, 104].into_iter().enumerate() {
            let record = route
                .get(&key_from_railway_route_row((7, 99), ordinal as u32))
                .expect("ordinal is in the route");
            assert_eq!(
                value_of(
                    &schema.collections[RAILWAY_ROUTE_INDEX],
                    record,
                    field::ROUTE_PROVINCE_ID
                ),
                Value::ContentId(province)
            );
        }
    }

    // --- The gate ------------------------------------------------------------

    #[test]
    fn the_gate_reproduces_the_lanes_own_verdict() {
        assert!(gate_frame(&frame_of(&Payload::clean())).ok);

        // No DLL-native source flag (`lib.rs:1422`).
        let mut bad = Payload::clean();
        bad.source_flags = 0;
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // A clean frame with no building rows (`lib.rs:1426`).
        let mut bad = Payload::clean();
        bad.buildings = vec![];
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // A clean frame with no resource rows (`lib.rs:1429`).
        let mut bad = Payload::clean();
        bad.resources = vec![];
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // target_state_id present while the frame badges it unavailable
        // (`lib.rs:1452`).
        let mut bad = Payload::clean();
        bad.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // target_state_id absent with no badge (`lib.rs:1446`).
        let mut bad = Payload::clean();
        bad.construction = vec![construction_row(
            7, 1, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // ...and the same rows WITH the badge pass.
        let mut good = Payload::clean();
        good.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        good.construction = vec![construction_row(
            7, 1, None, 0, 0, 0, 0, None, None, None, None, None,
        )];
        assert!(gate_frame(&frame_of(&good)).ok);

        // A duplicated construction identity (`lib.rs:1469`).
        let mut bad = Payload::clean();
        bad.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        bad.construction = vec![
            construction_row(7, 1, None, 0, 0, 0, 0, None, None, None, None, None),
            construction_row(7, 1, None, 0, 0, 0, 0, None, None, None, None, None),
        ];
        assert!(!gate_frame(&frame_of(&bad)).ok);

        // A duplicated resource identity (`lib.rs:1489`).
        let mut bad = Payload::clean();
        bad.resources = vec![resource_row(42, 9, 1), resource_row(42, 9, 2)];
        assert!(!gate_frame(&frame_of(&bad)).ok);
    }

    // --- Fail-closed row parsing ---------------------------------------------

    #[test]
    fn a_tombstone_is_a_removal_and_never_a_record() {
        let schema = s();
        let row = construction_tombstone_row(7, 1_234);
        assert!(record_from_construction_row(
            &schema.collections[CONSTRUCTION_INDEX],
            &row,
            0,
            false
        )
        .is_err());
        // The key of a tombstone is still the live key it removes.
        assert_eq!(
            key_from_construction_row(&row).expect("key"),
            construction_key(7, 1_234)
        );
    }

    #[test]
    fn a_mapper_called_with_the_wrong_collection_fails_instead_of_filling_nothing() {
        // `state_resources` and `railway_route` carry one value field each, so a
        // record built for the wrong collection would come out plausible rather
        // than empty. Every mapper checks its collection id first.
        let schema = s();
        assert!(record_from_resource_row(
            &schema.collections[CONSTRUCTION_INDEX],
            &resource_row(42, 9, 1),
            0
        )
        .is_err());
        assert!(record_from_railway_route_row(
            &schema.collections[RAILWAY_CONSTRUCTION_INDEX],
            &7u32.to_le_bytes(),
            0
        )
        .is_err());
        assert!(record_from_construction_row(
            &schema.collections[BUILDINGS_INDEX],
            &full_construction_row(),
            0,
            false
        )
        .is_err());
        assert!(record_from_building_row(
            &schema.collections[STATE_RESOURCES_INDEX],
            &full_building_row(),
            0
        )
        .is_err());
        assert!(record_from_railway_row(
            &schema.collections[CONSTRUCTION_INDEX],
            &railway_row_bytes(&railway(7, 99, 0), 0),
            0
        )
        .is_err());
    }

    #[test]
    fn every_construction_validator_the_fold_runs_is_reproduced() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "zero tag",
                construction_row(0, 1, None, 0, 0, 0, 0, None, None, None, None, None),
            ),
            (
                "tag out of range",
                construction_row(
                    MAX_COMPACT_OWNER_TAG_DB_ID + 1,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            ),
            (
                "zero line",
                construction_row(7, 0, None, 0, 0, 0, 0, None, None, None, None, None),
            ),
            (
                "zero template",
                construction_row(7, 1, Some(0), 0, 0, 0, 0, None, None, None, None, None),
            ),
            (
                "template out of range",
                construction_row(
                    7,
                    1,
                    Some(MAX_CONSTRUCTION_TEMPLATE_ID + 1),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            ),
            (
                "negative active_factories",
                construction_row(7, 1, None, -1, 0, 0, 0, None, None, None, None, None),
            ),
            (
                "active_factories out of range",
                construction_row(
                    7,
                    1,
                    None,
                    MAX_ACTIVE_FACTORIES + 1,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            ),
            (
                "negative cost",
                construction_row(7, 1, None, 0, -1, 0, 0, None, None, None, None, None),
            ),
            (
                "negative produced",
                construction_row(7, 1, None, 0, 0, -1, 0, None, None, None, None, None),
            ),
            (
                "zero target state",
                construction_row(7, 1, None, 0, 0, 0, 0, Some(0), None, None, None, None),
            ),
            (
                "target state out of range",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    Some(MAX_STATE_ID + 1),
                    None,
                    None,
                    None,
                    None,
                ),
            ),
            (
                "location without scope",
                construction_row(7, 1, None, 0, 0, 0, 0, Some(42), None, Some(42), None, None),
            ),
            (
                "scope without location",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    Some(42),
                    None,
                    None,
                    Some(SCOPE_STATE),
                    None,
                ),
            ),
            (
                "invalid scope",
                construction_row(7, 1, None, 0, 0, 0, 0, Some(42), None, Some(42), Some(3), None),
            ),
            (
                "state target mismatch",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    Some(41),
                    None,
                    Some(42),
                    Some(SCOPE_STATE),
                    None,
                ),
            ),
            (
                "province target lacks state",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(42),
                    Some(SCOPE_PROVINCE),
                    None,
                ),
            ),
            (
                "repair levels out of range",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    Some(42),
                    None,
                    Some(42),
                    Some(SCOPE_STATE),
                    Some(BUILDING_LEVEL_SANE_MAX + 1),
                ),
            ),
            (
                "zero created date",
                construction_row(7, 1, None, 0, 0, 0, 0, Some(42), Some(0), None, None, None),
            ),
            (
                "created date out of range",
                construction_row(
                    7,
                    1,
                    None,
                    0,
                    0,
                    0,
                    0,
                    Some(42),
                    Some(MAX_CONSTRUCTION_CREATED_DATE_RAW + 1),
                    None,
                    None,
                    None,
                ),
            ),
        ];
        for (name, row) in cases {
            assert!(
                parse_construction_row(&row).is_err(),
                "construction row case {name:?} must be refused"
            );
        }

        // "zero created date" is refused by the flag/word consistency check
        // rather than the range check, so pin the range check on its own too.
        let mut zero_date = full_construction_row();
        zero_date[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_construction_row(&zero_date).is_err());

        // A tombstone carrying live fields (`lib.rs:1181`, `:1649`).
        let mut tombstone = construction_tombstone_row(7, 5);
        tombstone[24..28].copy_from_slice(&9u32.to_le_bytes()); // priority
        assert!(parse_construction_row(&tombstone).is_err());

        // An unknown flag bit (`lib.rs:1176`).
        let mut unknown_flag = full_construction_row();
        unknown_flag[32..36].copy_from_slice(&(CONSTRUCTION_ROW_KNOWN_FLAGS | 1 << 6).to_le_bytes());
        assert!(parse_construction_row(&unknown_flag).is_err());

        // An ungated word carrying a value (`lib.rs:1193`-`:1214`).
        let mut ungated = gated_off_construction_row();
        ungated[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_construction_row(&ungated).is_err());
        let mut ungated = gated_off_construction_row();
        ungated[40..44].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_construction_row(&ungated).is_err());
        let mut ungated = gated_off_construction_row();
        ungated[44..48].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_construction_row(&ungated).is_err());
        let mut ungated = gated_off_construction_row();
        ungated[48..52].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_construction_row(&ungated).is_err());

        // A wrong-length slice.
        assert!(parse_construction_row(&full_construction_row()[..51]).is_err());
    }

    #[test]
    fn every_building_and_resource_validator_the_fold_runs_is_reproduced() {
        for (name, row) in [
            ("invalid scope", building_row(3, 42, 42, 1, 0, None, None)),
            (
                "zero location",
                building_row(SCOPE_STATE, 0, 0, 1, 0, None, None),
            ),
            (
                "state scope mismatch",
                building_row(SCOPE_STATE, 42, 43, 1, 0, None, None),
            ),
            (
                "zero state",
                building_row(SCOPE_PROVINCE, 42, 0, 1, 0, None, None),
            ),
            (
                "state out of range",
                building_row(SCOPE_PROVINCE, 42, MAX_STATE_ID + 1, 1, 0, None, None),
            ),
            (
                "zero type",
                building_row(SCOPE_STATE, 42, 42, 0, 0, None, None),
            ),
            (
                "level out of range",
                building_row(
                    SCOPE_STATE,
                    42,
                    42,
                    1,
                    BUILDING_LEVEL_SANE_MAX + 1,
                    None,
                    None,
                ),
            ),
            (
                "healthy above level",
                building_row(SCOPE_STATE, 42, 42, 1, 3, Some(4), None),
            ),
            (
                "partial above maximum",
                building_row(
                    SCOPE_STATE,
                    42,
                    42,
                    1,
                    1,
                    Some(0),
                    Some(CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL + 1),
                ),
            ),
            (
                "undamaged with partial",
                building_row(SCOPE_STATE, 42, 42, 1, 3, Some(3), Some(1)),
            ),
        ] {
            assert!(
                parse_building_row(&row).is_err(),
                "building row case {name:?} must be refused"
            );
        }
        // A building tombstone carrying live fields, and a clean one.
        assert!(parse_building_row(&building_row(
            SCOPE_STATE,
            42,
            42,
            1 | ROW_ID_DELETED,
            3,
            None,
            None
        ))
        .is_err());
        let tombstone = parse_building_row(&building_row(
            SCOPE_STATE,
            42,
            42,
            1 | ROW_ID_DELETED,
            0,
            None,
            None,
        ))
        .expect("parses");
        assert!(tombstone.deleted);
        assert_eq!(tombstone.type_id, 1);
        // An unknown building flag bit.
        let mut unknown_flag = full_building_row();
        unknown_flag[24..28].copy_from_slice(&(1u32 << 3).to_le_bytes());
        assert!(parse_building_row(&unknown_flag).is_err());
        assert!(parse_building_row(&full_building_row()[..39]).is_err());

        for (name, row) in [
            ("zero state", resource_row(0, 9, 0)),
            ("state out of range", resource_row(MAX_STATE_ID + 1, 9, 0)),
            ("zero resource", resource_row(42, 0, 0)),
            ("negative amount", resource_row(42, 9, -1)),
            (
                "amount out of range",
                resource_row(42, 9, STATE_RESOURCE_MAX_ABS_RAW + 1),
            ),
        ] {
            assert!(
                parse_resource_row(&row).is_err(),
                "resource row case {name:?} must be refused"
            );
        }
        assert!(parse_resource_row(&resource_row(42, 9 | ROW_ID_DELETED, 7)).is_err());
        let tombstone =
            parse_resource_row(&resource_row(42, 9 | ROW_ID_DELETED, 0)).expect("parses");
        assert!(tombstone.deleted);
        assert_eq!(tombstone.resource_id, 9);
        assert!(parse_resource_row(&resource_row(42, 9, 0)[..15]).is_err());

        // FOLD OBSERVATION, reproduced rather than tightened: the lane's
        // `resource_id == STATE_RESOURCE_NO_ID` rejection (`lib.rs:1815`,
        // `:1822`) is dead code. `STATE_RESOURCE_NO_ID` is `u32::MAX`, whose
        // top bit is `ROW_ID_DELETED`, and the check runs on the value AFTER
        // masking — so no masked id can ever equal it. A `u32::MAX` id word is
        // therefore a tombstone for resource `0x7fff_ffff` in the lane, and
        // here too. Mirroring is deliberate: refusing what the fold accepts is
        // the failure this recipe exists to prevent.
        let sentinel =
            parse_resource_row(&resource_row(42, STATE_RESOURCE_NO_ID, 0)).expect("parses");
        assert!(sentinel.deleted);
        assert_eq!(sentinel.resource_id, STATE_RESOURCE_NO_ID & !ROW_ID_DELETED);
    }

    #[test]
    fn the_railway_trailer_rejects_what_the_lane_rejects() {
        let ok = Payload::clean().bytes();
        let base = ok.len() - railway_trailer(&[railway(7, 99, 1)]).len();
        for (offset, bytes) in [
            (0usize, vec![b'X']),                    // magic
            (4, 2u16.to_le_bytes().to_vec()),        // version
            (6, 25u16.to_le_bytes().to_vec()),       // header_len
            (16, 63u16.to_le_bytes().to_vec()),      // row_len
            (18, 1u16.to_le_bytes().to_vec()),       // reserved
            (20, 1u32.to_le_bytes().to_vec()),       // reserved
        ] {
            let mut bad = ok.clone();
            bad[base + offset..base + offset + bytes.len()].copy_from_slice(&bytes);
            assert!(
                decode_frame(&bad).is_err(),
                "a corrupt RLC1 header word at +{offset} must be refused"
            );
        }
        // A truncated trailer, and a trailing byte.
        assert!(decode_frame(&ok[..ok.len() - 1]).is_err());
        let mut extra = ok.clone();
        extra.push(0);
        assert!(decode_frame(&extra).is_err());
        // A row invariant: `current_segment_index >= route_len - 1`.
        let mut payload = Payload::clean();
        let mut line = railway(7, 99, 1);
        line.current_segment_index = 2; // route_len 3 => must be < 2
        payload.railway = Some(vec![line]);
        assert!(decode_frame(&payload.bytes()).is_err());
        // ...and `-2` is not the sentinel.
        let mut payload = Payload::clean();
        let mut line = railway(7, 99, 1);
        line.current_segment_index = -2;
        payload.railway = Some(vec![line]);
        assert!(decode_frame(&payload.bytes()).is_err());
        // A route shorter than two provinces.
        let mut payload = Payload::clean();
        let mut line = railway(7, 99, 1);
        line.route = vec![11];
        line.current_segment_index = -1;
        payload.railway = Some(vec![line]);
        assert!(decode_frame(&payload.bytes()).is_err());
        // Rows out of canonical order.
        let mut payload = Payload::clean();
        payload.railway = Some(vec![railway(7, 99, 3), railway(7, 98, 1)]);
        assert!(decode_frame(&payload.bytes()).is_err());
    }

    /// `record_from_railway_row` is a public entry point, so it must be
    /// fail-closed on every invariant ONE ROW can decide — the same set
    /// `decode_railway_trailer` applies, minus the two that are relative to the
    /// trailer's shared route array. Sweeping the whole set rather than the one
    /// case that prompted it: a mapper that validated less than its own doc
    /// comment claimed would accept rows the fold refuses.
    #[test]
    fn the_railway_mapper_refuses_every_row_local_invariant_the_trailer_refuses() {
        let collection = c(RAILWAY_CONSTRUCTION_INDEX);
        let good = railway_row_bytes(&railway(7, 99, 1), 0);
        assert!(record_from_railway_row(&collection, &good, 0).is_ok());

        let cases: Vec<(&str, usize, Vec<u8>)> = vec![
            ("zero tag_db_id", 0, 0u32.to_le_bytes().to_vec()),
            ("zero line_id", 4, 0u32.to_le_bytes().to_vec()),
            ("negative active_factories", 12, (-1i32).to_le_bytes().to_vec()),
            (
                "active_factories out of range",
                12,
                (MAX_ACTIVE_FACTORIES + 1).to_le_bytes().to_vec(),
            ),
            ("negative cost_raw", 16, (-1i32).to_le_bytes().to_vec()),
            ("negative produced_raw", 20, (-1i32).to_le_bytes().to_vec()),
            ("zero created_date_raw", 24, 0u32.to_le_bytes().to_vec()),
            (
                "created_date_raw out of range",
                24,
                (MAX_CONSTRUCTION_CREATED_DATE_RAW + 1).to_le_bytes().to_vec(),
            ),
            (
                "base_level out of range",
                28,
                (BUILDING_LEVEL_SANE_MAX + 1).to_le_bytes().to_vec(),
            ),
            (
                "target_level below base_level",
                32,
                0u32.to_le_bytes().to_vec(),
            ),
            (
                "target_level out of range",
                32,
                (BUILDING_LEVEL_SANE_MAX + 1).to_le_bytes().to_vec(),
            ),
            ("route_len below two", 52, 1u32.to_le_bytes().to_vec()),
            (
                "route_len out of range",
                52,
                (MAX_RAILWAY_ROUTE_PROVINCES + 1).to_le_bytes().to_vec(),
            ),
            (
                "current_segment_index below the sentinel",
                36,
                (-2i32).to_le_bytes().to_vec(),
            ),
            (
                "current_segment_index at or past the last edge",
                36,
                2i32.to_le_bytes().to_vec(),
            ),
            (
                "negative remaining_hours_raw",
                40,
                (-1i64).to_le_bytes().to_vec(),
            ),
            ("unknown flag bit", 56, 2u32.to_le_bytes().to_vec()),
            ("nonzero reserved word", 60, 1u32.to_le_bytes().to_vec()),
        ];
        for (name, offset, bytes) in cases {
            let mut row = good.clone();
            row[offset..offset + bytes.len()].copy_from_slice(&bytes);
            assert!(
                record_from_railway_row(&collection, &row, 0).is_err(),
                "RLC1 mapper case {name:?} must be refused"
            );
            // ...and the trailer decoder refuses the very same row, which is
            // what keeps the mapper a subset rather than a second opinion.
            let mut payload = Payload::clean();
            payload.railway = Some(vec![railway(7, 99, 1)]);
            let mut bytes = payload.bytes();
            let trailer_base = bytes.len() - railway_trailer(&[railway(7, 99, 1)]).len();
            let row_base = trailer_base + RAILWAY_TRAILER_HEADER_LEN;
            bytes[row_base..row_base + RAILWAY_TRAILER_ROW_LEN].copy_from_slice(&row);
            assert!(
                decode_frame(&bytes).is_err(),
                "RLC1 trailer case {name:?} must be refused too"
            );
        }
    }

    #[test]
    fn the_public_boundary_never_panics_on_arbitrary_bytes() {
        // Fail-closed, total: no input of any length or content panics.
        let schema = s();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for len in [0usize, 1, 7, 40, 52, 63, 64, 65, 100, 116, 180, 300, 1_000] {
            for _ in 0..40 {
                let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
                let _ = peek_header(&bytes);
                let _ = split_payload(&bytes);
                let _ = decode_frame(&bytes);
                let _ = decode_railway_trailer(&bytes);
                let _ = parse_construction_row(&bytes);
                let _ = parse_building_row(&bytes);
                let _ = parse_resource_row(&bytes);
                let _ = key_from_construction_row(&bytes);
                let _ = key_from_building_row(&bytes);
                let _ = key_from_resource_row(&bytes);
                let _ = key_from_railway_row(&bytes);
                let _ = record_from_construction_row(
                    &schema.collections[CONSTRUCTION_INDEX],
                    &bytes,
                    0,
                    true,
                );
                let _ = record_from_building_row(&schema.collections[BUILDINGS_INDEX], &bytes, 0);
                let _ =
                    record_from_resource_row(&schema.collections[STATE_RESOURCES_INDEX], &bytes, 0);
                let _ = record_from_railway_row(
                    &schema.collections[RAILWAY_CONSTRUCTION_INDEX],
                    &bytes,
                    0,
                );
                let _ = record_from_railway_route_row(
                    &schema.collections[RAILWAY_ROUTE_INDEX],
                    &bytes,
                    0,
                );
            }
        }
        // A magic-correct payload with hostile counts must not overflow.
        for offset in [20usize, 24, 28] {
            let mut hostile = Payload::clean().bytes();
            hostile[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&hostile).is_err());
        }
        // ...and an RLC1 trailer whose counts overflow.
        let ok = Payload::clean().bytes();
        let base = ok.len() - railway_trailer(&[railway(7, 99, 1)]).len();
        for offset in [8usize, 12] {
            let mut hostile = ok.clone();
            hostile[base + offset..base + offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(decode_frame(&hostile).is_err());
        }
    }

    // --- The state boundary --------------------------------------------------

    #[test]
    fn the_five_states_are_built_in_schema_order_and_diff_cleanly() {
        let schema = s();
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        let states = fold.projected.to_states(&schema, 0).expect("states");
        assert_eq!(states.len(), COLLECTION_COUNT);
        assert_eq!(states[CONSTRUCTION_INDEX].len(), 1);
        assert_eq!(states[BUILDINGS_INDEX].len(), 1);
        assert_eq!(states[STATE_RESOURCES_INDEX].len(), 1);
        assert_eq!(states[RAILWAY_CONSTRUCTION_INDEX].len(), 1);
        assert_eq!(states[RAILWAY_ROUTE_INDEX].len(), 3);
        assert_eq!(fold.projected.rows(), 1 + 1 + 1 + 1 + 3);
        for (i, collection) in schema.collections.iter().enumerate() {
            assert!(diff(collection, &states[i], &states[i]).is_empty());
            let empty = KeyedState::new();
            assert_eq!(
                diff(collection, &empty, &states[i]).changed.len(),
                states[i].len()
            );
            assert_eq!(
                diff(collection, &states[i], &empty).tombstones.len(),
                states[i].len()
            );
        }

        // `fold_payload` is the same walk through one entry point, and it
        // commits what it projected.
        let mut fold = Domain48Fold::new();
        let again = fold_payload(
            &schema,
            &mut fold,
            &frame_of(&Payload::clean()),
            clean_mode(),
            0,
        )
        .expect("folds");
        assert_eq!(again, states);
        assert_eq!(fold.projected.construction.len(), 1);
        assert_eq!(fold.delta_baseline.construction.len(), 1);
    }

    /// Codex round-2 finding, class 6, carried in as its own regression test.
    ///
    /// `fold_payload` has TWO fallible steps — the fold, and the
    /// wire-representability check in `to_states` — and it used to commit
    /// between them, so the reviewer's input (a valid legacy clean `CBSD` frame
    /// with no `RLC1` trailer) advanced both accumulators and *then* returned
    /// `Err`. Swept over both steps and over all three frame modes, because the
    /// property is "no `Err` from this boundary leaves a mutation behind", not
    /// the one input that exposed it.
    #[test]
    fn fold_payload_is_all_or_nothing_over_both_of_its_fallible_steps() {
        let schema = s();

        // Step 2 — the reviewer's own trigger, from a virgin accumulator.
        let mut legacy = Payload::clean();
        legacy.railway = None;
        let mut fold = Domain48Fold::new();
        let before = fold.clone();
        let err = fold_payload(&schema, &mut fold, &frame_of(&legacy), clean_mode(), 0)
            .expect_err("an unmeasurable state is refused");
        assert!(err.contains("RLC1"), "{err}");
        assert_eq!(
            fold, before,
            "a refused projection must leave both accumulators untouched"
        );

        // ...and from a populated accumulator, where a wrongly-committed state
        // would corrupt every later delta rather than merely being empty.
        let mut fold = Domain48Fold::new();
        fold_payload(
            &schema,
            &mut fold,
            &frame_of(&Payload::clean()),
            clean_mode(),
            0,
        )
        .expect("folds");
        let before = fold.clone();
        assert!(fold_payload(&schema, &mut fold, &frame_of(&legacy), clean_mode(), 0).is_err());
        assert_eq!(fold, before, "the prior good state survives intact");
        // The state that survived is still the one the next delta must seed
        // from, which is the consequence the finding was really about.
        let mut delta = Payload::clean();
        delta.status_flags = STATUS_DELTA_DATA;
        delta.construction = vec![];
        delta.buildings = vec![];
        delta.resources = vec![];
        fold_payload(&schema, &mut fold, &frame_of(&delta), delta_mode(), 0).expect("folds");
        assert!(
            fold.projected.construction.contains_key(&(7, 1_234)),
            "the delta seeded from the uncorrupted baseline"
        );

        // Step 1 — a projection error must not commit either. The building-scope
        // check is the fold's own post-seed error path; `parse_building_row`
        // makes it unreachable from bytes, so it is driven through `project`
        // with a hand-built frame.
        let mut fold = Domain48Fold::new();
        fold_payload(
            &schema,
            &mut fold,
            &frame_of(&Payload::clean()),
            clean_mode(),
            0,
        )
        .expect("folds");
        let before = fold.clone();
        let mut bad = frame_of(&Payload::clean());
        bad.buildings[0].scope = 3;
        assert!(fold.project(&bad, clean_mode()).is_err());
        assert!(fold_payload(&schema, &mut fold, &bad, clean_mode(), 0).is_err());
        assert_eq!(fold, before);

        // And every mode, so a partial or delta cannot commit behind an `Err`
        // either.
        for mode in [clean_mode(), delta_mode(), partial_mode()] {
            let mut fold = Domain48Fold::new();
            fold_payload(
                &schema,
                &mut fold,
                &frame_of(&Payload::clean()),
                clean_mode(),
                0,
            )
            .expect("folds");
            let before = fold.clone();
            let mut trailerless = Payload::clean();
            trailerless.railway = None;
            trailerless.status_flags = if mode.delta {
                STATUS_DELTA_DATA
            } else if mode.expected_partial {
                STATUS_PARTIAL_DATA
            } else {
                0
            };
            // A delta or partial seeds from a state that already has a trailer,
            // so those two legitimately succeed; only the clean seed can reach
            // the refusal. Whichever way it goes, an `Err` must not mutate.
            if fold_payload(&schema, &mut fold, &frame_of(&trailerless), mode, 0).is_err() {
                assert_eq!(fold, before, "mode {mode:?} committed behind an Err");
            }
        }
    }

    #[test]
    fn a_frame_flag_flip_rewrites_every_construction_record() {
        // The consequence of the d17-shaped input: the flag is frame-level, so
        // flipping it changes `target_state_id` for every retained row and the
        // delta must carry all of them. This is measured, not assumed away.
        let schema = s();
        let mut fold = Domain48Fold::new();
        fold.apply(&frame_of(&Payload::clean()), clean_mode())
            .expect("folds");
        let before = fold.projected.to_states(&schema, 0).expect("states");

        let mut badged = Payload::clean();
        badged.status_flags = STATUS_DELTA_DATA;
        badged.source_flags =
            SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        badged.construction = vec![];
        badged.buildings = vec![];
        badged.resources = vec![];
        fold.apply(&frame_of(&badged), delta_mode()).expect("folds");
        let after = fold.projected.to_states(&schema, 0).expect("states");

        let d = diff(
            &schema.collections[CONSTRUCTION_INDEX],
            &before[CONSTRUCTION_INDEX],
            &after[CONSTRUCTION_INDEX],
        );
        assert_eq!(
            d.changed.len(),
            1,
            "the retained row's target_state_id became Unknown, so it changed"
        );
        assert!(d.tombstones.is_empty());
        assert!(
            diff(
                &schema.collections[BUILDINGS_INDEX],
                &before[BUILDINGS_INDEX],
                &after[BUILDINGS_INDEX]
            )
            .is_empty(),
            "and nothing else moved"
        );
    }
}
