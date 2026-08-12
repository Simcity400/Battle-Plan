//! d62 `hub_motorization_live` — exact per-node supply-hub motorization levels.
//!
//! Derived from `world_fold.rs:19248`-`:19311`
//! (`fold_hub_motorization`), its JSON emission at `:9187`-`:9220`, and the
// @world-fold-authority domain=62 functions=fold_hub_motorization lines=19248-19311 sha256=00f57454e997e08043a40a625ef9d2833c5c900c82cb22bbe8ae6ffef9c6f33f closure=4d37e6d1d7fef5637817dd1e99483b912ed3a44ec5e7832c406adca844367ea8
//! payload codec the fold calls, which for this domain lives **outside** the fold
//! in its own leaf crate: `extractor/native/lanes/hub_motorization_live/src/lib.rs`
//! (`decode_payload_rows` at `:264`, `read_row` at `:325`, `validate_row` at
//! `:340`). `extractor/documents/domains/d62_hub_motorization_live.md` carries the
//! exact-a729 structure law and one field-level fact used below (levels 1 and 2
//! are the only persisted values; 0 means "no entry" and is not stored).
//!
//! **Payload grammar** (`lib.rs:175`-`:196`, `:264`-`:323`): a 32-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `status_flags` u32 at
//! `+12`, `schema_version` u32 at `+16`, `total_settings_nodes` u32 at `+20`,
//! `row_len` u32 at `+24`, `total_map_entries` u32 at `+28` — followed by
//! `row_count` fixed 32-byte rows of eight little-endian u32 words. The header
//! carries its own `row_len` and `schema_version`, and the codec rejects any
//! payload whose `row_len != 32` or `schema_version != 2` (`lib.rs:284`-`:292`),
//! so the length never selects a row form: there is exactly one, and no legacy
//! short row exists.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id`    | raw | none (key 0; `1..=2048`, `lib.rs:341`) |
//! | `+4`  | u32 | `actor_tag_db_id`    | raw | none (key 2; `1..=2048`, `lib.rs:347`) |
//! | `+8`  | u32 | `province_id`        | raw | none (key 1; `1..=65535`, `lib.rs:353`) |
//! | `+12` | u32 | `state_id`           | raw | none (`1..=65535`, `lib.rs:359`) |
//! | `+16` | u32 | `record_index`       | raw | none |
//! | `+20` | u32 | `map_entry_index`    | raw | none |
//! | `+24` | u32 | `motorization_level` | raw | none (`1..=2`, `lib.rs:365`, `world_fold.rs::fold_hub_motorization`) |
//! | `+28` | u32 | `flags`              | —   | — (see below) |
//!
//! **No scale factor anywhere.** Every one of the eight words is an integer the
//! fold and the JSON emitter pass through unchanged (`world_fold.rs::insert_hub_motorization`).
//! The one number in this domain that looks like a fixed-point quantity —
//! `truck_fraction_raw`, 50 000 or 100 000 — is not a payload field at all: the
//! emitter computes it from the level with a two-armed conditional
//! (`world_fold.rs::insert_hub_motorization`), so it is a pure function of `motorization_level` and
//! carrying it would be a second encoding of the same fact.
//!
//! **No field can be absent, null or unknown, and the schema declares no
//! capability.** There is no gate anywhere in this row: `read_row` (`lib.rs:325`)
//! reads all eight words unconditionally, and `validate_row` (`lib.rs:340`)
//! range-rejects five of them outright. The row's `flags` word is not a gate
//! either — see below. A read that fails happens *earlier*, in the collector's
//! `derive_object_rows`, which returns `CcssMalformed` for the whole object
//! rather than emitting a row with a hole (`lib.rs:541`-`:698`); and a payload
//! that fails to decode marks the **domain-hour** unknown while retaining the
//! previous accumulator (`world_fold.rs:15926`-`:15935`, `:15947`, `:15962`,
//! `:15974`). That is an hour with no d62 state at all, not a row with an empty
//! field — §3's four states are per field, and this domain's rows only ever
//! carry values. Declaring a capability no mapper can emit would cost real bits
//! (§3 sizes the null and unknown bitmaps by the fields that declare them) and
//! assert a state that cannot occur.
//!
//! **The five flag bits are verification witnesses, not field gates, and are not
//! on the wire.** `validate_row` (`lib.rs:371`-`:381`) rejects a row that carries
//! any bit outside the five AND a row that is missing any of the five; either
//! rejection makes `decode_payload_rows` return `Err`, which the fold turns into
//! a domain-unknown hour that folds nothing (`world_fold.rs::fold_hub_motorization`).
//! So on the whole set of rows this schema can ever describe, `flags` is the
//! constant `0b11111` — it has no state to encode. The fold agrees in its own
//! output: it emits `"node_motorized": true` as a **literal**
//! (`world_fold.rs::insert_hub_motorization`) rather than reading `flags & ROW_FLAG_NODE_MOTORIZED`,
//! and emits no other flag-derived field. The mapper enforces the invariant the
//! omission rests on (any other flags word is an `Err`, never a coerced value),
//! and the grammar is version-pinned — a collector that gave the bits per-row
//! meaning would have to bump `SCHEMA_VERSION`, which [`split_payload`] rejects.
//! Corroborated empirically: across all 467 940 rows of
//! `runs/batch1/deepwar_t1/collector_stream.bin`, `flags` is `31` in every row.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock; `record_index` and `map_entry_index` are engine array positions, which
//! move when the engine reorders its node vector, not on a clock-linear rule.
//! The mapper still takes `frame_tick` so every domain's boundary has one shape.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 62;
pub const MOTORIZATION_ENTRIES: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;
    pub const PROVINCE_ID: u32 = 2;
    pub const ACTOR_TAG_DB_ID: u32 = 3;
    pub const STATE_ID: u32 = 4;
    pub const RECORD_INDEX: u32 = 5;
    pub const MAP_ENTRY_INDEX: u32 = 6;
    pub const MOTORIZATION_LEVEL: u32 = 7;
}

