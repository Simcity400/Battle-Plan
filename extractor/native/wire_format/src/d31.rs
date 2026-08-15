//! d31 `air_region_activity` — per-region air activity, aggregated into
//! `(region, owner, mission)` buckets.
//!
//! Derived from `world_fold.rs:20003`-`:20151`
//! (`fold_air_region_activity`), its payload constants at `:977`-`:978`, the
// @world-fold-authority domain=31 functions=fold_air_region_activity lines=20003-20151 sha256=b812fe069d8065790671c8009998762b2c176e34cf2c8c1d4ded33824b7fa0ec closure=895665c5a6537cf1963647af4bf7842ded4f7d56e005fdcac1b941514e4908bc
//! accumulator's own JSON projection (`world_fold.rs::insert_air_region_activity`), and the collector read
//! that builds the buckets and the delta rows
//! (`extractor/native/collector/src/attach_log.rs:48122`-`:48160`,
//! `:55870`-`:55963`, `:70192`-`:70225`). There is no
//! `extractor/documents/domains/d31_*.md` dossier, so the fold and the collector
//! read are the whole basis; where they could be read differently the fold wins.
//!
//! **Payload grammar** (`world_fold.rs::fold_air_region_activity`, collector
//! `attach_log.rs:70200`-`:70208`): a 36-byte header — `elapsed_wall_ms` u64 at
//! `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at `+12`, `status_flags`
//! u32 at `+16`, `observed_air_wing_rows` u32 at `+20`, `observed_bucket_rows`
//! u32 at `+24`, `deleted_rows` u32 at `+28`, `reserved0` u32 at `+32` — followed
//! by `row_count` fixed 32-byte rows. The fold requires the length to be exactly
//! `36 + row_count * 32` (`:14803`-`:14810`), so no legacy short row exists and
//! the length selects nothing.
//!
//! Row (`world_fold.rs::fold_air_region_activity`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `region_id`         | raw | none (key component 0) |
//! | `+4`  | u32 | `owner_tag_db_id`   | raw | none (key component 1) |
//! | `+8`  | u32 | `mission_mask`      | raw | none (key component 2) |
//! | `+12` | u32 | `flags`             | —   | only `1<<0` DELETED is legal (`world_fold.rs::fold_air_region_activity`) |
//! | `+16` | u32 | `wing_count`        | raw | none |
//! | `+20` | u32 | `plane_count`       | raw | none |
//! | `+24` | u32 | `effective_planes`  | raw | none |
//! | `+28` | u32 | `reserved`          | —   | must be zero (`world_fold.rs::fold_air_region_activity`) |
//!
//! **No scales anywhere.** All three counters are read raw by the fold
//! (`world_fold.rs::fold_air_region_activity`) and emitted raw by its JSON projection (`world_fold.rs::insert_air_region_activity`).
//! Upstream they are `u32` sums over the air-wing native sample —
//! `wing_count` counts wings, `plane_count` sums `CAirWing::count` and
//! `effective_planes` sums `CAirWing::effective_planes`, both read as plain
//! `u32` and range-filtered against `MAX_AIR_WING_PLANES = 10_000_000`
//! (`attach_log.rs:57115`-`:57118`, summed at `:55894`-`:55896`). No fixed-point
//! word, no scale header field, nothing divided at any layer.
//!
//! **The key is the fold's own triple** `(region_id, owner_tag_db_id,
//! mission_mask)` (`world_fold.rs::fold_air_region_activity`), and it is the same triple the
//! collector aggregates by (`attach_log.rs:55883`). The mission bitmask is a KEY
//! COMPONENT, not a payload field: it selects which bucket a wing's planes are
//! counted into, so two masks are two different rows rather than one row whose
//! mask changed. That is legal — a `u32` is concrete and totally ordered — but it
//! means key churn tracks mission changes rather than object lifetimes, so a wing
//! switching mission is a delete plus an insert rather than an update. Measured
//! on `runs/batch2/deepwar_t1` (146 hours, ~109 live buckets): 1.17 inserts,
//! 6.61 updates and 0.41 deletes per delta hour.
//!
//! **Four states: none.** Every value field is unconditionally present in every
//! non-deleted row. There is no presence flag word — the only flag bit the fold
//! accepts is DELETED (`:14822`-`:14824`), and that is a row-level *removal*
//! (`:14834`-`:14837`), not a field gate. So no field declares `.optional()`,
//! `.nullable()` or `.unknown_capable()`, and a zero is always a real measured
//! zero (22 rows of the capture carry `effective_planes == 0` beside a non-zero
//! `plane_count`, which is a fact about worn-out wings, not an empty state).
//! A deleted row maps to a wire tombstone; [`record_from_row`] refuses one rather
//! than inventing a zero-valued record, because the collector packs
//! `wing_count = plane_count = effective_planes = 0` into a delete row
//! (`attach_log.rs:55952`-`:55963`) and handing that back would turn a delete
//! into an upsert to the origin.
//!
//! **Sparse-after-first, not baseline-plus-delta.** `replace_or_sparse_after_first`
//! (`world_fold.rs::replace_or_sparse_after_first`, called at `:14811`) clears the accumulator only for the
//! FIRST record that reaches it without the `COMPACT_STATUS_DELTA_DATA` bit; every
//! record after that — delta-flagged or not — is applied sparsely, upserting the
//! rows it carries and deleting the keys it tombstones, without clearing. So a
//! single record is not a state, and the frame-level fold lives here as
//! [`Accumulator`] rather than in a harness. Measured on the capture: 1 record
//! with `status_flags == 0` followed by 145 with `COMPACT_STATUS_DELTA_DATA`,
//! 23 of which carry zero rows.
//!
//! **The one exception is a DELIBERATE RESYNC.** A record carrying
//! `COMPACT_STATUS_BASELINE_CLEAR` (bit 15) asks the fold to CLEAR d31's state
//! and apply the payload whole, and d31 is on
//! `compact_contract::COMPACT_BASELINE_CLEAR_ADMISSIONS` because its rows are
//! aggregates over the whole live wing walk: a record with no failure bit is a
//! complete claim, so an absent bucket means empty rather than unmeasured. The
//! producer emits it only after its own rejected bracket wiped a pin the fold
//! already had a baseline for. `DELTA_WIRE_DESIGN.md` §10 is the whole rule;
//! [`Accumulator::fold_payload`] takes it as the `baseline_clear` parameter.
//!
//! **Two paths return without folding**, and they are the caller's to honour
//! because both are decided before any row is read:
//!
//! 1. failure status (`handle_compact_failure`, `:14789`-`:14795`) — the
//!    domain-hour is marked unknown and the accumulator is RETAINED. d31 has no
//!    expected-partial tolerance (`compact_contract/src/lib.rs:136`-`:157` lists
//!    26, 27, 28 and 54 only), so every bit of `COMPACT_STATUS_FAILURE_MASK`,
//!    `COMPACT_STATUS_PARTIAL_DATA` included, is a plain failure here.
//! 2. delta-before-baseline (`reject_delta_before_baseline`, `:14796`-`:14802`) —
//!    same treatment. [`Accumulator::fold_payload`] owns this one, because whether
//!    a baseline has been seen is state this type holds.
//!
//! **The exclusions sit between two groups of structural checks**, which is why
//! the header read is split in two. `need`, the two `ensure_count_at_most` calls,
//! `ensure_row_count_with_deletes` and the `reserved0` check all run BEFORE the
//! exclusions (`:14754`-`:14788`) and are hard errors even on a failure-status
//! record; the exact-length check runs AFTER them (`:14803`), so a failure-status
//! record whose length disagrees with its `row_count` is accepted by the fold and
//! must be accepted here. [`peek_header`] is the first group, [`split_payload`]
//! adds the second.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: the three counters are re-derived from the current air-wing sample every
//! frame. `frame_tick` is taken and ignored so every domain's boundary has one
//! shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 31;
/// The one collection: one row per `(region, owner, mission)` activity bucket.
pub const REGION_BUCKETS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const REGION_ID: u32 = 1;
    pub const OWNER_TAG_DB_ID: u32 = 2;
    pub const MISSION_MASK: u32 = 3;
    pub const WING_COUNT: u32 = 4;
    pub const PLANE_COUNT: u32 = 5;
    pub const EFFECTIVE_PLANES: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs:977`-`:978` and `:14754`-`:14848`. --

/// `AIR_REGION_ACTIVITY_HEADER_LEN`, `world_fold.rs::AIR_REGION_ACTIVITY_HEADER_LEN`.
pub const HEADER_LEN: usize = 36;
/// `AIR_REGION_ACTIVITY_ROW_LEN`, `world_fold.rs::AIR_REGION_ACTIVITY_ROW_LEN`. Fixed; no legacy form.
pub const ROW_LEN: usize = 32;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`, and the collector's
/// `COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED` (`attach_log.rs:11336`).
pub const ROW_FLAG_DELETED: u32 = 1;
/// The whole legal flag vocabulary of a d31 row: `world_fold.rs::fold_air_region_activity` rejects
/// the frame outright when any other bit is set, so an unrecognised flag word
/// means the row is not this grammar and nothing in it can be trusted.
pub const ROW_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED;

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // The key is the fold's own triple, in the fold's own order
        // (`world_fold.rs::fold_air_region_activity`), which is also the row order.
        //
        // `region_id` is a strategic air region — map content a mod set defines,
        // so a content manifest must resolve it for the id to keep its meaning
        // (§5's deciding question). ContentId, like d20's `location_province`.
        Field::new(f::REGION_ID, Kind::ContentId).key(0),
        // A country tag-db id, classed as ContentId by d20 (`owner`), d62 and
        // d63 for the same reason. The fold's own projection resolves it through
        // the country reference table (`world_fold.rs::insert_air_region_activity`), which is the
        // manifest lookup in its shipping form.
        Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(1),
        // The wing's mission bitmask. `Unsigned`, NOT `ContentId`: air mission
        // types are engine constants — no mod-set manifest resolves bit 9 into a
        // different mission — which is exactly d20's `div_type` case, an engine
        // discriminator that is stable everywhere and still not content. It is
        // also not `Enum`: §7.4 admits only `Unsigned`, `Signed` and `ContentId`
        // as key kinds, and a bitmask is not a code in `0..n` anyway — the fold
        // constrains it not at all (`:14820` reads the word and uses it whole),
        // so any of the 2^32 words is a legal bucket. Measured on the capture:
        // nine distinct values, `{0, 1, 2, 4, 8, 512, 1024, 16384, 65536}`, one
        // bit or none set in each.
        Field::new(f::MISSION_MASK, Kind::Unsigned).key(2),
        // The three counters, all `u32` read raw (`world_fold.rs::fold_air_region_activity`).
        //
        // `Unsigned`, not `Monotone`: §4 makes monotone a hint that a value
        // usually rises, and none of these does. Each is recomputed from scratch
        // every frame as a sum over the wings currently flying that mission in
        // that region (`attach_log.rs:55893`-`:55896`), so it falls whenever
        // planes are lost or a wing re-tasks — the measured capture has 6.61
        // updates per delta hour in both directions. They count a current
        // population, not a lifetime total.
        Field::new(f::WING_COUNT, Kind::Unsigned),
        Field::new(f::PLANE_COUNT, Kind::Unsigned),
        // Not bounded by `plane_count`: 5 rows of the capture carry
        // `effective_planes > plane_count`. The engine's own effectiveness
        // measure, whatever its scale, is not a subset count — which is one more
        // reason it rides as its own raw field rather than as a ratio.
        Field::new(f::EFFECTIVE_PLANES, Kind::Unsigned),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: REGION_BUCKETS,
            // The fold upserts and deletes by key against a retained map
            // (`world_fold.rs::fold_air_region_activity`) — a keyed set is what that is, and
            // only a keyed set has a delta grammar (§7.3).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 36-byte payload header (`world_fold.rs::fold_air_region_activity`, collector
