//! d10 `fuel_live` — per-country fuel status.
//!
//! Derived from `world_fold.rs:12503`-`:12548`
//! (`fold_fuel`), its scale constant at `:152`, the collector read that produces
// @world-fold-authority domain=10 functions=fold_fuel lines=12503-12548 sha256=1ab92f138878297dc602291e5fb10a29e3aed69972c37219c70f824f979c3aee closure=8bef133ce4cfd5c61bcd2b93124268e22b83298bab072c9bba2bb9d4e8753811
//! the rows (`extractor/native/collector/src/attach_log.rs:83846`-`:83885`), the
//! collector's payload encoder (`:67976`-`:67999`), and the recorder's own
//! structural decoder for this payload
//! (`recorder_host/src/domain_data_authority.rs:932`-`:1011`).
//! `extractor/documents/domains/d10_fuel_live.md` records the live source
//! (`CCountry+0x1580 -> CCountryFuelStatus`, four signed i64 at `+0x08`, `+0x10`,
//! `+0x18`, `+0x38`, scale `32768`) and nothing about precision or four-state
//! semantics, so the fold and the collector read are the basis for those.
//!
//! **Payload grammar** (`world_fold.rs::fold_fuel`, `attach_log.rs:67982`-`:67996`):
//! a 24-byte header — `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`,
//! `changed_rows` u32 at `+12`, `status_flags` u32 at `+16`, `scale_raw` u32 at
//! `+20` — followed by `row_count` fixed 40-byte rows. There is no legacy short
//! row: the fold requires `payload.len() == 24 + row_count * 40` exactly
//! (`:9226`-`:9229`), so the length never selects a row form.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id` | raw    | none (key; zero rejected, `:9233`) |
//! | `+4`  | u32 | row flags   | —      | — (always `1`; see below) |
//! | `+8`  | i64 | `current`   | /32768 | none |
//! | `+16` | i64 | `max`       | /32768 | none |
//! | `+24` | i64 | `gain`      | /32768 | none |
//! | `+32` | i64 | `cost`      | /32768 | none |
//!
//! **The scale is 32768, not 100000.** `FUEL_SCALE` (`world_fold.rs::FUEL_SCALE`) is
//! applied to all four fields at `:9236`-`:9239`. The general `FIXED_SCALE`
//! (`world_fold.rs::FIXED_SCALE`) is 100_000 and the field named `fuel` in d20 folds at *that* scale
//! (`:19387`) — reading d10's fields at 100_000 would make every value ~3.05x too
//! small while every round-trip test still passed. The payload carries its own
//! scale word at `+20` (`FUEL_SCALE_RAW`, `collector/src/attach_log/offsets.rs:378`),
//! and [`split_payload`] checks it, so this domain's scale is self-verifying on
//! the wire — a guard the fold itself does not have (see the blind spots below).
//!
//! **No empty state exists at field granularity: every field is always a real
//! value.** There are no presence flags over the four measures. The collector
//! reads all four from one validated `CCountryFuelStatus` and emits a row only if
//! every one of them read back in range (`attach_log.rs:83846`-`:83877`); a
//! country whose read failed contributes **no row at all**
//! (`:67981` filters on `row.fuel.is_some()`). So `Absent`, `Null` and `Unknown`
//! are unreachable per field, none is declared, and a `0` — d10's `cost` is `0`
//! for every row of the reference capture — is a measured `Zero`, not an empty
//! state. The row-level "we could not read this country's fuel" state has no
//! encoding in this payload at all; see the delta-semantics note below.
//!
//! **The row flags word at `+4` carries nothing this mapper consumes.** Both
//! shipping encoders write a literal `1u32` (`attach_log.rs:67992`,
//! `recorder_host/src/main.rs:33666`); `fold_fuel` never reads the word; and the
//! recorder's structural decoder rejects only the bits outside bit 0
//! (`domain_data_authority.rs:989`), so a cleared bit 0 is accepted there too.
//! This mapper does exactly that — an unknown bit means the row is not this
//! grammar and is rejected, and a cleared bit 0 changes nothing, because the fold
//! maps the four measures regardless. Bit 0's intended meaning is therefore
//! untested by any producer; it is in the deliverable's open questions.
//!
//! **Delta semantics — the payload is incremental, not full-replace.** Under the
//! production capture profile (`compact_country_frame: true`,
//! `compact_contract/src/lib.rs:664`) the collector emits a fuel row only for a
//! country whose fuel fingerprint — `(status_ptr, current, max, gain, cost)` —
//! changed since the last frame (`attach_log.rs:79961`-`:79968`, gating the read
//! at `:79862`). `fold_fuel` matches that exactly: it upserts each row into the
//! country object (`:9240`-`:9244`, `country_mut` at `:3112`) and **never clears
//! countries that are missing from the frame**, unlike the full-replace domains
//! that wipe their per-country subtree first (e.g. `:18773`, `:20610`). The
//! reference capture confirms the shape: hour 1 carries all 356 countries, the
//! next hour carries 0 rows, and the rest average 26.6 rows — and no re-emitted
//! row ever repeats its previous value (0 of 23,652).
//!
//! Two consequences, both load-bearing:
//!
//! 1. The state this domain's `KeyedSet` models is the fold's **accumulated**
//!    country map, not one payload. A row missing from an hour means "unchanged",
//!    so a keyed state built from a single payload would delete every country the
//!    hour did not mention. `bench_real_d10.rs` accumulates; a future reader must
//!    too.
//! 2. Row absence therefore conflates *unchanged* with *unreadable this frame*:
//!    a country whose fuel pointer goes bad is omitted exactly like a country
//!    whose fuel did not move, and the fold keeps serving the last value. That is
//!    a property of the shipping payload and the fold, not of this schema; d10
//!    has no wire room to distinguish them and this mapper does not invent one.
//!    It is in the deliverable's open questions.
//!
//! **What `fold_fuel` does not check** (each is enforced here or in
//! [`split_payload`], with the authority that does check it named):
//!
//! - `status_flags` at `+16` — the fold never reads it and has no
//!   `handle_compact_failure` call (contrast d63 at `:9272`), so a
//!   failure-masked payload folds as authoritative. The measurement harness gates
//!   on the shipping predicate instead.
//! - `scale_raw` at `+20` — checked by `domain_data_authority.rs:949` and by
//!   [`split_payload`], never by the fold.
//! - `changed_rows` at `+12` — checked by `domain_data_authority.rs:962`
//!   (`changed_rows <= row_count`), never by the fold.
//! - unknown row flag bits and negative `current`/`max`
//!   (`domain_data_authority.rs:989`, `:998`-`:1007`).
//! - a `tag_db_id` duplicated inside one payload (`:984`) — the fold's upsert
//!   would silently take the last writer, and a keyed set cannot hold both rows;
//!   [`split_payload`] rejects the payload.
//! - `24 + row_count * 40` is unchecked arithmetic in the fold (`:9226`);
//!   [`split_payload`] uses checked multiplication.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: `current` moves by production and consumption, and `gain`/`cost` are
//! rates whose own value moves. §6 admits kinds `0`/`1`/`2` only, and all four
//! measures are continuous. The mapper still takes `frame_tick` so every domain's
//! boundary has one shape.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 10;
pub const COUNTRIES: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const CURRENT: u32 = 2;
    pub const MAX: u32 = 3;
    pub const GAIN: u32 = 4;
    pub const COST: u32 = 5;
}

