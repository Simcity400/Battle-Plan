//! d63 `country_core_scalars_live` — per-country core scalars.
//!
//! Derived from `world_fold.rs:15618`-`:15838`
//! (`fold_country_core_scalars`), its payload constants at `:159`-`:172`, and the
// @world-fold-authority domain=63 functions=fold_country_core_scalars lines=15618-15838 sha256=8b4e8e779f3c020cc0a8fc27070098698371f5023690e978b7a1c82c750dfb24 closure=593db20c8a895f2f363734a72440cbc30a71893853a026d6d4309abf789d8306
//! collector read that produces the presence flags
//! (`extractor/native/collector/src/attach_log.rs:60573`-`:60757`).
//! The fold and collector read are the field-level basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_country_core_scalars`): a 20-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16` — followed by `row_count` fixed 48-byte
//! rows. There is no legacy short row: the fold requires
//! `payload.len() == 20 + row_count * 48` exactly (`world_fold.rs::fold_country_core_scalars`), so the length never
//! selects a row form and the mapper accepts exactly one.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`               | raw     | none (key; zero rejected, `world_fold.rs::fold_country_core_scalars`) |
//! | `+4`  | u32 | `flags`                   | —       | — |
//! | `+8`  | u32 | `manpower_pool`           | raw     | `1<<0` |
//! | `+12` | u32 | `convoys_destroyed`       | raw     | `1<<1` |
//! | `+16` | u32 | `divisions_lost`          | raw     | `1<<2` |
//! | `+20` | i32 | `army_experience`         | /32768  | `1<<3` |
//! | `+24` | i32 | `army_experience_daily`   | /32768  | `1<<3` |
//! | `+28` | i32 | `navy_experience`         | /32768  | `1<<3` |
//! | `+32` | i32 | `navy_experience_daily`   | /32768  | `1<<3` |
//! | `+36` | i32 | `air_experience`          | /32768  | `1<<3` |
//! | `+40` | i32 | `air_experience_daily`    | /32768  | `1<<3` |
//! | `+44` | i32 | `num_armies_for_training` | /32768  | `1<<3` |
//!
//! **Every gated field is `unknown`, never `null` or `absent`, when its gate is
//! clear.** `fold_country_core_scalars` emits JSON `Null` uniformly on a clear
//! gate (`world_fold.rs::fold_country_core_scalars`), which decides value-vs-nothing
//! and nothing else. The collector decides the rest, and it is explicit
//! (`attach_log.rs:60576`): *"Fail-closed per field: an unreadable/out-of-range
//! value folds ABSENT (null), never a fabricated 0."* Each flag is exactly
//! `Option::is_some()` (`:60730`-`:60741`) over a read that returns `None` only
//! when the memory is unreadable, the pointer is not a heap object, the
//! experience vtable does not match, or the value fails its range filter
//! (`:60578`-`:60634`). That is record-state contract's **unknown** — "applicable, not captured" —
//! word for word. None of these four concepts is a relation that can point at
//! nothing (so not `null`), and every one of them applies to every country row
//! the D18 roster carries (so not `absent`). A country that has genuinely
//! destroyed no convoys reads `0` with its flag **set**, and that is a real
//! `Zero`, not one of the three empty states.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock: the `*_daily` channels are per-day gain *rates* whose own value moves,
//! not a stored quantity a clock-linear transform holds constant, and derived-field contract admits
//! kinds `0`/`1`/`2` only — the seven experience channels are continuous. The
//! mapper still takes `frame_tick` so every domain's boundary has one shape.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 63;
pub const COUNTRIES: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const MANPOWER_POOL: u32 = 2;
    pub const CONVOYS_DESTROYED: u32 = 3;
    pub const DIVISIONS_LOST: u32 = 4;
    pub const ARMY_EXPERIENCE: u32 = 5;
    pub const ARMY_EXPERIENCE_DAILY: u32 = 6;
    pub const NAVY_EXPERIENCE: u32 = 7;
    pub const NAVY_EXPERIENCE_DAILY: u32 = 8;
    pub const AIR_EXPERIENCE: u32 = 9;
    pub const AIR_EXPERIENCE_DAILY: u32 = 10;
    pub const NUM_ARMIES_FOR_TRAINING: u32 = 11;
}

// --- Payload grammar, from `world_fold.rs:159`-`:172` and `:9256`-`:9297`. ----