/// `attach_log.rs:70200`-`:70208`).
///
/// `observed_air_wing_rows` at `+20` is read by no authority — the fold skips
/// straight from `+16` to `+24` (`:14761`-`:14762`) — but it is part of the
/// grammar the collector writes, so it is carried rather than silently dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_air_wing_rows: u32,
    pub observed_bucket_rows: u32,
    pub deleted_rows: u32,
}

/// Read the 36-byte payload header and apply exactly the checks
/// `fold_air_region_activity` applies BEFORE its two return-without-folding
/// paths (`world_fold.rs::fold_air_region_activity`).
///
/// The split exists because the fold's own order is load-bearing: these checks
/// reject a frame even when its status marks the domain-hour failed, while the
/// exact-length check in [`split_payload`] runs only after both exclusions have
/// passed (`world_fold.rs::fold_air_region_activity`). A caller needs `status_flags` to decide the exclusions, and
/// `status_flags` lives in this header — exposing the header read is what keeps
/// that from becoming a second, hand-rolled copy of the header layout in every
/// caller.
///
/// Total: no input of any length panics.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    // `need(payload, AIR_REGION_ACTIVITY_HEADER_LEN, ...)`, `world_fold.rs::fold_air_region_activity`.
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_region_activity payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_air_wing_rows: u32_at(payload, 20),
        observed_bucket_rows: u32_at(payload, 24),
        deleted_rows: u32_at(payload, 28),
    };
    // `ensure_count_at_most`, `world_fold.rs:14765` and `:14772`. Note this is
    // `<=`, not `==`: d31 is NOT a full-replace domain, so a frame serializing
    // fewer rows than it counts is ordinary. (The shipping collector happens to
    // set the two equal — `row_count` is `rows.len()` and `changed_rows` is
    // `emitted.len()` over the same vector, `attach_log.rs:70202`-`:70203` and
    // `:55945`-`:55949`, and all 146 records of the capture agree — but enforcing
    // an equality no authority states would reject healthy frames from any other
    // emitter.)
    if header.changed_rows > header.row_count {
        return Err(format!(
            "air_region_activity changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "air_region_activity deleted_rows {} > row_count {}",
            header.deleted_rows, header.row_count
        ));
    }
    // `ensure_row_count_with_deletes`, `world_fold.rs:14779` / `:30228`: the
    // serialized rows are live buckets plus tombstones, so they cannot outnumber
    // the buckets observed plus the deletions declared. Widened to `u64` because
    // the fold's `saturating_add` on `usize` cannot overflow on a 64-bit host but
    // this arithmetic is on `u32` words from a file.
    let allowed = u64::from(header.observed_bucket_rows) + u64::from(header.deleted_rows);
    if u64::from(header.row_count) > allowed {
        return Err(format!(
            "air_region_activity row_count {} > observed_bucket_rows {} + deleted_rows {}",
            header.row_count, header.observed_bucket_rows, header.deleted_rows
        ));
    }
    // `world_fold.rs::fold_air_region_activity`.
    let reserved0 = u32_at(payload, 32);
    if reserved0 != 0 {
        return Err(format!(
            "air_region_activity header reserved field is nonzero ({reserved0})"
        ));
    }
    Ok(header)
}