// --- Payload grammar, from `world_fold.rs:9224`-`:9239` and `:152`. -----------

/// `world_fold.rs::fold_fuel` — `need(payload, 24, "fuel header")`, then rows at
/// `24 + i * 40`.
pub const HEADER_LEN: usize = 24;
/// `world_fold.rs::fold_fuel` — `24 + row_count * 40`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 40;

/// `FUEL_SCALE`, `world_fold.rs::FUEL_SCALE`, applied to all four measures at
/// `:9236`-`:9239`. **32768, not `FIXED_SCALE`'s 100_000.**
pub const FUEL_SCALE: f64 = 32768.0;

/// `FUEL_SCALE_RAW`, `collector/src/attach_log/offsets.rs:378`, written into the
/// header at `+20` (`attach_log.rs:67988`) and required by the recorder's own
/// decoder (`domain_data_authority.rs:949`, `COMPACT_FUEL_SCALE_RAW` at `:150`).
pub const FUEL_SCALE_RAW: u32 = 32768;

/// Row flags bit 0: the row's fuel constituents are present. Both shipping
/// encoders write exactly this (`attach_log.rs:67992`, `main.rs:33666`).
pub const ROW_FLAG_FUEL_PRESENT: u32 = 1 << 0;
/// Every bit the row flags word may carry (`domain_data_authority.rs:989`
/// rejects `row_flags & !1`).
pub const ROW_KNOWN_FLAGS: u32 = ROW_FLAG_FUEL_PRESENT;