/// `world_fold.rs::fold_country_core_scalars` — `need(payload, 20, ...)`, then rows at `20 + i * 48`.
pub const HEADER_LEN: usize = 20;
/// `COUNTRY_CORE_ROW_LEN`, `world_fold.rs::COUNTRY_CORE_ROW_LEN`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 48;

pub const FLAG_MANPOWER_POOL: u32 = 1 << 0;
pub const FLAG_CONVOYS_DESTROYED: u32 = 1 << 1;
pub const FLAG_DIVISIONS_LOST: u32 = 1 << 2;
pub const FLAG_EXPERIENCE: u32 = 1 << 3;
/// `COUNTRY_CORE_KNOWN_FLAGS`, `world_fold.rs::COUNTRY_CORE_KNOWN_FLAGS`. Any other bit is a hard
/// error (`:9283`) — an unrecognised flag word means the row is not this grammar.
pub const KNOWN_FLAGS: u32 =
    FLAG_MANPOWER_POOL | FLAG_CONVOYS_DESTROYED | FLAG_DIVISIONS_LOST | FLAG_EXPERIENCE;

/// `COUNTRY_CORE_EXPERIENCE_SCALE`, `world_fold.rs::COUNTRY_CORE_EXPERIENCE_SCALE`. **32768, not
/// `FIXED_SCALE`**: the channels are raw signed int32 CFixedPoint, and the fold
/// divides every one of the seven by this (`:9369`-`:9377`).
pub const EXPERIENCE_SCALE: f64 = 32768.0;

/// `COUNTRY_CORE_MAX_MANPOWER_POOL`, `world_fold.rs::COUNTRY_CORE_MAX_MANPOWER_POOL`.
pub const MAX_MANPOWER_POOL: u32 = 2_000_000_000;
/// `COUNTRY_CORE_MAX_CONVOYS_DESTROYED`, `world_fold.rs::COUNTRY_CORE_MAX_CONVOYS_DESTROYED`.
pub const MAX_CONVOYS_DESTROYED: u32 = 100_000_000;
/// `COUNTRY_CORE_MAX_DIVISIONS_LOST`, `world_fold.rs::COUNTRY_CORE_MAX_DIVISIONS_LOST`.
pub const MAX_DIVISIONS_LOST: u32 = 1_000_000;

/// The seven experience channels, in row order, with their byte offsets
/// (`world_fold.rs::fold_country_core_scalars`). All seven share the single `FLAG_EXPERIENCE`
/// gate and the single `EXPERIENCE_SCALE`.
pub const EXPERIENCE_CHANNELS: [(u32, usize); 7] = [
    (field::ARMY_EXPERIENCE, 20),
    (field::ARMY_EXPERIENCE_DAILY, 24),
    (field::NAVY_EXPERIENCE, 28),
    (field::NAVY_EXPERIENCE_DAILY, 32),
    (field::AIR_EXPERIENCE, 36),
    (field::AIR_EXPERIENCE_DAILY, 40),
    (field::NUM_ARMIES_FOR_TRAINING, 44),
];