/// Split a d31 DATA payload into its header and its rows slice: [`peek_header`]
/// plus the exact-length check the fold makes after its exclusions
/// (`world_fold.rs::fold_air_region_activity`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing. Per-row semantics stay
/// in [`parse_row`] and [`record_from_row`].
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is arbitrary input from a stream on disk,
    // and `row_count * ROW_LEN` overflows `usize` for a large enough word. The
    // fold saturates instead (`:14804`), which rejects the same payloads by way
    // of the length comparison; checked multiplication says so directly.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "air_region_activity row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "air_region_activity len {} != {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// One parsed 32-byte row.
///
/// The three counters are carried even on a deleted row, where the fold ignores
/// them (`world_fold.rs::fold_air_region_activity` removes the key and never reads past
/// `+12`). They are `0` in every delete row the shipping collector emits
/// (`attach_log.rs:55952`-`:55963`) but the fold does not require that, so this
/// does not either — rejecting a row the fold accepts would drop real data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Row {
    pub region_id: u32,
    pub owner_tag_db_id: u32,
    pub mission_mask: u32,
    pub deleted: bool,
    pub wing_count: u32,
    pub plane_count: u32,
    pub effective_planes: u32,
}

/// Parse one 32-byte row, applying every per-row check the fold applies, in the
/// fold's order (`world_fold.rs::fold_air_region_activity`).
///
/// **No key component has a range check**, and that is the fold's decision, not
/// an omission here: `world_fold.rs::fold_air_region_activity` read the three words and use them whole.
/// The collector will not produce a `region_id` outside `1..=65_535`
/// (`attach_log.rs:55881`-`:55883`, filtered against
/// `MAX_AIR_WING_MISSION_REGION_ID`) nor a zero `owner_tag_db_id`, and the
/// capture contains none — but the fold is the authority and it accepts them, so
/// a stricter reading here would refuse frames the recorder folds.
///
/// Total: no input of any length panics.
pub fn parse_row(row: &[u8]) -> core::result::Result<Row, String> {
    check_row_len(row)?;
    let region_id = u32_at(row, 0);
    let owner_tag_db_id = u32_at(row, 4);
    let mission_mask = u32_at(row, 8);
    let flags = u32_at(row, 12);
    // `world_fold.rs::fold_air_region_activity`.
    if flags & !ROW_KNOWN_FLAGS != 0 {
        return Err(format!(
            "air_region_activity row has unknown flags {flags:#x}"
        ));
    }
    // `world_fold.rs::fold_air_region_activity`. Read after the flag check, as the fold does.
    let reserved = u32_at(row, 28);
    if reserved != 0 {
        return Err(format!(
            "air_region_activity row reserved field is nonzero ({reserved})"
        ));
    }
    Ok(Row {
        region_id,
        owner_tag_db_id,
        mission_mask,
        deleted: flags & ROW_FLAG_DELETED != 0,
        wing_count: u32_at(row, 16),
        plane_count: u32_at(row, 20),
        effective_planes: u32_at(row, 24),
    })
}

/// The key of one compact row: the fold's `(region_id, owner_tag_db_id,
/// mission_mask)` triple (`world_fold.rs::fold_air_region_activity`), in the fold's order,
/// which is also the row order.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_row(row)?;
    Ok(row_key(&parsed))
}

/// The key of an already-parsed row.
pub fn row_key(row: &Row) -> Key {
    key(&[
        row.region_id as u64,
        row.owner_tag_db_id as u64,
        row.mission_mask as u64,
    ])
}