/// `MAX_FUEL_MEASURE`, `collector/src/attach_log/offsets.rs:380`: the collector
/// drops a row whose scaled `current`/`max` is outside `0..=1e9` or whose scaled
/// `gain`/`cost` exceeds `1e9` in magnitude (`attach_log.rs:83859`-`:83885`).
pub const MAX_FUEL_MEASURE: f64 = 1_000_000_000.0;

/// The same bound in the raw fixed-point domain: `1e9 * 32768`, exact in `i64`
/// and in binary64. Checking the raw integer rather than the quotient keeps the
/// bound exact and keeps every accepted value below `2^53`, where `i64 as f64` is
/// lossless — a row at this bound divides to exactly `1e9`.
pub const MAX_FUEL_RAW: i64 = 32_768_000_000_000;

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // Key: the country's `tag_db_id` (`world_fold.rs:9232`), the identity the
        // fold indexes its country map by (`country_mut`, `world_fold.rs::country_mut`). ContentId
        // because a country tag is content-defined (§5) — a manifest must resolve
        // it for it to keep its meaning under another mod set. d20 classes its
        // `owner` country the same way, and d63 keys on the same id.
        Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
        // All four are `Continuous`: the fold divides each by `FUEL_SCALE`
        // (`world_fold.rs:9236`-`:9239`), so none of them is an integer quantity.
        // `Quant::Exact` for all four — see `no_continuous_field_is_quantized`.
        // No `.optional()`, `.nullable()` or `.unknown_capable()`: a row exists
        // only when all four measures read back in range, so no field can reach
        // an empty state (module docs).
        Field::new(f::CURRENT, Kind::Continuous).quant(Quant::Exact),
        Field::new(f::MAX, Kind::Continuous).quant(Quant::Exact),
        // `gain` and `cost` are signed on the wire and the collector admits both
        // signs (`valid_signed_fuel_measure`, `attach_log.rs:83883`), so their
        // range check differs from `current`/`max`.
        Field::new(f::GAIN, Kind::Continuous).quant(Quant::Exact),
        Field::new(f::COST, Kind::Continuous).quant(Quant::Exact),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: COUNTRIES,
            // `tag_db_id` is a stable identity that persists across hours, and
            // only a keyed set has a delta grammar (§7.3). Note that the *state*
            // a d10 keyed set holds is the fold's accumulated country map, not
            // one payload — the payload is an incremental update (module docs).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 24-byte payload header (`world_fold.rs::fold_fuel`,
/// `attach_log.rs:67984`-`:67988`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    /// The payload's self-declared fixed-point scale; always [`FUEL_SCALE_RAW`].
    pub scale_raw: u32,
}