// --- Payload grammar, from `lanes/hub_motorization_live/src/lib.rs`. ----------
//
// The constants are mirrored here rather than imported: `wire_format` is the
// reference implementation of the spec and takes no dependency on a capture
// lane. Every one carries the line it was taken from, and the version pin below
// is what makes the mirror safe — a collector that changed the grammar must bump
// `SCHEMA_VERSION`, and `split_payload` refuses any other value. See the
// deliverable's open questions.

/// `PAYLOAD_HEADER_LEN`, `lib.rs:85`.
pub const HEADER_LEN: usize = 32;
/// `ROW_LEN`, `lib.rs:87`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 32;
/// `SCHEMA_VERSION`, `lib.rs:84`. `decode_payload_rows` (`:284`) rejects any
/// other value, so this pins the grammar the offsets below belong to.
pub const SCHEMA_VERSION: u32 = 2;

/// `MAX_COUNTRY_COUNT`, `lib.rs:65` — the bound `validate_row` applies to both
/// `owner_tag_db_id` and `actor_tag_db_id` (`:341`, `:347`). Zero is rejected too.
pub const MAX_COUNTRY_COUNT: u32 = 2_048;
/// `MAX_PROVINCE_ID`, `lib.rs:80` (`validate_row:353`). A REPRESENTATION
/// ENVELOPE, not a province count — see the producer's note; both spellings were
/// `20_000`, a guess at content scale, and are now the build's settled 65 535.
pub const MAX_PROVINCE_ID: u32 = 65_535;
/// `MAX_STATE_ID`, `lib.rs:81` (`validate_row:359`).
pub const MAX_STATE_ID: u32 = 65_535;