/// One compact row of the `region_buckets` table, as the collector emits it
/// today, onto its stored Record. `row` is the raw 32-byte row slice (the caller
/// has already consumed the payload header via [`split_payload`]); d31 has
/// exactly one row form, so length selects nothing and a wrong length is
/// rejected. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); d31 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_air_region_activity`: a row the fold
/// would reject is an `Err` here, never a coerced value. A **deleted** row is an
/// `Err` too — it is a removal (`world_fold.rs::fold_air_region_activity`) and has no
/// record; a caller maps it to a wire tombstone, and quietly handing one back a
/// record of the zeros the collector packs into a delete row would turn a delete
/// into an upsert. Nothing in this function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d31 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_row(row)?;
    record_from_parsed(collection, &parsed)
}

/// [`record_from_row`] for an already-parsed row, so a caller that needs both the
/// key and the record parses the bytes once.
pub fn record_from_parsed(
    collection: &Collection,
    parsed: &Row,
) -> core::result::Result<Record, String> {
    if parsed.deleted {
        return Err(format!(
            "air_region_activity row region={} owner={} mask={:#x} is a delete row, not a record",
            parsed.region_id, parsed.owner_tag_db_id, parsed.mission_mask
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // No gate, no scale: `world_fold.rs::fold_air_region_activity` reads each word
            // and stores it. A zero is a real measured zero.
            field::WING_COUNT => Value::Unsigned(parsed.wing_count as u64),
            field::PLANE_COUNT => Value::Unsigned(parsed.plane_count as u64),
            field::EFFECTIVE_PLANES => Value::Unsigned(parsed.effective_planes as u64),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "air_region_activity has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The shipping incremental layer ------------------------------------------

/// What one d31 DATA record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The FIRST record to reach `replace_or_sparse_after_first` without the
    /// delta bit (`world_fold.rs::replace_or_sparse_after_first`, called at `:14811`): the map was cleared
    /// and refilled, and the domain now has a baseline.
    FullReplace,
    /// Applied sparsely on top of the retained map — rows upserted, tombstoned
    /// keys removed, everything else left alone. This is EVERY record after the
    /// first, whether or not it carries the delta bit.
    Sparse,
    /// A delta-flagged record that arrived before any baseline.
    /// `reject_delta_before_baseline` (`world_fold.rs::reject_delta_before_baseline`, called at `:14796`)
    /// marks the domain-hour unknown, folds **nothing**, and retains the
    /// accumulator. The hour carries no d31 data and belongs in no measurement.
    DeltaBeforeBaseline,
}

/// The running d31 state, replaying `fold_air_region_activity`'s incremental
/// grammar.
///
/// This is what makes d31 different from a full-replace domain, so it lives
/// beside the schema rather than in a harness: the measurement bin, the tests and
/// any future caller share one implementation of clear-on-first,
/// sparse-thereafter, delete-by-flag and delta-before-baseline.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `region_buckets` collection's current content, keyed by
    /// `(region_id, owner_tag_db_id, mission_mask)`.
    pub rows: KeyedState,
    baseline_seen: bool,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a record has already cleared-and-refilled the map — the
    /// `delta_baselines_seen` membership `replace_or_sparse_after_first`
    /// (`world_fold.rs::replace_or_sparse_after_first`) and `reject_delta_before_baseline` (`world_fold.rs::reject_delta_before_baseline`) both
    /// consult. It is set by the first non-delta record that gets past both
    /// exclusions, and by nothing else.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d31 DATA payload, exactly as `fold_air_region_activity` folds it
    /// from `:14796` onward.
    ///
    /// `is_delta` is the caller's reading of `COMPACT_STATUS_DELTA_DATA` in
    /// `header.status_flags`. It is a parameter rather than a bit tested here
    /// because that constant belongs to `compact_contract` and d31 is not
    /// admitted to the production-codec dependency surface. Copying the literal
    /// into this module is exactly the drift the shared-contract rule forbids.
    ///
    /// **The caller must already have excluded failure-status records.** The fold
    /// returns at `:14789` before reaching any of this, retaining the accumulator;
    /// the predicate is `compact_contract`'s
    /// `compact_status_failure_flags_after_expected_partial_tolerance`, never a
    /// hand-copied mask. Delta-before-baseline is handled here instead, because
    /// `baseline_seen` is state this type holds.
    ///
    /// `frame_tick` is §6's derived-field input; d31 declares no derived field.
    ///
    /// **Atomic on `Err`.** Every row is parsed and mapped before anything is
    /// written, so a payload this rejects leaves the accumulator exactly as it
    /// found it. The fold itself is not atomic — it clears and then fails partway
    /// through the row loop — but every one of its `Err`s aborts the stream fold
    /// (`world_fold.rs::fold_domain_data` returns the error to its caller), so no reader ever
    /// observes the difference, and the stricter property is the safer one for a
    /// caller that reports and stops.
    pub fn fold_payload(
        &mut self,
        collection: &Collection,
        payload: &[u8],
        is_delta: bool,
        baseline_clear: bool,
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        // d31 has no derived field; §6's transform input is unused. Named rather
        // than `_frame_tick` because the signature is the cross-domain contract.
        let _ = frame_tick;
        // `world_fold.rs::fold_air_region_activity`: the header-structural checks that
        // precede both exclusions.
        peek_header(payload)?;
        // `:14796`-`:14802`. Decided before the exact-length check and before
        // anything is cleared, so the accumulator survives untouched — and a
        // payload whose length disagrees with its `row_count` is NOT an error on
        // this path, because the fold never reaches `:14803`.
        if is_delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }
        // `:14803`-`:14810`.
        let (_, rows) = split_payload(payload)?;

        // Parse and map everything first, so a structural error leaves the
        // accumulator as it was.
        let mut parsed: Vec<(Key, Row, Option<Record>)> = Vec::with_capacity(rows.len() / ROW_LEN);
        for row in rows.chunks_exact(ROW_LEN) {
            let fields = parse_row(row)?;
            let record = if fields.deleted {
                None
            } else {
                Some(record_from_parsed(collection, &fields)?)
            };
            parsed.push((row_key(&fields), fields, record));
        }

        // Only the first non-delta record clears -- UNLESS the record asks for
        // a DELIBERATE RESYNC. `COMPACT_STATUS_BASELINE_CLEAR` is the one thing
        // that makes a later whole payload a replacement rather than a merge,
        // and d31 is on `COMPACT_BASELINE_CLEAR_ADMISSIONS` because its rows are
        // aggregates over the whole wing walk: a clean record is a complete
        // claim, so replacing what the fold holds is always sound for it.
        //
        // `baseline_clear` is a parameter for the same reason `is_delta` is: the
        // constant belongs to `compact_contract` and this module is not on that
        // dependency surface, so copying the literal here would be exactly the
        // drift the shared-contract rule forbids. The caller must ALSO have
        // refused the contradictory `BASELINE_CLEAR | DELTA_DATA` pair and a
        // resync on a failure-class record; `compact_baseline_clear_requested`
        // is the single predicate that decides all three, and
        // `world_fold.rs::baseline_clear_requested` is where production asks it.
        let sparse = (is_delta || self.baseline_seen) && !baseline_clear;
        if !sparse {
            self.rows.clear();
        }
        // `:14816`-`:14849`. Rows are applied in payload order, so a key repeated
        // inside one frame resolves the way the fold's `BTreeMap` resolves it —
        // last writer wins, and a delete after an insert of the same key in one
        // frame removes it. The fold has no duplicate rejection, so neither does
        // this; the capture contains no intra-frame duplicate.
        for (k, fields, record) in parsed {
            if fields.deleted {
                // `:14835`: a tombstone for a key that is not there is a no-op,
                // not an error.
                self.rows.remove(&k);
            } else {
                self.rows.insert(k, record.expect("a live row has a record"));
            }
        }
        if !sparse {
            self.baseline_seen = true;
            Ok(FoldOutcome::FullReplace)
        } else {
            Ok(FoldOutcome::Sparse)
        }
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_region_activity row length {} != {ROW_LEN}",
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
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
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

    /// A raw 32-byte row, built the way the collector packs one
    /// (`attach_log.rs:70213`-`:70224`).
    fn raw_row(
        region_id: u32,
        owner_tag_db_id: u32,
        mission_mask: u32,
        flags: u32,
        wing_count: u32,
        plane_count: u32,
        effective_planes: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [
            region_id,
            owner_tag_db_id,
            mission_mask,
            flags,
            wing_count,
            plane_count,
            effective_planes,
            0,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    fn full_row() -> Vec<u8> {
        raw_row(37, 12, 512, 0, 4, 76, 61)
    }

    /// A d31 DATA payload with the collector's own header arithmetic:
    /// `row_count == changed_rows == rows.len()` and
    /// `observed_bucket_rows = live buckets`.
    fn payload(status_delta_ignored: u32, rows: &[Vec<u8>], observed_bucket_rows: u32) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| u32_at(r, 12) & ROW_FLAG_DELETED != 0)
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_delta_ignored.to_le_bytes()); // status_flags
        p.extend_from_slice(&0u32.to_le_bytes()); // observed_air_wing_rows
        p.extend_from_slice(&observed_bucket_rows.to_le_bytes());
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        for r in rows {
            p.extend_from_slice(r);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d31 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_region_owner_mission_triple() {
        let c = collection();
        assert_eq!(
            c.shape,
            Shape::KeyedSet,
            "the fold upserts and deletes against a retained map"
        );
        let keys = c.key_fields();
        assert_eq!(keys.len(), 3, "world_fold.rs:14755 keys rows by the triple");
        assert_eq!(keys[0].field_id, field::REGION_ID);
        assert_eq!(keys[1].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[2].field_id, field::MISSION_MASK);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "an air region is map content a manifest must resolve (§5)"
        );
        assert_eq!(
            keys[1].kind,
            Kind::ContentId,
            "a country tag-db id is content, as d20/d62/d63 class it"
        );
        assert_eq!(
            keys[2].kind,
            Kind::Unsigned,
            "the mission bitmask is an engine discriminator, not content"
        );
        for k in &keys {
            assert!(k.kind.legal_as_key(), "field {} is a legal key kind", k.field_id);
        }
        assert_eq!(c.fields.len(), 6, "3 key components + 3 counters");
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // Recipe step 8 case 3: the schema declares NO capability, so there is no
        // absent/null/unknown case to build a raw row for. The reason is the row
        // grammar itself — the only flag bit the fold accepts is DELETED
        // (`world_fold.rs::fold_air_region_activity`), which removes the key rather than gating a
        // field, so every value field is unconditionally present in every row
        // that becomes a record. `a_real_zero_is_a_value` and
        // `the_mapper_reads_every_field_at_its_own_offset` cover the two states
        // the mapper can actually reach; `a_delete_row_is_a_tombstone_not_a_record`
        // covers the removal.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field has a capture gate; a failed read fails the whole frame"
        );
        assert!(
            c.fields.iter().all(|f| !f.optional && !f.nullable && !f.unknown_capable),
            "key components are concrete too"
        );
    }

    #[test]
    fn nothing_is_quantized_because_nothing_is_continuous() {
        // Recipe step 8 case 4: the "quantization saves bytes" test does not
        // apply. d31 has no `Continuous` field at all — all three measurements
        // are `u32` counters the fold reads raw (`world_fold.rs::fold_air_region_activity`)
        // — so there is no precision to budget and no rounding to hide.
        let c = collection();
        assert_eq!(
            c.fields
                .iter()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
        assert!(c.fields.iter().all(|f| !f.quant.is_quantized()));
        assert!(c.fields.iter().all(|f| f.quant == Quant::None));
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; the three counters are
        // re-derived from the current air-wing sample every frame, so nothing
        // here changes deterministically with the clock and `frame_tick` never
        // enters a value.
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
    fn an_all_zero_counter_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &raw_row(1, 1, 0, 0, 0, 0, 0), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // The row shape the capture actually contains: planes present, none of
        // them effective. §3 keeps zero out of the three empty states.
        let c = collection();
        let r = record_from_row(&c, &raw_row(9, 3, 1, 0, 2, 30, 0), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::EFFECTIVE_PLANES)],
            Value::Unsigned(0),
            "an all-worn-out bucket measures zero effective planes"
        );
        assert_eq!(r.values[index_of(&c, field::WING_COUNT)], Value::Unsigned(2));
        assert_eq!(
            r.values[index_of(&c, field::PLANE_COUNT)],
            Value::Unsigned(30)
        );
        for v in &r.values {
            assert!(
                v.carries_value(),
                "no value field can reach absent, null or unknown"
            );
        }
    }

    #[test]
    fn the_mapper_reads_every_field_at_its_own_offset() {
        // Recipe step 8 case 6. d31 has no scale, so the failure class this
        // catches is transposition: seven distinct words, each at its own offset,
        // each asserted separately. The three key words are checked through
        // `key_from_row` because they do not ride in the record.
        let c = collection();
        let row = raw_row(0x1111_1111, 0x2222_2222, 0x3333_3333, 0, 44, 55, 66);
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::WING_COUNT)], Value::Unsigned(44));
        assert_eq!(r.values[index_of(&c, field::PLANE_COUNT)], Value::Unsigned(55));
        assert_eq!(
            r.values[index_of(&c, field::EFFECTIVE_PLANES)],
            Value::Unsigned(66)
        );
        assert_eq!(
            key_from_row(&row).expect("keys"),
            key(&[0x1111_1111, 0x2222_2222, 0x3333_3333])
        );

        // And the raw-ness claim, field by field: a word that a `/100_000`,
        // `/32768` or `/1000` reading would change is emitted unchanged. This is
        // the no-scale analogue of the 32768-vs-100000 test.
        let scaled_shaped = raw_row(1, 1, 0, 0, 100_000, 32_768, 1_000);
        let r = record_from_row(&c, &scaled_shaped, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::WING_COUNT)],
            Value::Unsigned(100_000),
            "wing_count is emitted raw by the fold, never divided"
        );
        assert_eq!(
            r.values[index_of(&c, field::PLANE_COUNT)],
            Value::Unsigned(32_768)
        );
        assert_eq!(
            r.values[index_of(&c, field::EFFECTIVE_PLANES)],
            Value::Unsigned(1_000)
        );

        // u32::MAX at every counter offset survives the widening to u64 intact.
        let r = record_from_row(&c, &raw_row(1, 1, 0, 0, u32::MAX, u32::MAX - 1, 1), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::WING_COUNT)],
            Value::Unsigned(u32::MAX as u64)
        );
        assert_eq!(
            r.values[index_of(&c, field::PLANE_COUNT)],
            Value::Unsigned(u32::MAX as u64 - 1)
        );
    }

    #[test]
    fn the_key_is_the_triple_in_the_folds_order() {
        // Transposition again, on the key side: the three key words are adjacent
        // in the row, so a swapped pair must be visible. Distinct values, and one
        // case per position.
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[37, 12, 512]));
        assert_ne!(key_from_row(&full_row()).expect("keys"), key(&[12, 37, 512]));
        assert_eq!(
            key_from_row(&raw_row(u32::MAX, u32::MAX, u32::MAX, 0, 0, 0, 0)).expect("keys"),
            key(&[u32::MAX as u64, u32::MAX as u64, u32::MAX as u64]),
            "no key component is sign-extended or truncated"
        );
        // A delete row still has a key — that is the whole point of a tombstone.
        assert_eq!(
            key_from_row(&raw_row(37, 12, 512, ROW_FLAG_DELETED, 0, 0, 0)).expect("keys"),
            key(&[37, 12, 512])
        );
        assert!(key_from_row(&[]).is_err());
        assert!(key_from_row(&vec![0u8; ROW_LEN - 1]).is_err());
    }

    #[test]
    fn a_delete_row_is_a_tombstone_not_a_record() {
        let c = collection();
        // The collector's own delete row: zeroed counters (`attach_log.rs:55952`).
        assert!(record_from_row(&c, &raw_row(37, 12, 512, ROW_FLAG_DELETED, 0, 0, 0), 0).is_err());
        // And one carrying counters, which the fold accepts and ignores: still a
        // removal, never a record of those numbers.
        assert!(record_from_row(&c, &raw_row(37, 12, 512, ROW_FLAG_DELETED, 9, 9, 9), 0).is_err());
        let parsed = parse_row(&raw_row(37, 12, 512, ROW_FLAG_DELETED, 9, 9, 9)).expect("parses");
        assert!(parsed.deleted);
        assert_eq!(parsed.wing_count, 9, "the fold reads the row, it just ignores it");
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects_and_nothing_else() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 40] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(parse_row(&row).is_err(), "length {len}");
        }
        // Every flag bit that is not DELETED (`world_fold.rs::fold_air_region_activity`) — all 31 of
        // them, not one sample.
        for bit in 1..32u32 {
            let row = raw_row(1, 1, 0, 1 << bit, 0, 0, 0);
            assert!(parse_row(&row).is_err(), "flag bit {bit} must be rejected");
            let row = raw_row(1, 1, 0, ROW_FLAG_DELETED | (1 << bit), 0, 0, 0);
            assert!(
                parse_row(&row).is_err(),
                "DELETED plus bit {bit} must be rejected"
            );
        }
        // A non-zero row reserved word (`:14826`), every bit of it.
        for bit in 0..32u32 {
            let mut row = full_row();
            row[28..32].copy_from_slice(&(1u32 << bit).to_le_bytes());
            assert!(parse_row(&row).is_err(), "reserved bit {bit}");
        }
        // What the fold does NOT reject must be accepted here: no key component
        // has a range check at `:14818`-`:14820`, so a zero or out-of-range key
        // word is data, not an error.
        for row in [
            raw_row(0, 0, 0, 0, 0, 0, 0),
            raw_row(u32::MAX, u32::MAX, u32::MAX, 0, 1, 1, 1),
            raw_row(70_000, 1, 0, 0, 1, 1, 1), // beyond MAX_AIR_WING_MISSION_REGION_ID
        ] {
            assert!(
                record_from_row(&c, &row, 0).is_ok(),
                "the fold accepts this row, so the mapper must too"
            );
        }
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
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![full_row()];
        let p = payload(0, &rows, 1);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.changed_rows, 1);
        assert_eq!(header.observed_bucket_rows, 1);
        assert_eq!(body, &rows[0][..]);

        // Short of the header (`world_fold.rs::fold_air_region_activity`).
        for len in 0..HEADER_LEN {
            assert!(peek_header(&p[..len]).is_err(), "len {len}");
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // A length that does not match the row count (`:14805`). This one is
        // `split_payload` only — `peek_header` mirrors the fold, which has not
        // reached the length check yet.
        let mut truncated = p.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        assert!(peek_header(&truncated).is_ok());
        let mut overlong = p.clone();
        overlong.push(0);
        assert!(split_payload(&overlong).is_err());

        // changed_rows > row_count (`:14765`).
        let mut bad = p.clone();
        bad[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(peek_header(&bad).is_err());
        // changed_rows < row_count is LEGAL: the fold checks `<=`, and d31 is not
        // a full-replace domain.
        let mut partial = p.clone();
        partial[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&partial).is_ok());

        // deleted_rows > row_count (`:14772`).
        let mut bad = p.clone();
        bad[28..32].copy_from_slice(&2u32.to_le_bytes());
        assert!(peek_header(&bad).is_err());

        // row_count > observed_bucket_rows + deleted_rows (`:14779`).
        let mut bad = p.clone();
        bad[24..28].copy_from_slice(&0u32.to_le_bytes()); // observed_bucket_rows
        bad[28..32].copy_from_slice(&0u32.to_le_bytes()); // deleted_rows
        assert!(peek_header(&bad).is_err());
        // ...and the boundary itself is accepted, so the bound is not off by one.
        let mut edge = p.clone();
        edge[24..28].copy_from_slice(&0u32.to_le_bytes());
        edge[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(peek_header(&edge).is_ok());
        // The sum must not wrap: `observed_bucket_rows + deleted_rows` overflows
        // u32 here, and a `u32` addition would make `row_count = 1` look too big.
        let mut wide = p.clone();
        wide[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        wide[28..32].copy_from_slice(&0u32.to_le_bytes());
        assert!(peek_header(&wide).is_ok());

        // A non-zero header reserved word (`:14786`).
        let mut bad = p.clone();
        bad[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(peek_header(&bad).is_err());

        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());

        // A zero-row payload is well-formed: 23 of the capture's 146 records are
        // exactly this.
        let empty = payload(0, &[], 109);
        let (header, body) = split_payload(&empty).expect("an empty delta is a frame");
        assert_eq!(header.row_count, 0);
        assert!(body.is_empty());
    }

    #[test]
    fn peek_header_reads_every_word_at_its_own_offset() {
        // The header counterpart of `the_mapper_reads_every_field_at_its_own_offset`:
        // eight distinct sentinels, one per word, so any transposition of an
        // adjacent pair shows up. `elapsed_wall_ms` and `observed_air_wing_rows`
        // are read by no authority (`world_fold.rs::fold_air_region_activity` skips both),
        // which is exactly why they need a direct assertion — nothing else in the
        // suite would notice if they moved.
        let mut p = Vec::new();
        p.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&3u32.to_le_bytes()); // row_count
        p.extend_from_slice(&2u32.to_le_bytes()); // changed_rows
        p.extend_from_slice(&0x0000_0400u32.to_le_bytes()); // status_flags
        p.extend_from_slice(&77u32.to_le_bytes()); // observed_air_wing_rows
        p.extend_from_slice(&5u32.to_le_bytes()); // observed_bucket_rows
        p.extend_from_slice(&1u32.to_le_bytes()); // deleted_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        assert_eq!(p.len(), HEADER_LEN);
        let h = peek_header(&p).expect("well-formed header");
        assert_eq!(h.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(h.row_count, 3);
        assert_eq!(h.changed_rows, 2);
        assert_eq!(h.status_flags, 0x0000_0400);
        assert_eq!(h.observed_air_wing_rows, 77);
        assert_eq!(h.observed_bucket_rows, 5);
        assert_eq!(h.deleted_rows, 1);
        // The eight values are pairwise distinct where a swap could hide, so a
        // transposed read cannot coincide with the right answer.
        let words = [3u64, 2, 0x400, 77, 5, 1];
        for (i, a) in words.iter().enumerate() {
            for b in &words[i + 1..] {
                assert_ne!(a, b, "sentinels must stay distinct for this test to bite");
            }
        }
    }

    #[test]
    fn peek_header_never_panics_on_arbitrary_bytes() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
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
            let _ = peek_header(&bytes);
            let _ = split_payload(&bytes);
            let _ = parse_row(&bytes);
            let _ = key_from_row(&bytes);
            let _ = record_from_row(&collection(), &bytes, 0);
        }
    }

    // --- The incremental grammar ---------------------------------------------

    #[test]
    fn the_first_non_delta_record_clears_and_every_later_one_does_not() {
        let c = collection();
        let mut acc = Accumulator::new();
        assert!(!acc.baseline_seen());

        // First record, no delta bit: a full replace.
        let first = payload(0, &[full_row(), raw_row(9, 3, 1, 0, 2, 30, 0)], 2);
        assert_eq!(
            acc.fold_payload(&c, &first, false, false, 0).expect("folds"),
            FoldOutcome::FullReplace
        );
        assert!(acc.baseline_seen());
        assert_eq!(acc.rows.len(), 2);

        // A delta record upserts without clearing.
        let second = payload(0, &[raw_row(9, 3, 1, 0, 5, 60, 55)], 2);
        assert_eq!(
            acc.fold_payload(&c, &second, true, false, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows.len(), 2, "the untouched row survives a delta");
        let updated = &acc.rows[&key(&[9, 3, 1])];
        assert_eq!(updated.values[index_of(&c, field::PLANE_COUNT)], Value::Unsigned(60));

        // `replace_or_sparse_after_first` (`world_fold.rs::replace_or_sparse_after_first`): a record with
        // NO delta bit, arriving after the baseline, is still sparse. This is the
        // rule that separates d31 from d22/d23, where a non-delta record always
        // clears.
        let third = payload(0, &[raw_row(1, 1, 0, 0, 1, 1, 1)], 3);
        assert_eq!(
            acc.fold_payload(&c, &third, false, false, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(
            acc.rows.len(),
            3,
            "a non-delta record after the first must NOT clear the map"
        );
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing() {
        let c = collection();
        let mut acc = Accumulator::new();
        let p = payload(0, &[full_row()], 1);
        assert_eq!(
            acc.fold_payload(&c, &p, true, false, 0).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(acc.rows.is_empty(), "nothing was folded");
        assert!(!acc.baseline_seen(), "and no baseline was recorded");

        // It is decided BEFORE the exact-length check (`world_fold.rs::fold_air_region_activity` vs
        // `world_fold.rs::fold_air_region_activity`), so a payload whose length disagrees with its row_count is
        // still not an error on this path — exactly as the fold returns Ok.
        let mut ragged = p.clone();
        ragged.pop();
        assert_eq!(
            acc.fold_payload(&c, &ragged, true, false, 0).expect("the fold returns Ok here"),
            FoldOutcome::DeltaBeforeBaseline
        );
        // But the header-structural checks that PRECEDE the exclusion still bite.
        let mut bad_header = p.clone();
        bad_header[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(acc.fold_payload(&c, &bad_header, true, false, 0).is_err());
    }

    #[test]
    fn a_delete_row_removes_its_key_and_an_absent_one_is_a_no_op() {
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[full_row()], 1), false, false, 0)
            .expect("baseline");
        assert_eq!(acc.rows.len(), 1);

        // `world_fold.rs::fold_air_region_activity`.
        let del = payload(0, &[raw_row(37, 12, 512, ROW_FLAG_DELETED, 0, 0, 0)], 0);
        acc.fold_payload(&c, &del, true, false, 0).expect("folds");
        assert!(acc.rows.is_empty());

        // A tombstone for a key that is already gone is not an error there, and
        // must not be one here.
        acc.fold_payload(&c, &del, true, false, 0).expect("a no-op delete is legal");
        assert!(acc.rows.is_empty());

        // The mask is a KEY component: deleting `(37, 12, 512)` leaves
        // `(37, 12, 1024)` alone even though region and owner match.
        let mut acc = Accumulator::new();
        let two = payload(
            0,
            &[raw_row(37, 12, 512, 0, 1, 1, 1), raw_row(37, 12, 1024, 0, 2, 2, 2)],
            2,
        );
        acc.fold_payload(&c, &two, false, false, 0).expect("baseline");
        acc.fold_payload(&c, &del, true, false, 0).expect("folds");
        assert_eq!(acc.rows.len(), 1);
        assert!(acc.rows.contains_key(&key(&[37, 12, 1024])));
    }

    #[test]
    fn a_repeated_key_in_one_frame_resolves_in_payload_order() {
        // The fold has no duplicate rejection (`world_fold.rs::fold_air_region_activity` inserts into
        // a BTreeMap), so the last row wins and a delete after an insert removes.
        let c = collection();
        let mut acc = Accumulator::new();
        let p = payload(
            0,
            &[raw_row(1, 1, 0, 0, 1, 1, 1), raw_row(1, 1, 0, 0, 7, 7, 7)],
            2,
        );
        acc.fold_payload(&c, &p, false, false, 0).expect("folds");
        assert_eq!(acc.rows.len(), 1);
        assert_eq!(
            acc.rows[&key(&[1, 1, 0])].values[index_of(&c, field::WING_COUNT)],
            Value::Unsigned(7)
        );

        let mut acc = Accumulator::new();
        let p = payload(
            0,
            &[
                raw_row(1, 1, 0, 0, 1, 1, 1),
                raw_row(1, 1, 0, ROW_FLAG_DELETED, 0, 0, 0),
            ],
            1,
        );
        acc.fold_payload(&c, &p, false, false, 0).expect("folds");
        assert!(acc.rows.is_empty(), "the delete came last, so the key is gone");
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[full_row()], 1), false, false, 0)
            .expect("baseline");
        let before = acc.rows.clone();

        // A bad row in the middle of an otherwise good frame.
        let bad = payload(
            0,
            &[
                raw_row(9, 3, 1, 0, 2, 30, 0),
                raw_row(9, 4, 1, 1 << 5, 0, 0, 0), // unknown flag bit
                raw_row(9, 5, 1, 0, 2, 30, 0),
            ],
            3,
        );
        assert!(acc.fold_payload(&c, &bad, true, false, 0).is_err());
        assert_eq!(acc.rows, before, "a rejected delta changed nothing");

        // And on a would-be full replace, so the clear does not happen either.
        let mut fresh = Accumulator::new();
        assert!(fresh.fold_payload(&c, &bad, false, false, 0).is_err());
        assert!(fresh.rows.is_empty());
        assert!(!fresh.baseline_seen(), "a rejected frame is not a baseline");
    }

    #[test]
    fn an_empty_delta_frame_is_a_legal_no_op() {
        // 23 of the capture's 146 records carry zero rows.
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[full_row()], 1), false, false, 0)
            .expect("baseline");
        let before = acc.rows.clone();
        assert_eq!(
            acc.fold_payload(&c, &payload(0, &[], 1), true, false, 0)
                .expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows, before);
    }

    #[test]
    fn an_accumulated_state_survives_the_wire_delta_grammar() {
        // The codec-level counterpart of the measurement bin's reader replay:
        // baseline, then a delta built from the shipping fold's own two states,
        // decoded against the reader's reconstructed prior.
        let schema = schema();
        let c = &schema.collections[0];
        let mut acc = Accumulator::new();
        acc.fold_payload(
            c,
            &payload(
                0,
                &[raw_row(37, 12, 512, 0, 4, 76, 61), raw_row(9, 3, 1, 0, 2, 30, 0)],
                2,
            ),
            false,
            false,
            0,
        )
        .expect("baseline");
        let first = acc.rows.clone();

        let mut bytes = Vec::new();
        encode_baseline(&schema, std::slice::from_ref(&first), &mut bytes).expect("encodes");
        let (states, consumed) = decode_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        let mut reader = states.into_iter().next().unwrap();
        assert_eq!(reader, first);

        // One update, one insert, one delete — the churn shape the capture has.
        acc.fold_payload(
            c,
            &payload(
                0,
                &[
                    raw_row(9, 3, 1, 0, 3, 44, 40),
                    raw_row(37, 12, 512, ROW_FLAG_DELETED, 0, 0, 0),
                    raw_row(50, 12, 2, 0, 1, 10, 10),
                ],
                2,
            ),
            true,
            false,
            0,
        )
        .expect("delta");
        let second = acc.rows.clone();

        let d = diff(c, &first, &second);
        assert_eq!(d.changed.len(), 2);
        assert_eq!(d.tombstones.len(), 1);
        let mut bytes = Vec::new();
        encode_delta(&schema, std::slice::from_ref(&first), &[Some(d)], &mut bytes)
            .expect("encodes");
        let (deltas, consumed) =
            decode_delta(&schema, std::slice::from_ref(&reader), &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        apply(&mut reader, deltas[0].as_ref().unwrap());
        assert_eq!(reader, second, "the reader reconstructs the fold's state");
    }
}