/// Split a d10 DATA payload into its header and its rows slice, applying the
/// fold's structural checks (`world_fold.rs::fold_fuel`) plus the three the
/// recorder's own decoder applies to this same payload and the fold does not:
/// the scale word (`domain_data_authority.rs:949`), `changed_rows <= row_count`
/// (`:962`), and a `tag_db_id` repeated inside one payload (`:984`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "fuel payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        scale_raw: u32_at(payload, 20),
    };
    // Checked arithmetic: `row_count` is input from a stream on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough word. The fold's
    // own `24 + row_count * 40` (`:9226`) is unchecked.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("fuel row_count {} overflows", header.row_count))?;
    if payload.len() != expected {
        return Err(format!("fuel len {} != {expected}", payload.len()));
    }
    // `domain_data_authority.rs:949`. The scale word is what makes this domain's
    // 32768 self-verifying rather than assumed: a payload written against another
    // scale announces itself here instead of folding ~3.05x wrong.
    if header.scale_raw != FUEL_SCALE_RAW {
        return Err(format!(
            "fuel scale {} != expected {FUEL_SCALE_RAW}",
            header.scale_raw
        ));
    }
    // d10 is NOT full-replace (module docs), so `changed_rows == row_count` is
    // not the invariant it is in d63 — it is merely what both shipping encoders
    // happen to write (`attach_log.rs:67985`-`:67986`, `main.rs:33660`-`:33661`,
    // the row count twice) and what the reference capture carries in 903 of 903
    // records. The rule enforced here is the one an authority actually states:
    // `domain_data_authority.rs:962`'s `changed_rows <= row_count`. Requiring
    // equality would refuse a payload both the fold and the recorder's own
    // decoder accept, which is a rejection with nothing behind it.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "fuel changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    let rows = &payload[HEADER_LEN..];
    // A `tag_db_id` repeated inside one payload (`domain_data_authority.rs:984`).
    // This is a cross-row structural property, so it belongs here rather than in
    // the per-row mapper, which cannot see it: `fold_fuel`'s upsert
    // (`world_fold.rs::fold_fuel`) would silently take the last writer, and a keyed set
    // cannot hold both rows at all. Every caller gets the check, not just the one
    // that remembered to write it. A zero `tag_db_id` is still the per-row
    // mapper's rejection (`world_fold.rs::fold_fuel`); this pass only reads the word.
    let mut seen = std::collections::BTreeSet::new();
    for row in rows.chunks_exact(ROW_LEN) {
        let tag_db_id = u32_at(row, 0);
        if !seen.insert(tag_db_id) {
            return Err(format!("fuel payload repeats tag_db_id {tag_db_id}"));
        }
    }
    Ok((header, rows))
}

/// The key of one compact row: its `tag_db_id` (`world_fold.rs::fold_fuel`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[tag_db_id_of_row(row)? as u64]))
}

/// `tag_db_id` with the fold's zero check (`world_fold.rs::fold_fuel`).
pub fn tag_db_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let tag_db_id = u32_at(row, 0);
    if tag_db_id == 0 {
        return Err("fuel row with zero tag_db_id".into());
    }
    Ok(tag_db_id)
}

/// One compact row of the `countries` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 40-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d10 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d10 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout: a row the fold or the recorder's own structural
/// decoder would reject is an `Err` here, never a coerced value. Nothing in this
/// function can panic on any 40-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d10 has no derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row)?;
    // `tag_db_id` is the key and so is not carried in the record body (§3), but
    // its validity is still part of this row's validity.
    let tag_db_id = tag_db_id_of_row(row)?;

    // `domain_data_authority.rs:989` rejects `row_flags & !1` and nothing else,
    // so bit 0 clear is accepted there; `fold_fuel` ignores the word entirely and
    // reads all four measures regardless (`world_fold.rs::fold_fuel`). This
    // mapper matches both: an unknown bit means the row is not this grammar, and
    // a clear bit 0 does not, so the measures are mapped exactly as the fold maps
    // them. No shipping encoder writes anything but `1` (`attach_log.rs:67992`,
    // `main.rs:33666`), so bit 0's meaning is untested by any producer — see the
    // deliverable's open questions.
    let flags = u32_at(row, 4);
    if flags & !ROW_KNOWN_FLAGS != 0 {
        return Err(format!(
            "fuel row {tag_db_id} has unknown flags {flags:#x}"
        ));
    }

    let current = i64_at(row, 8);
    let max = i64_at(row, 16);
    let gain = i64_at(row, 24);
    let cost = i64_at(row, 32);
    // The collector's own admission filter, in the raw domain
    // (`attach_log.rs:83859`-`:83885`): `current`/`max` non-negative and bounded,
    // `gain`/`cost` bounded in magnitude. A row outside these did not come from
    // the shipping read path — the read that produced it would have returned
    // `None` and the row would not exist. `domain_data_authority.rs:998`-`:1007`
    // independently flags a negative `current`/`max`.
    check_nonnegative(current, "current", tag_db_id)?;
    check_nonnegative(max, "max", tag_db_id)?;
    check_magnitude(gain, "gain", tag_db_id)?;
    check_magnitude(cost, "cost", tag_db_id)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let raw = match f.field_id {
            field::CURRENT => current,
            field::MAX => max,
            field::GAIN => gain,
            field::COST => cost,
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!("fuel has no mapping for field id {other}"));
            }
        };
        // `world_fold.rs::fold_fuel`. Every accepted raw is `<= MAX_FUEL_RAW`
        // in magnitude, well inside `2^53`, so this conversion is exact.
        values.push(Value::Continuous(raw as f64 / FUEL_SCALE));
    }
    Ok(Record::new(values))
}