pub fn schema() -> Schema {
    use field as f;
    let mut fields = vec![
        // Key: the country's `tag_db_id` (`world_fold.rs::fold_country_core_scalars`), the same
        // identity d18 keys its roster by. The fold's D18 gate is CONTAINMENT
        // (every D18 tag must have a d63 row; rows for tags D18 cannot name
        // are dropped at the persistence boundary with a provenance note,
        // and previously written d63 fields are evicted when a tag later
        // leaves the roster, because the persistent country map doubles as
        // the completeness authority d48/d67 fabricate measured-empty
        // entries from, `:9396`-`:9505`) — d63's collector walks
        // the COMPLETE gamestate country table while D18 publishes the
        // filtered cache, so d63 is a superset of D18 on real captures
        // (439 vs 356 in all 903 deepwar_t1 hours) and the historical
        // bijectivity gate folded zero hours. ContentId because a country tag is
        // content-defined (content-identity contract) — a manifest must resolve it for it to keep its
        // meaning under another mod set, which is the deciding question for
        // ContentId-vs-Unsigned. d20 classes its `owner` country the same way.
        Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
        // A pool that is spent and regenerates, not a counter: `Unsigned`.
        Field::new(f::MANPOWER_POOL, Kind::Unsigned).unknown_capable(),
        // Cumulative lifetime counters. `Monotone` is an encoding hint and never
        // a validity constraint (schema-kind contract), so a counter the engine may reset or trim
        // still declares it.
        Field::new(f::CONVOYS_DESTROYED, Kind::Monotone).unknown_capable(),
        Field::new(f::DIVISIONS_LOST, Kind::Monotone).unknown_capable(),
    ];

    // The seven experience channels. `Quant::Exact` for every one of them, on
    // derived-field contract's own rule: "A field is `exact` unless its quantization is justified in
    // domain evidence",
    // carries no field-level knowledge at all — there is no recorded precision
    // and no showing that any rounding is invisible, so no quantization is
    // justified and none is declared. Both vetoes also have live edges here: the
    // accumulated pools are spent against integer thresholds (outcome test), and
    // the `*_daily` rates are what a viewer would integrate between samples
    // (appearance test). See the open questions in the deliverable.
    //
    // `num_armies_for_training` is `Continuous`, not `Unsigned`, because the
    // fold divides it by `EXPERIENCE_SCALE` like every other channel
    // (`world_fold.rs::fold_country_core_scalars`) — the fold is the authority even where the name
    // reads like a count.
    for (id, _) in EXPERIENCE_CHANNELS {
        fields.push(
            Field::new(id, Kind::Continuous)
                .unknown_capable()
                .quant(Quant::Exact),
        );
    }

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: COUNTRIES,
            // Full-replace each frame (`world_fold.rs::fold_country_core_scalars` requires
            // `changed_rows == row_count`), but `tag_db_id` is a stable identity
            // that persists across hours, which is what a keyed set needs — and
            // only a keyed set has a delta grammar (collection-shape contract).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 20-byte payload header (`world_fold.rs::fold_country_core_scalars`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
}