pub const ROW_FLAG_COUNTRY_VERIFIED: u32 = 1 << 0;
pub const ROW_FLAG_STRUCT_VERIFIED: u32 = 1 << 1;
pub const ROW_FLAG_NODE_MOTORIZED: u32 = 1 << 2;
pub const ROW_FLAG_ACTOR_VERIFIED: u32 = 1 << 3;
pub const ROW_FLAG_LEVEL_VERIFIED: u32 = 1 << 4;
/// `lib.rs:371`-`:381`: a row must carry **exactly** these five bits. Any extra
/// bit and any missing bit are both hard rejections, which is why this is one
/// constant and not a "known" mask plus a separate "required" mask.
pub const REQUIRED_FLAGS: u32 = ROW_FLAG_COUNTRY_VERIFIED
    | ROW_FLAG_STRUCT_VERIFIED
    | ROW_FLAG_NODE_MOTORIZED
    | ROW_FLAG_ACTOR_VERIFIED
    | ROW_FLAG_LEVEL_VERIFIED;

/// The lowest and highest motorization level the engine persists
/// (`lib.rs:365`, `world_fold.rs::fold_hub_motorization`, dossier "Persisted levels are exact").
pub const MIN_MOTORIZATION_LEVEL: u32 = 1;
pub const MAX_MOTORIZATION_LEVEL: u32 = 2;

/// §4 cardinality for `motorization_level`: **two** codes, not three.
///
/// The engine's own values are `1` (50 % truck fraction) and `2` (100 %); `0`
/// means "this actor has no entry for this node" and is **never persisted**
/// (dossier line 45, `lib.rs:657`-`:660`), which in a keyed set is the absence of
/// the row, not a code the row can hold. Declaring `enum_card(3)` to keep the raw
/// engine number would therefore make `Enum(0)` a decodable value meaning a level
/// the fold rejects (`world_fold.rs::fold_hub_motorization`) — a record that passes
/// `Record::validate` carrying data no capture can produce — and would cost two
/// packed bits per row instead of one (`record.rs:334`). The mapping is a
/// bijection onto the legal levels and is applied in exactly one place, by
/// [`level_to_code`] and [`code_to_level`].
pub const MOTORIZATION_LEVEL_CARDINALITY: u32 = 2;

/// Engine level (`1..=2`) to the stored §4 code (`0..=1`).
pub fn level_to_code(level: u32) -> core::result::Result<u32, String> {
    if !(MIN_MOTORIZATION_LEVEL..=MAX_MOTORIZATION_LEVEL).contains(&level) {
        return Err(format!(
            "hub_motorization row motorization_level {level} out of range"
        ));
    }
    Ok(level - MIN_MOTORIZATION_LEVEL)
}

/// The stored §4 code back to the engine level. `None` for a code the schema's
/// cardinality does not admit — the decoder rejects those before they get here,
/// and a reader that reaches this must not invent a level.
pub fn code_to_level(code: u32) -> Option<u32> {
    if code < MOTORIZATION_LEVEL_CARDINALITY {
        Some(code + MIN_MOTORIZATION_LEVEL)
    } else {
        None
    }
}

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // The key is the fold's own tuple, in the fold's own order:
        // `(owner_tag_db_id, province_id, actor_tag_db_id)` (`world_fold.rs::fold_hub_motorization`).
        // A duplicate is a rejected frame there (`world_fold.rs::fold_hub_motorization`), never a
        // last-writer-wins overwrite, which is what makes it a real identity.
        //
        // All three are `ContentId`: each is an id whose meaning a content
        // manifest must resolve — a country tag-db id and a province id are
        // whatever the mod set defines (§5), which is the deciding question, and
        // d20 classes both `owner` (country) and `location_province` the same
        // way. The §5 assignment now exists, but this mapper still casts the raw
        // u32 until its resolver retrofit; that is recorded in the deliverable, and a
        // ContentId **key** is the heavier retrofit class because key components
        // sort and delta-encode by raw value.
        Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
        Field::new(f::PROVINCE_ID, Kind::ContentId).key(1),
        Field::new(f::ACTOR_TAG_DB_ID, Kind::ContentId).key(2),
        // The supply node's state. Content-defined exactly as the province is,
        // and read from the same node record (`lib.rs:597`); not part of the
        // fold's key, so it rides as a value.
        Field::new(f::STATE_ID, Kind::ContentId),
        // Engine array positions: `record_index` is the node's index in the
        // CCSS node vector and `map_entry_index` the entry's index in that
        // node's motorization map (`lib.rs:666`-`:667`). They are neither
        // identity (the fold keys by owner/province/actor, not by these) nor
        // content (no manifest resolves an array slot), so §5 puts them at
        // `Unsigned`. Not `Monotone`: an index does not count anything and does
        // not usually rise — it is re-derived from the engine's current vector
        // order every frame.
        Field::new(f::RECORD_INDEX, Kind::Unsigned),
        Field::new(f::MAP_ENTRY_INDEX, Kind::Unsigned),
        // The one non-identity measurement in the domain, and a natural enum:
        // the fold constrains it to `1..=2` (`world_fold.rs::fold_hub_motorization`). See
        // `MOTORIZATION_LEVEL_CARDINALITY` for why the cardinality is 2.
        Field::new(f::MOTORIZATION_LEVEL, Kind::Enum).enum_card(MOTORIZATION_LEVEL_CARDINALITY),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: MOTORIZATION_ENTRIES,
            // Full replace each frame (`world_fold.rs::fold_hub_motorization` assigns the whole
            // fresh map), but the key triple is a stable identity that persists
            // across hours — measured churn on the real capture is under one
            // insert and one delete per hour against ~518 rows — and only a
            // keyed set has a delta grammar (§7.3).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 32-byte payload header (`lib.rs:175`-`:196`, `:275`-`:283`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub status_flags: u32,
    pub schema_version: u32,
    pub total_settings_nodes: u32,
    pub row_len: u32,
    pub total_map_entries: u32,
}