fn check_nonnegative(raw: i64, field: &str, tag_db_id: u32) -> core::result::Result<(), String> {
    if !(0..=MAX_FUEL_RAW).contains(&raw) {
        return Err(format!(
            "fuel row {tag_db_id} {field} raw {raw} outside 0..={MAX_FUEL_RAW}"
        ));
    }
    Ok(())
}

fn check_magnitude(raw: i64, field: &str, tag_db_id: u32) -> core::result::Result<(), String> {
    // `i64::MIN.abs()` overflows, so compare against the bounds directly.
    if !(-MAX_FUEL_RAW..=MAX_FUEL_RAW).contains(&raw) {
        return Err(format!(
            "fuel row {tag_db_id} {field} raw {raw} outside +/-{MAX_FUEL_RAW}"
        ));
    }
    Ok(())
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!("fuel row length {} != {ROW_LEN}", row.len()));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
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

    /// A raw 40-byte row, built the way the collector packs one
    /// (`attach_log.rs:67991`-`:67996`).
    fn raw_row(tag_db_id: u32, flags: u32, measures: [i64; 4]) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in measures {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// The first row of the reference capture
    /// (`runs/batch1/deepwar_t1/collector_stream.bin`), verbatim.
    fn full_row() -> Vec<u8> {
        raw_row(
            1,
            ROW_FLAG_FUEL_PRESENT,
            [23_717_908_696, 24_002_560_000, 200_000, 0],
        )
    }

    fn payload(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        payload.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        payload.extend_from_slice(&FUEL_SCALE_RAW.to_le_bytes());
        for row in rows {
            payload.extend_from_slice(row);
        }
        payload
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d10 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_country() {
        let c = collection();
        assert_eq!(
            c.shape,
            Shape::KeyedSet,
            "baseline replaces, deltas upsert; the state is the fold's accumulated country map"
        );
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by tag_db_id alone");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 5, "1 key + current/max/gain/cost");
        assert_eq!(
            c.value_fields().map(|f| f.field_id).collect::<Vec<_>>(),
            vec![field::CURRENT, field::MAX, field::GAIN, field::COST],
            "field order is the record's value order and is part of the schema hash"
        );
    }

    #[test]
    fn no_field_declares_an_empty_state() {
        // Step 8's per-capability raw-bytes cases do not apply: d10 declares no
        // `.optional()`, no `.nullable()` and no `.unknown_capable()` field, so
        // there is no Absent, Null or Unknown case to build. A d10 row exists
        // only when the collector read all four measures back in range
        // (`attach_log.rs:83846`-`:83877`) and emitted the row at all
        // (`:67981`); an unreadable country contributes no row, not an empty
        // field. This test is what keeps that claim from drifting: if a field
        // ever gains a capability, its raw-bytes case has to arrive with it.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "nothing is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field can be present-but-uncaptured: the row would not exist"
        );
    }

    #[test]
    fn no_continuous_field_is_quantized() {
        // Step 4's "quantization saves bytes" test does not apply: d10 declares
        // no quantized field. §6 makes `exact` the default and requires the
        // domain document to justify anything else, and
        // `domains/d10_fuel_live.md` records no precision at all. Both of §6's
        // vetoes also have live edges here: the dossier's stated use of
        // `current`/`max` is the fuel *ratio* (a quotient of two rounded values
        // is not the rounded quotient), and `gain`/`cost` are per-hour rates a
        // viewer integrates between samples. Under-quantizing costs bytes;
        // over-quantizing corrupts what the replay draws, and only the second is
        // unrecoverable.
        let c = collection();
        for f in c.value_fields() {
            assert_eq!(f.kind, Kind::Continuous, "field {} is a measure", f.field_id);
            assert_eq!(f.quant, Quant::Exact, "field {} must stay exact", f.field_id);
        }
        assert_eq!(c.value_fields().count(), 4);
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d10 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        assert_eq!(
            record_from_row(&c, &full_row(), 0).expect("maps"),
            record_from_row(&c, &full_row(), 987_654_321).expect("maps"),
            "the tick is taken and ignored"
        );
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
    fn the_mapper_emits_values_for_a_full_row() {
        let c = collection();
        let r = record_from_row(&c, &full_row(), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CURRENT)],
            Value::Continuous(23_717_908_696.0 / 32_768.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::MAX)],
            Value::Continuous(24_002_560_000.0 / 32_768.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::GAIN)],
            Value::Continuous(200_000.0 / 32_768.0)
        );
        // The reference capture's `cost` is 0 in every one of its 24,008 rows:
        // a measured Zero, never an empty state (§3).
        assert_eq!(r.values[index_of(&c, field::COST)], Value::Continuous(0.0));
    }

    #[test]
    fn a_zero_row_is_four_measured_zeroes() {
        let c = collection();
        let r = record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, [0; 4]), 0).expect("maps");
        for f in c.value_fields() {
            assert_eq!(
                r.values[index_of(&c, f.field_id)],
                Value::Continuous(0.0),
                "field {} zero is a measured value, not Null/Unknown/Absent",
                f.field_id
            );
            assert!(r.values[index_of(&c, f.field_id)].carries_value());
        }
        // And it survives a round-trip as a value rather than an empty state.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, r);
    }

    #[test]
    fn every_measure_uses_the_32768_scale() {
        // The test that catches a 32768-vs-100000 mapping error: known
        // fixed-point words against their exact f64 readings, each at its own
        // offset so a transposed pair fails too. 3_276_800_000 / 32768 is exactly
        // 100_000; under `FIXED_SCALE` it would read 32_768 instead.
        let c = collection();
        let raws: [i64; 4] = [3_276_800_000, 49_152, 32_768, -16_384];
        let expected: [(u32, f64); 4] = [
            (field::CURRENT, 100_000.0),
            (field::MAX, 1.5),
            (field::GAIN, 1.0),
            (field::COST, -0.5),
        ];
        let r = record_from_row(&c, &raw_row(9, ROW_FLAG_FUEL_PRESENT, raws), 0).expect("maps");
        for (id, want) in expected {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(want),
                "field {id} must read {want} at scale {FUEL_SCALE}"
            );
        }
        // The smallest representable step is 1/32768, not 1/100000: a mapper on
        // the wrong scale cannot reproduce this pair.
        let r = record_from_row(&c, &raw_row(9, ROW_FLAG_FUEL_PRESENT, [1, 2, 3, -1]), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CURRENT)],
            Value::Continuous(1.0 / 32_768.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::COST)],
            Value::Continuous(-1.0 / 32_768.0)
        );
        // The bound the mapper admits divides exactly, with no rounding: the
        // conversion stays inside 2^53.
        let r = record_from_row(
            &c,
            &raw_row(9, ROW_FLAG_FUEL_PRESENT, [MAX_FUEL_RAW, MAX_FUEL_RAW, MAX_FUEL_RAW, -MAX_FUEL_RAW]),
            0,
        )
        .expect("the collector's own bound is admitted, not rejected");
        assert_eq!(
            r.values[index_of(&c, field::CURRENT)],
            Value::Continuous(MAX_FUEL_MEASURE)
        );
        assert_eq!(
            r.values[index_of(&c, field::COST)],
            Value::Continuous(-MAX_FUEL_MEASURE)
        );
    }

    #[test]
    fn the_mapper_rejects_every_row_no_shipping_encoder_can_produce() {
        let c = collection();
        // Wrong length, both directions, including the empty slice and d63's.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
        }
        // Zero tag_db_id (`world_fold.rs::fold_fuel`).
        assert!(record_from_row(&c, &raw_row(0, ROW_FLAG_FUEL_PRESENT, [0; 4]), 0).is_err());
        // Every unknown row flag bit (`domain_data_authority.rs:989`), not one
        // sample.
        for bit in 1..32u32 {
            for flags in [1u32 << bit, ROW_FLAG_FUEL_PRESENT | (1 << bit)] {
                assert!(
                    record_from_row(&c, &raw_row(1, flags, [0; 4]), 0).is_err(),
                    "flag bit {bit} is not in ROW_KNOWN_FLAGS and must be rejected"
                );
            }
        }
        assert_eq!(ROW_KNOWN_FLAGS, ROW_FLAG_FUEL_PRESENT);
        // A negative `current` or `max` (`domain_data_authority.rs:998`-`:1007`,
        // `attach_log.rs:83879`), at -1 and at the extreme.
        for slot in [0usize, 1] {
            for value in [-1i64, i64::MIN, -MAX_FUEL_RAW] {
                let mut measures = [0i64; 4];
                measures[slot] = value;
                assert!(
                    record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, measures), 0).is_err(),
                    "measure {slot} at {value} is negative"
                );
            }
        }
        // Out-of-range magnitudes, every measure, both signs where legal
        // (`attach_log.rs:83859`-`:83885`), and the bound itself accepted so it
        // is not off by one.
        for slot in 0..4usize {
            for value in [MAX_FUEL_RAW + 1, i64::MAX] {
                let mut measures = [0i64; 4];
                measures[slot] = value;
                assert!(
                    record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, measures), 0).is_err(),
                    "measure {slot} at {value} exceeds the collector's bound"
                );
                let mut measures = [0i64; 4];
                measures[slot] = MAX_FUEL_RAW;
                assert!(
                    record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, measures), 0).is_ok(),
                    "measure {slot} at the bound itself is admitted"
                );
            }
            if slot >= 2 {
                let mut measures = [0i64; 4];
                measures[slot] = -MAX_FUEL_RAW - 1;
                assert!(
                    record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, measures), 0).is_err(),
                    "signed measure {slot} below -bound"
                );
                let mut measures = [0i64; 4];
                measures[slot] = i64::MIN;
                assert!(
                    record_from_row(&c, &raw_row(1, ROW_FLAG_FUEL_PRESENT, measures), 0).is_err(),
                    "signed measure {slot} at i64::MIN must not overflow an abs()"
                );
            }
        }
    }

    #[test]
    fn a_cleared_presence_bit_maps_exactly_as_the_fold_maps_it() {
        // Round-1 review input, generalized. `fold_fuel` never reads the row
        // flags word and reads all four measures unconditionally
        // (`world_fold.rs::fold_fuel`), and `domain_data_authority.rs:989`
        // rejects only bits outside bit 0 — so a cleared bit 0 is a row both
        // authorities accept, and refusing it here would be a rejection with
        // nothing behind it. The mapped record must be identical to the one the
        // same measures produce with the bit set: the word carries no value.
        let c = collection();
        let measures = [32_768i64, 65_536, 200_000, -1];
        let with = record_from_row(&c, &raw_row(3, ROW_FLAG_FUEL_PRESENT, measures), 0)
            .expect("maps with the bit set");
        let without =
            record_from_row(&c, &raw_row(3, 0, measures), 0).expect("maps with the bit clear");
        assert_eq!(with, without, "bit 0 does not change any mapped value");
        assert_eq!(
            with.values[index_of(&c, field::CURRENT)],
            Value::Continuous(1.0)
        );
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_structural_rules() {
        let rows = full_row();
        let good = payload(std::slice::from_ref(&rows));
        let (header, body) = split_payload(&good).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.changed_rows, 1);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.scale_raw, FUEL_SCALE_RAW);
        assert_eq!(body, &rows[..]);

        // An empty payload is legal and carries no rows: the reference capture
        // has one such hour, and the verification gates build exactly this
        // (`lanes/compact_replay_verification_gates/src/lib.rs:7535`).
        let empty = payload(&[]);
        let (header, body) = split_payload(&empty).expect("a zero-row payload is well-formed");
        assert_eq!(header.row_count, 0);
        assert!(body.is_empty());

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // Length that does not match the row count, in both directions.
        let mut truncated = good.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        let mut extended = good.clone();
        extended.push(0);
        assert!(split_payload(&extended).is_err());
        // A wrong scale word (`domain_data_authority.rs:949`) — including
        // `FIXED_SCALE`'s 100_000, the exact confusion this guards.
        for wrong in [0u32, 1, 100_000, 65_536] {
            let mut patched = good.clone();
            patched[20..24].copy_from_slice(&wrong.to_le_bytes());
            assert!(
                split_payload(&patched).is_err(),
                "scale {wrong} must be refused"
            );
        }
        // changed_rows: the authority's rule is `<= row_count`
        // (`domain_data_authority.rs:962`), so a payload that reports fewer
        // changed rows than it carries is accepted — the fold accepts it too, and
        // refusing it would be a rejection with no authority behind it. Only a
        // count above the row count is refused. Round-1 review input.
        for allowed in [0u32, 1] {
            let mut patched = good.clone();
            patched[12..16].copy_from_slice(&allowed.to_le_bytes());
            let (header, _) = split_payload(&patched)
                .unwrap_or_else(|e| panic!("changed_rows {allowed} must be accepted: {e}"));
            assert_eq!(header.changed_rows, allowed);
        }
        for wrong in [2u32, u32::MAX] {
            let mut patched = good.clone();
            patched[12..16].copy_from_slice(&wrong.to_le_bytes());
            assert!(split_payload(&patched).is_err(), "changed_rows {wrong}");
        }
        // A tag_db_id repeated inside one payload (`domain_data_authority.rs:984`):
        // the fold's upsert would take the last writer silently and a keyed set
        // cannot hold both. Round-1 review input. Two DIFFERENT tags in the same
        // shape are accepted, so the check is about identity, not about row count.
        let two_same = payload(&[full_row(), full_row()]);
        assert!(
            split_payload(&two_same).is_err(),
            "a payload repeating tag_db_id 1 must be refused"
        );
        let two_different = payload(&[
            full_row(),
            raw_row(2, ROW_FLAG_FUEL_PRESENT, [1, 2, 3, -1]),
        ]);
        let (header, body) = split_payload(&two_different).expect("distinct tags are fine");
        assert_eq!(header.row_count, 2);
        assert_eq!(body.len(), 2 * ROW_LEN);
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
        // Total over arbitrary bytes: a diagnosis, never a panic.
        for len in 0..(HEADER_LEN + 2 * ROW_LEN) {
            let bytes = vec![0xFFu8; len];
            let _ = split_payload(&bytes);
        }
    }

    #[test]
    fn the_key_is_the_tag_db_id() {
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[1]));
        assert_eq!(
            key_from_row(&raw_row(u32::MAX, ROW_FLAG_FUEL_PRESENT, [0; 4])).expect("keys"),
            key(&[u32::MAX as u64])
        );
        assert!(key_from_row(&raw_row(0, ROW_FLAG_FUEL_PRESENT, [0; 4])).is_err());
        assert!(key_from_row(&[]).is_err());
    }
}