/// Split a d63 DATA payload into its header and its rows slice, applying the
/// fold's own structural checks (`world_fold.rs::fold_country_core_scalars`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "country_core_scalars payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
    };
    // Checked arithmetic: `row_count` is attacker-shaped input from a stream on
    // disk, and `row_count * ROW_LEN` overflows `usize` for a large enough word.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("country_core_scalars row_count {} overflows", header.row_count))?;
    if payload.len() != expected {
        return Err(format!(
            "country_core_scalars len {} != {expected}",
            payload.len()
        ));
    }
    // `world_fold.rs::fold_country_core_scalars`: d63 is full-replace, so a header claiming a partial
    // update is a malformed frame rather than a delta to be applied.
    if header.changed_rows != header.row_count {
        return Err(format!(
            "country_core_scalars changed_rows {} != full-replace row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The key of one compact row: its `tag_db_id` (`world_fold.rs::fold_country_core_scalars`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[tag_db_id_of_row(row)? as u64]))
}

/// `tag_db_id` with the fold's zero check (`world_fold.rs::fold_country_core_scalars`).
pub fn tag_db_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let tag_db_id = u32_at(row, 0);
    if tag_db_id == 0 {
        return Err("country_core_scalars row with zero tag_db_id".into());
    }
    Ok(tag_db_id)
}

/// One compact row of the `countries` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 48-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d63 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d63 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_country_core_scalars`: a row that the
/// fold would reject is an `Err` here, never a coerced value. Nothing in this
/// function can panic on any 48-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d63 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row)?;
    // `tag_db_id` is the key and so is not carried in the record body (record-state contract), but
    // its validity is still part of this row's validity.
    let _ = tag_db_id_of_row(row)?;

    let flags = u32_at(row, 4);
    // `world_fold.rs::fold_country_core_scalars`: an unknown flag bit means the row is not this
    // grammar, so nothing in it can be trusted.
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "country_core_scalars row {} has unknown flags {flags:#x}",
            u32_at(row, 0)
        ));
    }

    let has_manpower = flags & FLAG_MANPOWER_POOL != 0;
    let has_convoys = flags & FLAG_CONVOYS_DESTROYED != 0;
    let has_divisions_lost = flags & FLAG_DIVISIONS_LOST != 0;
    let has_experience = flags & FLAG_EXPERIENCE != 0;

    // `validate_optional_u32_field`, `world_fold.rs::validate_optional_u32_field`: a present value must
    // be in range, and a gated-off field must pack a zero. The second half is
    // what makes an ungated non-zero word a malformed row rather than data.
    validate_optional_u32(
        has_manpower,
        u32_at(row, 8),
        MAX_MANPOWER_POOL,
        "country_manpower_pool",
    )?;
    validate_optional_u32(
        has_convoys,
        u32_at(row, 12),
        MAX_CONVOYS_DESTROYED,
        "convoys_destroyed",
    )?;
    validate_optional_u32(
        has_divisions_lost,
        u32_at(row, 16),
        MAX_DIVISIONS_LOST,
        "divisions_lost",
    )?;
    // `world_fold.rs::fold_country_core_scalars`: the seven channels only carry meaning under
    // the flag, and must be zero-packed without it.
    if !has_experience
        && EXPERIENCE_CHANNELS
            .iter()
            .any(|(_, offset)| i32_at(row, *offset) != 0)
    {
        return Err("country_core_scalars experience present without flag".into());
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::MANPOWER_POOL => {
                gated_unsigned(has_manpower, u32_at(row, 8) as u64)
            }
            field::CONVOYS_DESTROYED => {
                gated_unsigned(has_convoys, u32_at(row, 12) as u64)
            }
            field::DIVISIONS_LOST => {
                gated_unsigned(has_divisions_lost, u32_at(row, 16) as u64)
            }
            other => {
                let Some((_, offset)) =
                    EXPERIENCE_CHANNELS.iter().copied().find(|(id, _)| *id == other)
                else {
                    // A collection this mapper does not know how to fill. Filling
                    // it with a placeholder would publish invented data under a
                    // field id that means something else.
                    return Err(format!(
                        "country_core_scalars has no mapping for field id {other}"
                    ));
                };
                if has_experience {
                    Value::Continuous(i32_at(row, offset) as f64 / EXPERIENCE_SCALE)
                } else {
                    Value::Unknown
                }
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// A gate that is clear means the collector could not read the value, not that
/// the value is nothing — see the module docs. record-state contract calls that **unknown**.
fn gated_unsigned(present: bool, value: u64) -> Value {
    if present {
        Value::Unsigned(value)
    } else {
        Value::Unknown
    }
}

fn validate_optional_u32(
    present: bool,
    value: u32,
    max_value: u32,
    field: &str,
) -> core::result::Result<(), String> {
    if present {
        if value > max_value {
            return Err(format!("{field} value {value} out of range"));
        }
    } else if value != 0 {
        return Err(format!("{field} present value without flag"));
    }
    Ok(())
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "country_core_scalars row length {} != {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// A raw 48-byte row, built the way the collector packs one
    /// (`attach_log.rs:60743`-`:60754`).
    fn raw_row(tag_db_id: u32, flags: u32, scalars: [u32; 3], experience: [i32; 7]) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in scalars {
            row.extend_from_slice(&v.to_le_bytes());
        }
        for v in experience {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    fn full_row() -> Vec<u8> {
        raw_row(
            7,
            KNOWN_FLAGS,
            [1_234_567, 42, 9],
            // 32768 == 1.0, 49152 == 1.5, -32768 == -1.0, 1 == 1/32768.
            [32_768, 1, -32_768, 49_152, 65_536, -1, 3],
        )
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d63 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_country() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by tag_db_id alone");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (content-identity contract), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 11, "1 key + 3 scalars + 7 experience channels");
    }

    #[test]
    fn every_gated_field_is_unknown_capable_and_nothing_else() {
        // The schema declares exactly one capability, so the mapper may emit
        // exactly one empty state. Absent would claim the concept does not apply
        // to a country; Null would claim we know there is nothing. The collector
        // clears a gate only when the read failed (`attach_log.rs:60576`), which
        // is record-state contract's "applicable, not captured".
        let c = collection();
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
        assert_eq!(unknown_capable.len(), 10, "every value field is gated");
        assert_eq!(
            unknown_capable,
            c.value_fields().map(|f| f.field_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_continuous_field_is_quantized() {
        // Step 4's "quantization saves bytes" test does not apply: d63 declares
        // no quantized field. derived-field contract makes `exact` the default and requires the
        // domain document to justify anything else, and
        // No retained evidence specifies field-level
        // knowledge — no precision, no showing that rounding is invisible. The
        // accumulated pools are also spent against thresholds (outcome test) and
        // the `*_daily` rates are integrable between samples (appearance test).
        let c = collection();
        for f in c.value_fields() {
            if matches!(f.kind, Kind::Continuous) {
                assert_eq!(f.quant, Quant::Exact, "field {} must stay exact", f.field_id);
            }
        }
        assert_eq!(
            c.value_fields()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            7
        );
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d63 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
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
    fn a_fully_gated_off_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &raw_row(7, 0, [0; 3], [0; 7]), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    #[test]
    fn the_mapper_emits_values_for_a_full_row() {
        let c = collection();
        let r = record_from_row(&c, &full_row(), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::MANPOWER_POOL)],
            Value::Unsigned(1_234_567)
        );
        assert_eq!(
            r.values[index_of(&c, field::CONVOYS_DESTROYED)],
            Value::Unsigned(42)
        );
        assert_eq!(
            r.values[index_of(&c, field::DIVISIONS_LOST)],
            Value::Unsigned(9)
        );
        assert_eq!(
            r.values[index_of(&c, field::ARMY_EXPERIENCE)],
            Value::Continuous(1.0)
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Every flag set, every value zero: the row the collector packs for a
        // country that has destroyed no convoys and lost no divisions. record-state contract keeps
        // zero out of the three empty states.
        let c = collection();
        let r = record_from_row(&c, &raw_row(1, KNOWN_FLAGS, [0; 3], [0; 7]), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::MANPOWER_POOL)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::CONVOYS_DESTROYED)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::DIVISIONS_LOST)],
            Value::Unsigned(0)
        );
        for (id, _) in EXPERIENCE_CHANNELS {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(0.0),
                "channel {id} zero is a measured value"
            );
        }
    }

    #[test]
    fn a_clear_gate_is_unknown_field_by_field() {
        let c = collection();
        // One gate clear at a time — the whole gate space, not one sampled case.
        let cases: [(u32, &[u32]); 4] = [
            (FLAG_MANPOWER_POOL, &[field::MANPOWER_POOL]),
            (FLAG_CONVOYS_DESTROYED, &[field::CONVOYS_DESTROYED]),
            (FLAG_DIVISIONS_LOST, &[field::DIVISIONS_LOST]),
            (
                FLAG_EXPERIENCE,
                &[
                    field::ARMY_EXPERIENCE,
                    field::ARMY_EXPERIENCE_DAILY,
                    field::NAVY_EXPERIENCE,
                    field::NAVY_EXPERIENCE_DAILY,
                    field::AIR_EXPERIENCE,
                    field::AIR_EXPERIENCE_DAILY,
                    field::NUM_ARMIES_FOR_TRAINING,
                ],
            ),
        ];
        for (cleared, gated_ids) in cases {
            let flags = KNOWN_FLAGS & !cleared;
            // A gated-off field must pack a zero, so build the row that way.
            let scalars = [
                if flags & FLAG_MANPOWER_POOL != 0 { 5 } else { 0 },
                if flags & FLAG_CONVOYS_DESTROYED != 0 { 6 } else { 0 },
                if flags & FLAG_DIVISIONS_LOST != 0 { 7 } else { 0 },
            ];
            let experience = if flags & FLAG_EXPERIENCE != 0 {
                [32_768; 7]
            } else {
                [0; 7]
            };
            let r = record_from_row(&c, &raw_row(3, flags, scalars, experience), 0)
                .unwrap_or_else(|e| panic!("maps with flags {flags:#x}: {e}"));
            for f in c.value_fields() {
                let expected_unknown = gated_ids.contains(&f.field_id);
                assert_eq!(
                    r.values[index_of(&c, f.field_id)] == Value::Unknown,
                    expected_unknown,
                    "field {} under flags {flags:#x}",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn the_unknown_state_survives_a_roundtrip() {
        let c = collection();
        let row = raw_row(3, FLAG_MANPOWER_POOL, [11, 0, 0], [0; 7]);
        let record = record_from_row(&c, &row, 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(
            back.values[index_of(&c, field::MANPOWER_POOL)],
            Value::Unsigned(11)
        );
        for id in [
            field::CONVOYS_DESTROYED,
            field::DIVISIONS_LOST,
            field::ARMY_EXPERIENCE,
            field::NUM_ARMIES_FOR_TRAINING,
        ] {
            assert_eq!(back.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
    }

    #[test]
    fn every_experience_channel_uses_the_32768_scale() {
        // The test that catches a 32768-vs-100000 mapping error: known
        // fixed-point words against their exact f64 readings. Each channel is
        // checked at its own offset, so a transposed pair fails too.
        let c = collection();
        let raws: [i32; 7] = [32_768, 49_152, -32_768, 1, -1, 65_536, 16_384];
        let expected: [f64; 7] = [
            1.0,
            1.5,
            -1.0,
            1.0 / 32_768.0,
            -1.0 / 32_768.0,
            2.0,
            0.5,
        ];
        let r = record_from_row(&c, &raw_row(9, KNOWN_FLAGS, [0; 3], raws), 0).expect("maps");
        for (i, (id, _)) in EXPERIENCE_CHANNELS.into_iter().enumerate() {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(expected[i]),
                "channel {id} raw {} must read {} (scale {EXPERIENCE_SCALE})",
                raws[i],
                expected[i]
            );
        }
        // And the whole-scale claim: the three u32 scalars are raw, unscaled.
        let r = record_from_row(&c, &raw_row(9, KNOWN_FLAGS, [100_000, 32_768, 1], [0; 7]), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::MANPOWER_POOL)],
            Value::Unsigned(100_000),
            "manpower_pool is emitted raw by the fold, never divided"
        );
        assert_eq!(
            r.values[index_of(&c, field::CONVOYS_DESTROYED)],
            Value::Unsigned(32_768)
        );
        assert_eq!(
            r.values[index_of(&c, field::DIVISIONS_LOST)],
            Value::Unsigned(1)
        );
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 104] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
        }
        // Zero tag_db_id (`world_fold.rs::fold_country_core_scalars`).
        assert!(record_from_row(&c, &raw_row(0, KNOWN_FLAGS, [0; 3], [0; 7]), 0).is_err());
        // An unknown flag bit (`:9283`) — every one of them, not just one sample.
        for bit in 4..32u32 {
            let row = raw_row(1, KNOWN_FLAGS | (1 << bit), [0; 3], [0; 7]);
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "flag bit {bit} is not in KNOWN_FLAGS and must be rejected"
            );
        }
        // Out-of-range present values (`:9306`-`:9326`), each at max + 1.
        for (flag_index, max) in [
            (0usize, MAX_MANPOWER_POOL),
            (1, MAX_CONVOYS_DESTROYED),
            (2, MAX_DIVISIONS_LOST),
        ] {
            let mut scalars = [0u32; 3];
            scalars[flag_index] = max + 1;
            assert!(
                record_from_row(&c, &raw_row(1, KNOWN_FLAGS, scalars, [0; 7]), 0).is_err(),
                "scalar {flag_index} at {} exceeds its maximum",
                max + 1
            );
            // And the maximum itself is accepted, so the bound is not off by one.
            scalars[flag_index] = max;
            assert!(record_from_row(&c, &raw_row(1, KNOWN_FLAGS, scalars, [0; 7]), 0).is_ok());
        }
        // A gated-off scalar carrying a non-zero word (`:30208`).
        for flag_index in 0..3usize {
            let mut scalars = [0u32; 3];
            scalars[flag_index] = 1;
            let flags = KNOWN_FLAGS & !(1 << flag_index);
            assert!(
                record_from_row(&c, &raw_row(1, flags, scalars, [0; 7]), 0).is_err(),
                "scalar {flag_index} is non-zero without its flag"
            );
        }
        // A gated-off experience channel carrying a non-zero word (`:9329`),
        // each of the seven independently.
        for channel in 0..7usize {
            let mut experience = [0i32; 7];
            experience[channel] = 1;
            let flags = KNOWN_FLAGS & !FLAG_EXPERIENCE;
            assert!(
                record_from_row(&c, &raw_row(1, flags, [0; 3], experience), 0).is_err(),
                "experience channel {channel} is non-zero without the flag"
            );
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
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = full_row();
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&1u32.to_le_bytes()); // row_count
        payload.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        payload.extend_from_slice(&rows);
        let (header, body) = split_payload(&payload).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(body, &rows[..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // Length that does not match the row count.
        let mut truncated = payload.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        // changed_rows != row_count (`world_fold.rs::fold_country_core_scalars`).
        let mut partial = payload.clone();
        partial[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&partial).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = payload.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
    }

    #[test]
    fn the_key_is_the_tag_db_id() {
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[7]));
        assert!(key_from_row(&raw_row(0, 0, [0; 3], [0; 7])).is_err());
        assert!(key_from_row(&[]).is_err());
    }
}