/// Split a d62 DATA payload into its header and its rows slice, applying the
/// codec's and the fold's **structural** checks (`lib.rs:268`-`:317`,
/// `world_fold.rs::fold_hub_motorization`). Per-row semantics stay in
/// [`record_from_row`].
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "hub_motorization payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        status_flags: u32_at(payload, 12),
        schema_version: u32_at(payload, 16),
        total_settings_nodes: u32_at(payload, 20),
        row_len: u32_at(payload, 24),
        total_map_entries: u32_at(payload, 28),
    };
    // `lib.rs:284`: the payload declares which grammar it is. Every offset in
    // this module belongs to version 2, so another version is not a payload this
    // mapper may read.
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "hub_motorization schema_version {} != {SCHEMA_VERSION}",
            header.schema_version
        ));
    }
    // `lib.rs:290`: the header also declares its own row stride.
    if header.row_len as usize != ROW_LEN {
        return Err(format!(
            "hub_motorization row_len {} != {ROW_LEN}",
            header.row_len
        ));
    }
    // Checked arithmetic: `row_count` is arbitrary input from a stream on disk,
    // and `row_count * ROW_LEN` overflows `usize` for a large enough word. The
    // codec saturates instead (`lib.rs:293`), which rejects the same payloads by
    // way of the length comparison; checked multiplication says so directly.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "hub_motorization row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "hub_motorization len {} != {expected}",
            payload.len()
        ));
    }
    // `lib.rs:312` and `world_fold.rs::fold_hub_motorization`: the codec counts one map entry per
    // emitted row and the fold counts them again, so the header's total is the
    // row count. A mismatch means rows were dropped between collection and
    // emission — the fold marks the hour unknown for exactly this.
    if u64::from(header.total_map_entries) != u64::from(header.row_count) {
        return Err(format!(
            "hub_motorization total_map_entries {} != row_count {}",
            header.total_map_entries, header.row_count
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The key of one compact row: the fold's `(owner_tag_db_id, province_id,
/// actor_tag_db_id)` triple (`world_fold.rs::fold_hub_motorization`), in the fold's order.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let (owner, province, actor) = key_fields_of_row(row)?;
    Ok(key(&[owner as u64, province as u64, actor as u64]))
}

/// The three key words with `validate_row`'s range checks: owner at
/// `lib.rs:341`, actor at `:347`, province at `:353` (`:341`-`:357` covers all
/// three).
/// Returned in **key order**, which is not row order: the row packs
/// `owner, actor, province` and the key is `(owner, province, actor)`.
pub fn key_fields_of_row(row: &[u8]) -> core::result::Result<(u32, u32, u32), String> {
    check_row_len(row)?;
    let owner = u32_at(row, 0);
    let actor = u32_at(row, 4);
    let province = u32_at(row, 8);
    check_range(owner, 1, MAX_COUNTRY_COUNT, "owner_tag_db_id")?;
    check_range(actor, 1, MAX_COUNTRY_COUNT, "actor_tag_db_id")?;
    check_range(province, 1, MAX_PROVINCE_ID, "province_id")?;
    Ok((owner, province, actor))
}

/// One compact row of the `motorization_entries` table, as the collector emits
/// it today, onto its stored Record. `row` is the raw 32-byte row slice (the
/// caller has already consumed the table header via [`split_payload`]); d62 has
/// exactly one row form, so length selects nothing and a wrong length is
/// rejected. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); d62 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `validate_row` (`lib.rs:340`) and
/// `fold_hub_motorization`: a row either of them would reject is an `Err` here,
/// never a coerced value. The codec accumulates every error and reports them
/// together; this returns the first, which is the same accept/reject decision.
/// Nothing in this function can panic on any 32-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d62 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // Key words: validated here too, because a row's key being well-formed is
    // part of the row being well-formed, even for a caller that wants only the
    // record. (`check_row_len` runs inside.)
    let _ = key_fields_of_row(row)?;

    let state_id = u32_at(row, 12);
    check_range(state_id, 1, MAX_STATE_ID, "state_id")?;

    // `lib.rs:365` and `world_fold.rs::fold_hub_motorization`. Rejected, never clamped: a level
    // outside `1..=2` is a row the fold refuses, and coercing it would publish
    // a motorization level the engine never persisted.
    let level_code = level_to_code(u32_at(row, 24))?;

    // `lib.rs:371`-`:381`. Both halves: an unknown bit means the row is not this
    // grammar, and a missing witness means the collector did not verify what the
    // row asserts. Either one makes the whole frame unfoldable upstream.
    let flags = u32_at(row, 28);
    if flags != REQUIRED_FLAGS {
        return Err(format!(
            "hub_motorization row flags {flags:#x} != the required {REQUIRED_FLAGS:#x}"
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::STATE_ID => Value::ContentId(state_id as u64),
            field::RECORD_INDEX => Value::Unsigned(u32_at(row, 16) as u64),
            field::MAP_ENTRY_INDEX => Value::Unsigned(u32_at(row, 20) as u64),
            field::MOTORIZATION_LEVEL => Value::Enum(level_code),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field
                // id that means something else.
                return Err(format!(
                    "hub_motorization has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

fn check_range(value: u32, min: u32, max: u32, field: &str) -> core::result::Result<(), String> {
    if value < min || value > max {
        return Err(format!(
            "hub_motorization row {field} {value} out of range {min}..={max}"
        ));
    }
    Ok(())
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "hub_motorization row length {} != {ROW_LEN}",
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

    /// A raw 32-byte row, built the way the lane packs one (`lib.rs:185`-`:194`)
    /// — in ROW order, which is not key order.
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        owner: u32,
        actor: u32,
        province: u32,
        state: u32,
        record_index: u32,
        map_entry_index: u32,
        level: u32,
        flags: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [
            owner,
            actor,
            province,
            state,
            record_index,
            map_entry_index,
            level,
            flags,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A row whose every word is a distinct number, so a transposed pair of
    /// offsets cannot pass any assertion made against it.
    fn full_row() -> Vec<u8> {
        raw_row(16, 17, 13071, 5, 1, 3, 2, REQUIRED_FLAGS)
    }

    fn payload_of(rows: &[Vec<u8>], status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes()); // total_settings_nodes
        payload.extend_from_slice(&(ROW_LEN as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // total_map_entries
        assert_eq!(payload.len(), HEADER_LEN);
        for row in rows {
            payload.extend_from_slice(row);
        }
        payload
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d62 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_folds_own_triple() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 3, "the fold keys by (owner, province, actor)");
        assert_eq!(
            keys.iter().map(|f| f.field_id).collect::<Vec<_>>(),
            vec![
                field::OWNER_TAG_DB_ID,
                field::PROVINCE_ID,
                field::ACTOR_TAG_DB_ID
            ],
            "key order is the fold's tuple order (`world_fold.rs:15883`)"
        );
        for f in &keys {
            assert_eq!(
                f.kind,
                Kind::ContentId,
                "field {} is a mod-set-defined id",
                f.field_id
            );
            assert!(f.kind.legal_as_key());
        }
        assert_eq!(c.fields.len(), 7, "3 key + 4 value fields");
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // Step 8 case 3's capability list is empty for this domain, so there are
        // no per-capability raw-byte cases to write, and this test records why
        // instead of inventing them. Every one of the eight row words is read
        // unconditionally (`lib.rs:325`) and five are range-rejected
        // (`lib.rs:340`); nothing is gated. A failed read never becomes a row —
        // the collector fails the whole CCSS object (`lib.rs:541`) and a payload
        // that will not decode makes the whole domain-hour unknown with the
        // previous accumulator retained (`world_fold.rs::fold_hub_motorization`), which
        // is an absent hour, not an empty field.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field is capturable-but-uncaptured; the hour is what goes unknown"
        );
    }

    #[test]
    fn no_field_is_continuous_so_none_is_quantized() {
        // Step 8 case 4 does not apply: quantization is declarable on continuous
        // fields only (`schema.rs:371`), and d62 has none — every payload word is
        // an integer id, an array index, or an enum code, and the fold applies no
        // scale to any of them. There is therefore no all-`Exact` variant to
        // compare bytes against.
        let c = collection();
        assert!(
            c.fields.iter().all(|f| !matches!(f.kind, Kind::Continuous)),
            "d62 carries no continuous field"
        );
        assert!(c.fields.iter().all(|f| !f.quant.is_quantized()));
        assert!(c
            .fields
            .iter()
            .all(|f| matches!(f.quant, Quant::None | Quant::Exact)));
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d62 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        for tick in [0u64, 1, 7, 1 << 40] {
            assert_eq!(
                record_from_row(&c, &full_row(), tick).expect("maps"),
                record_from_row(&c, &full_row(), 0).expect("maps"),
                "the record must not depend on the frame tick"
            );
        }
    }

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
    fn every_word_lands_in_its_own_field_unscaled() {
        // The offset-and-scale test. d62 applies no scale factor at all, so the
        // property to pin is the other half of step 8 case 6: each word arrives
        // in its own field as the exact integer the collector packed. Every word
        // is distinct, so a transposed pair — `owner`/`actor`, which the ROW
        // packs adjacent while the KEY separates them, is the live hazard —
        // fails here.
        let c = collection();
        let row = raw_row(11, 22, 3333, 44, 55, 66, 2, REQUIRED_FLAGS);
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::STATE_ID)],
            Value::ContentId(44)
        );
        assert_eq!(
            r.values[index_of(&c, field::RECORD_INDEX)],
            Value::Unsigned(55)
        );
        assert_eq!(
            r.values[index_of(&c, field::MAP_ENTRY_INDEX)],
            Value::Unsigned(66)
        );
        // Level 2 stores code 1; see `MOTORIZATION_LEVEL_CARDINALITY`.
        assert_eq!(
            r.values[index_of(&c, field::MOTORIZATION_LEVEL)],
            Value::Enum(1)
        );
        // And the key words, in key order rather than row order.
        assert_eq!(key_fields_of_row(&row).expect("keys"), (11, 3333, 22));
        assert_eq!(key_from_row(&row).expect("keys"), key(&[11, 3333, 22]));
    }

    #[test]
    fn a_zero_array_index_is_a_value_not_an_empty_state() {
        // §3 keeps zero out of the three empty states, and both array indices
        // reach zero constantly in the real capture (the first node, the first
        // map entry).
        let c = collection();
        let r = record_from_row(&c, &raw_row(1, 1, 1, 1, 0, 0, 1, REQUIRED_FLAGS), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::RECORD_INDEX)],
            Value::Unsigned(0),
            "node index 0 is the first node, not a missing value"
        );
        assert_eq!(
            r.values[index_of(&c, field::MAP_ENTRY_INDEX)],
            Value::Unsigned(0)
        );
        // Level 1 is the lowest legal level and stores code 0 — also not empty.
        assert_eq!(
            r.values[index_of(&c, field::MOTORIZATION_LEVEL)],
            Value::Enum(0)
        );
        // ... and it survives the codec as itself.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, r);
    }

    #[test]
    fn both_motorization_levels_roundtrip_through_their_codes() {
        let c = collection();
        for (level, code) in [(1u32, 0u32), (2, 1)] {
            assert_eq!(level_to_code(level).expect("legal level"), code);
            assert_eq!(code_to_level(code), Some(level));
            let r = record_from_row(&c, &raw_row(2, 3, 4, 5, 6, 7, level, REQUIRED_FLAGS), 0)
                .expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::MOTORIZATION_LEVEL)],
                Value::Enum(code)
            );
            let mut bytes = Vec::new();
            r.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(
                back.values[index_of(&c, field::MOTORIZATION_LEVEL)],
                Value::Enum(code),
                "level {level} survives the codec"
            );
            assert_eq!(
                code_to_level(code),
                Some(level),
                "and reads back as the engine's own level"
            );
        }
        // The cardinality admits exactly the legal levels and nothing else, so
        // no decodable code means a level the fold rejects.
        assert_eq!(code_to_level(MOTORIZATION_LEVEL_CARDINALITY), None);
        for level in [0u32, 3, 4, u32::MAX] {
            assert!(level_to_code(level).is_err(), "level {level} is not legal");
        }
    }

    #[test]
    fn an_enum_code_past_the_cardinality_is_rejected_by_the_codec() {
        // The cardinality is load-bearing, not decoration: a record carrying a
        // code the schema does not declare must not encode. This is what makes
        // `enum_card(2)` a guarantee about every decodable record rather than a
        // convention the mapper happens to follow.
        let c = collection();
        let mut r = record_from_row(&c, &full_row(), 0).expect("maps");
        r.values[index_of(&c, field::MOTORIZATION_LEVEL)] = Value::Enum(2);
        let mut bytes = Vec::new();
        assert!(r.encode(&c, &mut bytes).is_err(), "code 2 is not declared");
        assert!(r.validate(&c).is_err());
    }

    #[test]
    fn the_mapper_rejects_every_row_the_codec_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
        }
        // Every range rule of `validate_row` (`lib.rs:340`-`:370`), at both
        // edges: the bound itself is accepted and one past it is not.
        let bounds: [(usize, u32, u32, &str); 5] = [
            (0, 1, MAX_COUNTRY_COUNT, "owner"),
            (1, 1, MAX_COUNTRY_COUNT, "actor"),
            (2, 1, MAX_PROVINCE_ID, "province"),
            (3, 1, MAX_STATE_ID, "state"),
            (6, MIN_MOTORIZATION_LEVEL, MAX_MOTORIZATION_LEVEL, "level"),
        ];
        for (word, min, max, name) in bounds {
            for (value, ok) in [
                (min - 1, false),
                (min, true),
                (max, true),
                (max + 1, false),
                (u32::MAX, false),
            ] {
                let mut words = [16u32, 17, 13071, 5, 1, 3, 2, REQUIRED_FLAGS];
                words[word] = value;
                let row = raw_row(
                    words[0], words[1], words[2], words[3], words[4], words[5], words[6],
                    words[7],
                );
                assert_eq!(
                    record_from_row(&c, &row, 0).is_ok(),
                    ok,
                    "{name} = {value} (bounds {min}..={max})"
                );
            }
        }
        // The flags word: the whole 32-bit space in the two directions that
        // matter. Any missing witness and any unknown bit are both rejections
        // (`lib.rs:371`-`:381`), which is what the schema's omission of the
        // field rests on.
        for bit in 0..32u32 {
            let missing = REQUIRED_FLAGS & !(1 << bit);
            if missing != REQUIRED_FLAGS {
                let row = raw_row(16, 17, 13071, 5, 1, 3, 2, missing);
                assert!(
                    record_from_row(&c, &row, 0).is_err(),
                    "a row missing witness bit {bit} must be rejected"
                );
            }
            let extra = REQUIRED_FLAGS | (1 << bit);
            if extra != REQUIRED_FLAGS {
                let row = raw_row(16, 17, 13071, 5, 1, 3, 2, extra);
                assert!(
                    record_from_row(&c, &row, 0).is_err(),
                    "a row carrying unknown bit {bit} must be rejected"
                );
            }
        }
        // And the only accepted value is the exact witness set.
        assert!(record_from_row(&c, &raw_row(16, 17, 13071, 5, 1, 3, 2, REQUIRED_FLAGS), 0).is_ok());
        assert_eq!(REQUIRED_FLAGS, 0b1_1111);
    }

    #[test]
    fn the_key_extractor_and_the_mapper_agree_on_what_a_valid_row_is() {
        // A harness calls both; if one accepts a row the other rejects, a frame
        // can be half-folded. Swept over the whole key-word range space.
        let c = collection();
        for owner in [0u32, 1, MAX_COUNTRY_COUNT, MAX_COUNTRY_COUNT + 1] {
            for actor in [0u32, 1, MAX_COUNTRY_COUNT, MAX_COUNTRY_COUNT + 1] {
                for province in [0u32, 1, MAX_PROVINCE_ID, MAX_PROVINCE_ID + 1] {
                    let row = raw_row(owner, actor, province, 5, 1, 3, 2, REQUIRED_FLAGS);
                    assert_eq!(
                        key_from_row(&row).is_ok(),
                        record_from_row(&c, &row, 0).is_ok(),
                        "owner {owner} actor {actor} province {province}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_codecs_structural_rules() {
        let rows = vec![full_row()];
        let payload = payload_of(&rows, 0);
        let (header, body) = split_payload(&payload).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.total_map_entries, 1);
        assert_eq!(header.schema_version, SCHEMA_VERSION);
        assert_eq!(header.row_len, ROW_LEN as u32);
        assert_eq!(header.elapsed_wall_ms, 7);
        assert_eq!(body, &rows[0][..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // Length that does not match the row count.
        let mut truncated = payload.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        // Another schema version (`lib.rs:284`) — every one but 2.
        for version in [0u32, 1, 3, u32::MAX] {
            let mut wrong = payload.clone();
            wrong[16..20].copy_from_slice(&version.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "schema_version {version}");
        }
        // Another row stride (`lib.rs:290`).
        for row_len in [0u32, 16, 31, 33, 64] {
            let mut wrong = payload.clone();
            wrong[24..28].copy_from_slice(&row_len.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "row_len {row_len}");
        }
        // total_map_entries != row_count (`world_fold.rs::fold_hub_motorization`).
        for total in [0u32, 2, u32::MAX] {
            let mut wrong = payload.clone();
            wrong[28..32].copy_from_slice(&total.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "total_map_entries {total}");
        }
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = payload.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
        // An empty table is structurally legal: no rows, no map entries.
        let empty = payload_of(&[], 0);
        let (header, body) = split_payload(&empty).expect("an empty table is well-formed");
        assert_eq!(header.row_count, 0);
        assert!(body.is_empty());
        // The header's own status flags are carried through untouched — the
        // caller decides what a failure status means for it.
        let flagged = payload_of(&rows, 0x1234);
        assert_eq!(
            split_payload(&flagged).expect("well-formed").0.status_flags,
            0x1234
        );
    }

    #[test]
    fn split_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: every prefix of a good payload, and every
        // single-byte corruption of one.
        let payload = payload_of(&[full_row(), raw_row(1, 2, 3, 4, 5, 6, 1, REQUIRED_FLAGS)], 0);
        for len in 0..=payload.len() {
            let _ = split_payload(&payload[..len]);
        }
        for i in 0..payload.len() {
            let mut corrupt = payload.clone();
            corrupt[i] ^= 0xFF;
            if let Ok((_, body)) = split_payload(&corrupt) {
                let c = collection();
                for row in body.chunks_exact(ROW_LEN) {
                    let _ = record_from_row(&c, row, 0);
                    let _ = key_from_row(row);
                }
            }
        }
    }
}
