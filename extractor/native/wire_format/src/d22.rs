//! d22 `map_control_live` — who owns and who controls every state and province.
//!
//! Derived from `world_fold.rs:31151`-`:31275`
//! (`fold_map_control`), its row-length constants at `:386`-`:387`, the projection
// @world-fold-authority domain=22 functions=fold_map_control lines=31151-31275 sha256=78a9bbbfbd40b1faf44e6d45b3a3e9bf164c22d85fecfa8be226543976794ad9 closure=3c027964b2c9e7facf94b02d9b6ebb67b01124fc9881defb413fde2c8331da1f
//! that merges its output into the shared state/province objects (`world_fold.rs::build_snapshot`),
//! and the collector read and payload writer that produce the rows
//! (`extractor/native/collector/src/attach_log.rs:59480`-`:59538`,
//! `:59540`-`:59665` and `:71678`-`:71727`).
//! The fold and collector read are the field-level basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_map_control`, writer at
//! `attach_log.rs:71687`-`:71699`): a 32-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `state_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `province_count` u32 at `+20`,
//! `changed_state_rows` u32 at `+24`, `changed_province_rows` u32 at `+28` —
//! followed by `state_count` fixed 16-byte state rows and then `province_count`
//! fixed 16-byte province rows. The fold requires
//! `payload.len() == 32 + state_count*16 + province_count*16` exactly (`:22234`),
//! so the length selects nothing and there is no legacy row form.
//!
//! **Two collections in one domain**, keyed independently:
//!
//! | offset | width | state row field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `state_id`   | raw | none (key) |
//! | `+4`  | u32 | `owner`      | raw | `flags & 1`      (`:22270`) |
//! | `+8`  | u32 | `controller` | raw | `flags & 1<<1`   (`:22278`) |
//! | `+12` | u32 | `flags`      | —   | — |
//!
//! | offset | width | province row field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `province_id` | raw | none (key) |
//! | `+4`  | u32 | `controller`  | raw | `flags & 1`     (`:22297`) |
//! | `+8`  | u32 | `state_id`    | raw | `flags & 1<<1`  (`:22305`) |
//! | `+12` | u32 | `flags`       | —   | — |
//!
//! **No scale factor anywhere.** Every one of the eight words is an integer
//! identity emitted raw: the fold stringifies the two country ids
//! (`owner.to_string()`, `:22271`) and passes `state_id` through `json!` unchanged
//! (`:22306`). Nothing here is fixed-point, so the 32768-vs-100000 class of error
//! has no surface in this domain.
//!
//! **Every gated field is `null`, never `unknown` and never `absent`, when its
//! gate is clear.** The fold emits JSON `Null` uniformly on a clear gate, which
//! decides value-vs-nothing and nothing else; the
//! collector decides the rest, and here it is decisive in a way d63's was not.
//! The payload writer sets each bit as a pure non-zero test of the value it gates
//! (`attach_log.rs:71702`-`:71720`):
//!
//! ```text
//! if row.owner_tag_db_id      != 0 { flags |= 1 }      // state row
//! if row.controller_tag_db_id != 0 { flags |= 1 << 1 } // state row
//! if row.controller_tag_db_id != 0 { flags |= 1 }      // province row
//! if row.state_id             != 0 { flags |= 1 << 1 } // province row
//! ```
//!
//! So a clear gate carries exactly one fact: **the value was read successfully and
//! it is zero** — tag_db_id `0` is "no country", and a province `state_id` of `0`
//! is a province whose `CProvince::state` pointer is null
//! (`attach_log.rs:59588`), i.e. sea, lake or wasteland. That is record-state contract's **null**,
//! "applicable, the relation genuinely points at nothing", word for word.
//!
//! It is specifically **not unknown**: an unreadable value never reaches a row at
//! all. Every per-field read on this path is `?`-propagated out of
//! `sample_compact_map_control_from_topology` (`attach_log.rs:59490`, `:59496`,
//! `:59522`) and out of `build_compact_map_control_topology` (`:59587`, `:59613`),
//! so a single unreadable word fails the whole sample; `read_compact_map_control`
//! then returns zero rows with `COMPACT_STATUS_SOURCE_UNAVAILABLE`
//! (`:58552`-`:58566`), which is a domain-hour failure status the fold handles at
//! `world_fold.rs::fold_map_control` — not a row with a clear flag. Uncaptured is a
//! whole-hour state in this domain, never a per-field one.
//!
//! It is also not **absent**: `world_fold.rs:5350` and `:5403` insert these keys
//! into every state and province object the domain carries, so the concept
//! applies to every row. A sea province genuinely has no controller; it is not a
//! row for which "controller" is unsupported.
//!
//! Real capture corroboration (`runs/batch1/deepwar_t1/collector_stream.bin`, 903
//! d22 DATA records): every state row in the capture carries `flags == 3`, and
//! province rows carry only `flags == 3` (10,412) or `flags == 0` (3,141) — the
//! sea/lake/wasteland provinces, both fields null together. No row ever carries a
//! partial flag word or an undefined bit.
//!
//! **Zero is still a value.** `flags` bit set with a zero word is accepted and
//! maps to `ContentId(0)`, exactly as the fold does (`json!("0")`): see
//! "Mirroring the fold" below.
//!
//! ## Shipping BASELINE/DELTA vs wire BASELINE/DELTA — two different layers
//!
//! d22 is the first fan-out domain whose *shipping* payload is itself incremental,
//! so the two layers have to be kept apart.
//!
//! *Shipping layer* (`world_fold.rs::fold_map_control`): a record
//! with `COMPACT_STATUS_DELTA_DATA` clear is a BASELINE — the fold **clears both
//! maps** and then inserts every row in the payload. With the bit set it is a
//! DELTA — the fold upserts only the rows present and touches nothing else. There
//! is no delete row in either form, so **within a run of shipping deltas a key
//! never leaves**. A DELTA before any BASELINE is not an error: the fold marks the
//! domain-hour unknown, folds nothing, and retains the accumulator (`world_fold.rs::fold_map_control`).
//!
//! *Wire layer*: [`Accumulator`] replays the shipping layer into the two full
//! per-hour keyed states, and only then does the wire grammar apply — a wire
//! BASELINE at each block start and a stored-minimal wire DELTA (delta-minimality contract) between,
//! computed by `body::diff` from the previous hour's full state. The two layers do
//! not line up one for one:
//!
//! - a shipping DELTA carrying no rows is a wire **UNCHANGED** hour, not a wire
//!   DELTA;
//! - a shipping BASELINE that re-states the same rows is also a wire UNCHANGED
//!   hour;
//! - a wire **tombstone** is emitted exactly when a key present at hour `h-1` is
//!   absent at hour `h`, which under the shipping grammar can only happen when a
//!   shipping BASELINE lands that omits a previously seen key. The shipping format
//!   has no way to say that inside a delta run; the wire format does, and that
//!   difference is carried by [`Accumulator::fold_payload`]'s clear-on-baseline,
//!   not by any per-row flag.
//!
//! ## Mirroring the fold
//!
//! `fold_map_control` is markedly more permissive than `fold_country_core_scalars`:
//! it does **not** reject a zero `state_id`, does **not** reject undefined
//! `flags` bits, does **not** require a gated-off word to be zero, and does **not**
//! reject a key repeated inside one frame (`.entry(..).or_default()` at `:22264`
//! and `:22291`, with both fields re-inserted, makes a repeat a whole-row
//! last-writer-wins). The recipe's precedence rule is that the fold is the
//! authority on the payload grammar, so this module mirrors every one of those
//! choices rather than tightening them, and each is covered by a test that pins
//! the mirrored behaviour. Mirroring is information-preserving in both directions:
//! a set gate with a zero word keeps the zero, and a clear gate with a non-zero
//! word drops it — which is precisely what the fold stores. The collector cannot
//! actually produce either case (the writer's flag is a pure non-zero test), so
//! the divergence between the writer's biconditional and the fold's one-way read
//! is recorded as an open question rather than enforced here.
//!
//! **No derived field (derived-field contract).** Nothing in d22 changes deterministically with the
//! clock: ownership and control move on player and AI action. The mappers still
//! take `frame_tick` so every domain's boundary has one shape.
//!
//! ## The snapshot's three TAG mirrors, and what MAPC1 certifies (GP-U15)
//!
//! Besides the per-state and per-province objects, d22 is the sole producer of
//! `map.owner_by_state`, `map.control_by_state` and `map.control_by_province` —
//! the flat id -> TAG tables replay's raster paths read. All three are derived
//! by ONE function
//! (`recorder_host/src/world_fold/wave2_sidecars/xmap05_control_authority.rs::control_census`),
//! which also mints `map._r2_xmap05_control_authority`, the MAPC1 envelope XMAP05
//! requires (`map_viewer/src/production_features/xmap05.rs::admitted_epoch_keying`).
//!
//! A mirror row exists **iff** the cell holds a country id this hour's D18 map
//! resolved to a TAG. The two rows that never reach a mirror are the two this
//! module documents above:
//!
//! * a CLEARED GATE is a measurement — "read successfully, and it is zero". For a
//!   province that means sea, lake or wasteland (`state_id` null too, 3,141 of
//!   13,413 rows in `runs/batch1/deepwar_t1`); those rows have no controller to
//!   mirror. Replay's own rule for these tables is that a MISSING id is genuinely
//!   uncontrolled while an explicit null blocks the whole projection
//!   (`replay_app/src/map_labels.rs::measured_control_by_province`), so a null row
//!   in a mirror is not neutral — it is a refusal.
//! * an UNRESOLVED `tag_db_id` keeps its numeric spelling in the snapshot's
//!   state/province objects (`world_fold.rs` canonical primitive contract.2, never fabricate a TAG). It must
//!   not reach a mirror, because replay's TAG charset is `[A-Z0-9_]` and would
//!   accept `"286"` as a country.
//!
//! Neither exclusion loses anything: the state and province objects still carry
//! every d22 row with its explicit nulls and numeric ids, and the unresolved ids
//! are named in `provenance.notes`.
//!
//! The MAPC1 envelope is the statement that neither exclusion was needed —
//! every state has a resolved owner and controller, every in-state province has a
//! resolved controller — plus the counts and the five digests replay re-derives.
//! When that does not hold the envelope is OMITTED (not nulled: the consumer
//! collapses null and absence to one error) and the hour names the cause once.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 22;
/// Collection ids. Both are `Shape::KeyedSet`; they share a payload, not a key
/// space — a `state_id` and a `province_id` of the same number are different rows.
pub const STATES: u32 = 1;
pub const PROVINCES: u32 = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
/// Ids are unique across both collections even though the format only requires
/// uniqueness within one, so a field id names one field in this domain.
pub mod field {
    pub const STATE_ID: u32 = 1;
    pub const STATE_OWNER: u32 = 2;
    pub const STATE_CONTROLLER: u32 = 3;
    pub const PROVINCE_ID: u32 = 4;
    pub const PROVINCE_CONTROLLER: u32 = 5;
    pub const PROVINCE_STATE_ID: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs:386`-`:387` and `:22230`-`:22311`. ---

/// `world_fold.rs::fold_map_control` — `need(payload, 32, ...)`, then rows at `32 + ..`.
pub const HEADER_LEN: usize = 32;
/// `MAP_STATE_ROW_LEN`, `world_fold.rs::MAP_STATE_ROW_LEN`. Fixed; there is no legacy form.
pub const STATE_ROW_LEN: usize = 16;
/// `MAP_PROVINCE_ROW_LEN`, `world_fold.rs::MAP_PROVINCE_ROW_LEN`.
pub const PROVINCE_ROW_LEN: usize = 16;

/// State row: gates `owner` (`world_fold.rs::fold_map_control`).
pub const FLAG_STATE_OWNER: u32 = 1;
/// State row: gates `controller` (`world_fold.rs::fold_map_control`).
pub const FLAG_STATE_CONTROLLER: u32 = 1 << 1;
/// Province row: gates `controller` (`world_fold.rs::fold_map_control`).
pub const FLAG_PROVINCE_CONTROLLER: u32 = 1;
/// Province row: gates `state_id` (`world_fold.rs::fold_map_control`).
pub const FLAG_PROVINCE_STATE_ID: u32 = 1 << 1;

pub fn schema() -> Schema {
    use field as f;
    // Both collections are `Shape::KeyedSet`: rows have an identity that persists
    // across hours (`state_id` / `province_id` are the fold's own map keys,
    // `world_fold.rs:22266` and `:22293`), and only a keyed set has a delta
    // grammar (collection-shape contract) — which this domain needs, since its shipping form is
    // incremental and most hours change a handful of rows out of ~14,500.
    //
    // Kinds: every one of the six fields is `ContentId`. content-identity contract's deciding question is
    // whether a content manifest must resolve the id for it to keep its meaning
    // under another mod set.
    //   * `state_id`, `province_id`  — states and provinces are defined by map and
    //     history content; province 1234 is whatever province the mod set defines.
    //     d20 classes `location_province` and `move_target_province` the same way.
    //   * `owner`, `controller`      — a country `tag_db_id` indexes the country
    //     database the game builds from content at load, so the same number names
    //     a different country under another mod set. d20's `owner` is the *same*
    //     quantity from `fold_divisions` (`world_fold.rs::fold_divisions`, also stringified)
    //     and is `ContentId`; d63's `tag_db_id` key is `ContentId` too. Classifying
    //     it `Unsigned` here would split one concept across sibling domains and
    //     hide these fields from the manifest-assignment retrofit sweep.
    // No field is `Monotone` (nothing counts), `Enum` (no bounded code),
    // `Boolean`, or `Continuous` (no measured quantity, hence no quantization).
    let states = Collection {
        collection_id: STATES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            // Null when the gate is clear: tag_db_id 0 is "no country owns this
            // state" — the relation points at nothing. Not optional (the concept
            // applies to every state row, `world_fold.rs::build_snapshot`) and not
            // unknown-capable (an unreadable read fails the whole hour, never one
            // field — see the module docs).
            Field::new(f::STATE_OWNER, Kind::ContentId).nullable(),
            Field::new(f::STATE_CONTROLLER, Kind::ContentId).nullable(),
        ],
    };
    let provinces = Collection {
        collection_id: PROVINCES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::PROVINCE_ID, Kind::ContentId).key(0),
            Field::new(f::PROVINCE_CONTROLLER, Kind::ContentId).nullable(),
            // Null for a province with no state: sea, lake or wasteland, whose
            // `CProvince::state` pointer is null (`attach_log.rs:59588`).
            Field::new(f::PROVINCE_STATE_ID, Kind::ContentId).nullable(),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: at most one DATA record per game
        // hour (903 records over 903 consecutive hours in the reference capture).
        cadence_log2: 0,
        collections: vec![states, provinces],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 32-byte payload header (`world_fold.rs::fold_map_control`, written at
/// `attach_log.rs:71692`-`:71699`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub state_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub province_count: u32,
    pub changed_state_rows: u32,
    pub changed_province_rows: u32,
}

/// Split a d22 DATA payload into its header, its state rows and its province
/// rows, applying the fold's own structural checks (`world_fold.rs::fold_map_control`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// The fold's checks are the only ones applied. In particular there is **no**
/// full-replace `changed_rows == row_count` invariant to enforce — d22 is
/// incremental, and on a shipping delta the header's counts describe only the
/// rows carried (`attach_log.rs:58620`-`:58624`). The header's two per-collection
/// changed counts are surfaced for reporting and deliberately not cross-checked
/// against `changed_rows`: the writer sums them saturatingly
/// (`attach_log.rs:58617`), so equality is not a grammar rule the recorder holds.
///
/// This is called on **every** d22 DATA record, including failure-status ones:
/// the fold checks the length before it checks the status (`:22230` before
/// `:22238`), so a malformed failure record is still a malformed record.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "map_control payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        state_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        province_count: u32_at(payload, 20),
        changed_state_rows: u32_at(payload, 24),
        changed_province_rows: u32_at(payload, 28),
    };
    // Checked arithmetic throughout: both counts are attacker-shaped input from a
    // stream on disk, and `count * ROW_LEN` overflows `usize` for a large enough
    // word. The fold multiplies unchecked (`:22234`), which is a debug panic and a
    // wrapped length in release; a boundary that reads files must not do either.
    let states_len = (header.state_count as usize)
        .checked_mul(STATE_ROW_LEN)
        .ok_or_else(|| format!("map_control state_count {} overflows", header.state_count))?;
    let provinces_len = (header.province_count as usize)
        .checked_mul(PROVINCE_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "map_control province_count {} overflows",
                header.province_count
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(states_len)
        .and_then(|len| len.checked_add(provinces_len))
        .ok_or_else(|| "map_control payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("map_control len {} != {expected}", payload.len()));
    }
    let states_end = HEADER_LEN + states_len;
    Ok((
        header,
        &payload[HEADER_LEN..states_end],
        &payload[states_end..],
    ))
}

/// The key of one state row: its `state_id` (`world_fold.rs::fold_map_control`).
///
/// Zero is a legal key here. `fold_map_control` applies no zero check — unlike
/// `fold_country_core_scalars` (`world_fold.rs::fold_country_core_scalars`) — so a zero `state_id` becomes the map
/// entry `"0"` rather than a rejected frame, and this mirrors that.
pub fn key_from_state_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_row_len(row, STATE_ROW_LEN, "state")?;
    Ok(key(&[u32_at(row, 0) as u64]))
}

/// The key of one province row: its `province_id` (`world_fold.rs::fold_map_control`).
pub fn key_from_province_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_row_len(row, PROVINCE_ROW_LEN, "province")?;
    Ok(key(&[u32_at(row, 0) as u64]))
}

/// One compact row of the `states` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 16-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); d22 has exactly one state
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d22 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Nothing in this function can panic on any 16-byte input.
pub fn record_from_state_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d22 has no derived field; derived-field contract's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row, STATE_ROW_LEN, "state")?;
    let flags = u32_at(row, 12);

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `world_fold.rs::fold_map_control`.
            field::STATE_OWNER => gated_id(flags & FLAG_STATE_OWNER != 0, u32_at(row, 4)),
            // `world_fold.rs::fold_map_control`.
            field::STATE_CONTROLLER => {
                gated_id(flags & FLAG_STATE_CONTROLLER != 0, u32_at(row, 8))
            }
            other => {
                // A collection this mapper does not know how to fill — another
                // domain's, or this domain's *province* collection. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!("map_control has no state mapping for field id {other}"));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact row of the `provinces` table, onto its stored Record. See
/// [`record_from_state_row`] for the contract; `row` is the raw 16-byte province
/// row slice.
pub fn record_from_province_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_row_len(row, PROVINCE_ROW_LEN, "province")?;
    let flags = u32_at(row, 12);

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `world_fold.rs::fold_map_control`.
            field::PROVINCE_CONTROLLER => {
                gated_id(flags & FLAG_PROVINCE_CONTROLLER != 0, u32_at(row, 4))
            }
            // `world_fold.rs::fold_map_control`.
            field::PROVINCE_STATE_ID => {
                gated_id(flags & FLAG_PROVINCE_STATE_ID != 0, u32_at(row, 8))
            }
            other => {
                return Err(format!(
                    "map_control has no province mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// A clear gate means the value was read and it is zero — no country, or no state
/// — not that it could not be captured. record-state contract calls that **null**. See the module
/// docs for the collector evidence.
fn gated_id(present: bool, value: u32) -> Value {
    if present {
        Value::ContentId(value as u64)
    } else {
        Value::Null
    }
}

// --- The shipping incremental layer -----------------------------------------

/// What one d22 DATA record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// A shipping BASELINE: both maps were cleared and refilled
    /// (`world_fold.rs::fold_map_control`). The domain now has a baseline.
    Baseline,
    /// A shipping DELTA applied on top of an existing baseline: the payload's rows
    /// were upserted and nothing else was touched.
    Delta,
    /// A shipping DELTA that arrived before any BASELINE. `world_fold.rs:22247`-
    /// `:22252` marks the domain-hour unknown, folds **nothing**, and retains the
    /// accumulator. The hour carries no d22 data and belongs in no measurement.
    DeltaBeforeBaseline,
}

/// The running d22 state, replaying `fold_map_control`'s incremental grammar.
///
/// This is the piece that makes d22 different from a full-replace domain, so it
/// lives beside the schema rather than in a harness: the measurement bin, the
/// tests and any future caller share one implementation of clear-on-baseline,
/// upsert-on-delta and delta-before-baseline.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `states` collection's current content, keyed by `state_id`.
    pub states: KeyedState,
    /// The `provinces` collection's current content, keyed by `province_id`.
    pub provinces: KeyedState,
    baseline_seen: bool,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `world_fold.rs::fold_map_control` — whether a BASELINE has been folded, which is what
    /// decides whether an arriving DELTA is data or a marked-unknown hour.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d22 DATA payload, exactly as `fold_map_control` does from
    /// `:22246` onward.
    ///
    /// `is_delta` is the caller's reading of `COMPACT_STATUS_DELTA_DATA` in
    /// `header.status_flags`. It is a parameter rather than a bit tested here
    /// because that constant belongs to `compact_contract` and d22 is not
    /// admitted to the production-codec dependency surface. Copying the literal
    /// into this module is exactly the drift the recipe forbids.
    ///
    /// The caller must already have excluded failure-status records: the fold
    /// returns at `:22238` before reaching any of this, retaining the accumulator.
    ///
    /// On `Err` the accumulator is left as it was, so a caller that reports the
    /// error and stops is not left holding a half-folded frame. Errors are only
    /// structural (a row slice of the wrong length, a field id this domain does
    /// not map); no row *content* can fail, because the fold rejects no content.
    pub fn fold_payload(
        &mut self,
        states_collection: &Collection,
        provinces_collection: &Collection,
        payload: &[u8],
        is_delta: bool,
    ) -> core::result::Result<FoldOutcome, String> {
        let (header, state_rows, province_rows) = split_payload(payload)?;
        let _ = header;
        // `world_fold.rs::fold_map_control`. Note the order: this is decided before
        // anything is cleared or upserted, so the accumulator survives untouched.
        if is_delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        // Build both row sets before mutating anything, so a structural error
        // leaves the accumulator exactly as the fold would have left it — the
        // fold's own errors all precede its first mutation (`:22230`-`:22252`).
        let mut new_states: Vec<(Key, Record)> = Vec::with_capacity(state_rows.len() / STATE_ROW_LEN);
        for row in state_rows.chunks_exact(STATE_ROW_LEN) {
            new_states.push((
                key_from_state_row(row)?,
                record_from_state_row(states_collection, row, 0)?,
            ));
        }
        let mut new_provinces: Vec<(Key, Record)> =
            Vec::with_capacity(province_rows.len() / PROVINCE_ROW_LEN);
        for row in province_rows.chunks_exact(PROVINCE_ROW_LEN) {
            new_provinces.push((
                key_from_province_row(row)?,
                record_from_province_row(provinces_collection, row, 0)?,
            ));
        }

        // `world_fold.rs::fold_map_control`: a baseline clears BOTH maps first.
        if !is_delta {
            self.states.clear();
            self.provinces.clear();
        }
        // `.entry(..).or_default()` then re-inserting every field (`:22264`-`:22283`,
        // `:22291`-`:22310`) is a whole-row upsert, and a key repeated inside one
        // frame is last-writer-wins rather than a rejected frame.
        for (k, record) in new_states {
            self.states.insert(k, record);
        }
        for (k, record) in new_provinces {
            self.provinces.insert(k, record);
        }
        if !is_delta {
            // `world_fold.rs::fold_map_control`.
            self.baseline_seen = true;
            Ok(FoldOutcome::Baseline)
        } else {
            Ok(FoldOutcome::Delta)
        }
    }
}

fn check_row_len(row: &[u8], expected: usize, what: &str) -> core::result::Result<(), String> {
    if row.len() != expected {
        return Err(format!(
            "map_control {what} row length {} != {expected}",
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

    const DELTA_BIT: bool = true;
    const BASELINE_BIT: bool = false;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn states() -> Collection {
        schema().collections[0].clone()
    }

    fn provinces() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 16-byte state row, packed the way the collector packs one
    /// (`attach_log.rs:71700`-`:71712`).
    fn state_row(state_id: u32, owner: u32, controller: u32, flags: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(STATE_ROW_LEN);
        for v in [state_id, owner, controller, flags] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), STATE_ROW_LEN);
        row
    }

    /// A raw 16-byte province row (`attach_log.rs:71713`-`:71725`).
    fn province_row(province_id: u32, controller: u32, state_id: u32, flags: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(PROVINCE_ROW_LEN);
        for v in [province_id, controller, state_id, flags] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), PROVINCE_ROW_LEN);
        row
    }

    /// A whole d22 DATA payload: the 32-byte header then the two row runs.
    fn payload(status_flags: u32, state_rows: &[Vec<u8>], province_rows: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(state_rows.len() as u32).to_le_bytes());
        out.extend_from_slice(
            &((state_rows.len() + province_rows.len()) as u32).to_le_bytes(), // changed_rows
        );
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&(province_rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(state_rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(province_rows.len() as u32).to_le_bytes());
        assert_eq!(out.len(), HEADER_LEN);
        for row in state_rows {
            out.extend_from_slice(row);
        }
        for row in province_rows {
            out.extend_from_slice(row);
        }
        out
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d22 schema is well-formed");
    }

    #[test]
    fn it_is_two_keyed_sets_keyed_by_state_and_province() {
        let s = states();
        assert_eq!(s.collection_id, STATES);
        assert_eq!(s.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = s.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys state rows by state_id alone");
        assert_eq!(keys[0].field_id, field::STATE_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a state is defined by map/history content (content-identity contract)"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(s.fields.len(), 3, "1 key + owner + controller");

        let p = provinces();
        assert_eq!(p.collection_id, PROVINCES);
        assert_eq!(p.shape, Shape::KeyedSet);
        let keys = p.key_fields();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].field_id, field::PROVINCE_ID);
        assert_eq!(keys[0].kind, Kind::ContentId);
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(p.fields.len(), 3, "1 key + controller + state_id");

        assert_eq!(schema().collections.len(), 2, "one payload, two collections");
    }

    #[test]
    fn every_gated_field_is_nullable_and_nothing_else() {
        // The schema declares exactly one capability, so the mapper may emit
        // exactly one empty state. Absent would claim the concept does not apply
        // to a state or province — but `world_fold.rs::build_snapshot` insert these
        // keys on every row. Unknown would claim the value was not captured — but
        // an unreadable read fails the whole hour with
        // COMPACT_STATUS_SOURCE_UNAVAILABLE (`attach_log.rs:58564`) and never
        // reaches a row. What is left is record-state contract's null: the value was read and it is
        // zero, meaning no country / no state.
        for c in [states(), provinces()] {
            assert_eq!(
                c.optional_count(),
                0,
                "collection {}: no field is ever inapplicable",
                c.collection_id
            );
            assert!(
                c.value_fields().all(|f| !f.unknown_capable),
                "collection {}: uncaptured is an hour-level state here, not a field-level one",
                c.collection_id
            );
            let nullable: Vec<u32> = c
                .value_fields()
                .filter(|f| f.nullable)
                .map(|f| f.field_id)
                .collect();
            assert_eq!(
                nullable,
                c.value_fields().map(|f| f.field_id).collect::<Vec<_>>(),
                "collection {}: every value field is gated",
                c.collection_id
            );
            assert_eq!(nullable.len(), 2);
        }
    }

    #[test]
    fn no_field_is_continuous_so_no_quantization_applies() {
        // Step 4's "quantization saves bytes" test does not apply: d22 has no
        // continuous field at all. Every one of the eight payload words is an
        // integer identity emitted raw by the fold (`world_fold.rs:22271`,
        // `:22306`) — no scale, no measured quantity, nothing a viewer draws or
        // interpolates. Step 8's scale test does not apply for the same reason,
        // and `every_field_is_emitted_raw` below pins the identity mapping that
        // takes its place.
        for c in [states(), provinces()] {
            for f in c.fields.iter() {
                assert!(!matches!(f.kind, Kind::Continuous), "field {}", f.field_id);
                assert_eq!(f.quant, Quant::None, "field {}", f.field_id);
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; ownership and control move
        // on player and AI action, so `frame_tick` never enters a value.
        for c in [states(), provinces()] {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    #[test]
    fn a_full_state_row_roundtrips_byte_for_byte() {
        let c = states();
        let record = record_from_state_row(&c, &state_row(42, 7, 9, 3), 0).expect("maps");
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
    fn a_full_province_row_roundtrips_byte_for_byte() {
        let c = provinces();
        let record = record_from_province_row(&c, &province_row(1234, 7, 42, 3), 0).expect("maps");
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
    fn a_fully_gated_off_row_roundtrips_byte_for_byte() {
        // `flags == 0` is 3,141 of the 13,413 province rows in the reference
        // capture — the sea, lake and wasteland provinces.
        for (c, row) in [
            (states(), state_row(42, 0, 0, 0)),
            (provinces(), province_row(1234, 0, 0, 0)),
        ] {
            let record = if c.collection_id == STATES {
                record_from_state_row(&c, &row, 0)
            } else {
                record_from_province_row(&c, &row, 0)
            }
            .expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len());
            assert_eq!(back, record);
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes);
        }
    }

    #[test]
    fn the_mapper_emits_values_for_full_rows() {
        let s = states();
        let r = record_from_state_row(&s, &state_row(42, 7, 9, 3), 0).expect("maps");
        assert_eq!(r.values[index_of(&s, field::STATE_OWNER)], Value::ContentId(7));
        assert_eq!(
            r.values[index_of(&s, field::STATE_CONTROLLER)],
            Value::ContentId(9)
        );

        let p = provinces();
        let r = record_from_province_row(&p, &province_row(1234, 7, 42, 3), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&p, field::PROVINCE_CONTROLLER)],
            Value::ContentId(7)
        );
        assert_eq!(
            r.values[index_of(&p, field::PROVINCE_STATE_ID)],
            Value::ContentId(42)
        );
    }

    #[test]
    fn every_field_is_emitted_raw() {
        // The domain has no scale factor, so step 8's scale test becomes the
        // claim it would otherwise verify: the mapped value is the row's word,
        // bit for bit, across the whole u32 range including the extremes. A stray
        // division or a transposed offset fails here.
        let s = states();
        let p = provinces();
        for v in [1u32, 32_768, 100_000, 1_000_000_000, u32::MAX] {
            let r = record_from_state_row(&s, &state_row(1, v, v.wrapping_sub(1), 3), 0)
                .expect("maps");
            assert_eq!(
                r.values[index_of(&s, field::STATE_OWNER)],
                Value::ContentId(v as u64),
                "owner {v} must map to itself"
            );
            assert_eq!(
                r.values[index_of(&s, field::STATE_CONTROLLER)],
                Value::ContentId(v.wrapping_sub(1) as u64),
                "controller is read at +8, not +4"
            );
            let r = record_from_province_row(&p, &province_row(1, v, v.wrapping_sub(1), 3), 0)
                .expect("maps");
            assert_eq!(
                r.values[index_of(&p, field::PROVINCE_CONTROLLER)],
                Value::ContentId(v as u64)
            );
            assert_eq!(
                r.values[index_of(&p, field::PROVINCE_STATE_ID)],
                Value::ContentId(v.wrapping_sub(1) as u64),
                "province state_id is read at +8, not +4"
            );
        }
        // And the keys, at +0 in both row forms.
        assert_eq!(
            key_from_state_row(&state_row(u32::MAX, 0, 0, 0)).expect("keys"),
            key(&[u32::MAX as u64])
        );
        assert_eq!(
            key_from_province_row(&province_row(13_413, 0, 0, 0)).expect("keys"),
            key(&[13_413])
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // A set gate with a zero word: the fold stores `json!("0")` rather than
        // Null (`world_fold.rs::fold_map_control` tests the flag, never the value),
        // so this mapper keeps the zero. record-state contract keeps zero out of the three empty
        // states. The collector's writer cannot produce this row — its flag is a
        // pure non-zero test (`attach_log.rs:71702`) — which is the open question
        // recorded in the deliverable, not a licence to diverge from the fold.
        let s = states();
        let r = record_from_state_row(&s, &state_row(42, 0, 0, 3), 0).expect("maps");
        assert_eq!(r.values[index_of(&s, field::STATE_OWNER)], Value::ContentId(0));
        assert_eq!(
            r.values[index_of(&s, field::STATE_CONTROLLER)],
            Value::ContentId(0)
        );
        let p = provinces();
        let r = record_from_province_row(&p, &province_row(1, 0, 0, 3), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&p, field::PROVINCE_CONTROLLER)],
            Value::ContentId(0)
        );
        assert_eq!(
            r.values[index_of(&p, field::PROVINCE_STATE_ID)],
            Value::ContentId(0)
        );
    }

    #[test]
    fn a_clear_gate_is_null_field_by_field() {
        // The whole gate space for both row forms, one bit at a time — not one
        // sampled case. A gated-off word carries a non-zero value here to pin the
        // other half of the mirror: the fold reads the flag and never the word
        // (`:22270`, `:22278`, `:22297`, `:22305`), so the value is dropped.
        let s = states();
        for flags in 0..4u32 {
            let r = record_from_state_row(&s, &state_row(42, 7, 9, flags), 0).expect("maps");
            let owner = r.values[index_of(&s, field::STATE_OWNER)];
            let controller = r.values[index_of(&s, field::STATE_CONTROLLER)];
            assert_eq!(
                owner,
                if flags & FLAG_STATE_OWNER != 0 {
                    Value::ContentId(7)
                } else {
                    Value::Null
                },
                "state owner under flags {flags}"
            );
            assert_eq!(
                controller,
                if flags & FLAG_STATE_CONTROLLER != 0 {
                    Value::ContentId(9)
                } else {
                    Value::Null
                },
                "state controller under flags {flags}"
            );
        }
        let p = provinces();
        for flags in 0..4u32 {
            let r = record_from_province_row(&p, &province_row(1234, 7, 42, flags), 0)
                .expect("maps");
            assert_eq!(
                r.values[index_of(&p, field::PROVINCE_CONTROLLER)],
                if flags & FLAG_PROVINCE_CONTROLLER != 0 {
                    Value::ContentId(7)
                } else {
                    Value::Null
                },
                "province controller under flags {flags}"
            );
            assert_eq!(
                r.values[index_of(&p, field::PROVINCE_STATE_ID)],
                if flags & FLAG_PROVINCE_STATE_ID != 0 {
                    Value::ContentId(42)
                } else {
                    Value::Null
                },
                "province state_id under flags {flags}"
            );
        }
    }

    #[test]
    fn undefined_flag_bits_are_ignored_exactly_as_the_fold_ignores_them() {
        // `fold_map_control` tests bit 0 and bit 1 and nothing else — unlike
        // `fold_country_core_scalars`, it has no KNOWN_FLAGS check (`:9283`) to
        // mirror. Tightening this here would make the mapper refuse a row the
        // recorder accepts, which the recipe's precedence rule forbids.
        let s = states();
        let p = provinces();
        for bit in 2..32u32 {
            let flags = 3 | (1 << bit);
            let r = record_from_state_row(&s, &state_row(42, 7, 9, flags), 0)
                .unwrap_or_else(|e| panic!("state row with bit {bit} set: {e}"));
            assert_eq!(r.values[index_of(&s, field::STATE_OWNER)], Value::ContentId(7));
            let r = record_from_province_row(&p, &province_row(1, 7, 42, flags), 0)
                .unwrap_or_else(|e| panic!("province row with bit {bit} set: {e}"));
            assert_eq!(
                r.values[index_of(&p, field::PROVINCE_STATE_ID)],
                Value::ContentId(42)
            );
        }
    }

    #[test]
    fn a_zero_id_is_a_key_not_a_rejected_row() {
        // `fold_map_control` applies no zero check to either id, so a zero id is
        // the map entry "0". `fold_country_core_scalars` does (`:9279`) — the
        // difference is the fold's, not a lapse here.
        assert_eq!(key_from_state_row(&state_row(0, 1, 1, 3)).expect("keys"), key(&[0]));
        assert_eq!(
            key_from_province_row(&province_row(0, 1, 1, 3)).expect("keys"),
            key(&[0])
        );
        assert!(record_from_state_row(&states(), &state_row(0, 1, 1, 3), 0).is_ok());
    }

    #[test]
    fn the_mappers_reject_every_row_length_but_their_own() {
        let s = states();
        let p = provinces();
        for len in [0usize, 1, STATE_ROW_LEN - 1, STATE_ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_state_row(&s, &row, 0).is_err(),
                "state length {len} must be rejected, not read"
            );
            assert!(record_from_province_row(&p, &row, 0).is_err(), "province length {len}");
            assert!(key_from_state_row(&row).is_err(), "state key length {len}");
            assert!(key_from_province_row(&row).is_err(), "province key length {len}");
        }
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        // Including each other's: the two collections share a payload but not a
        // field-id space, so crossing them must be an error rather than a record
        // of the wrong arity.
        let mut s = states();
        s.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_state_row(&s, &state_row(1, 1, 1, 3), 0).is_err());
        assert!(record_from_state_row(&provinces(), &state_row(1, 1, 1, 3), 0).is_err());
        assert!(record_from_province_row(&states(), &province_row(1, 1, 1, 3), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let good = payload(0, &[state_row(1, 2, 3, 3)], &[province_row(4, 5, 1, 3)]);
        let (header, state_rows, province_rows) = split_payload(&good).expect("well-formed");
        assert_eq!(header.state_count, 1);
        assert_eq!(header.province_count, 1);
        assert_eq!(header.changed_state_rows, 1);
        assert_eq!(header.changed_province_rows, 1);
        assert_eq!(state_rows, &state_row(1, 2, 3, 3)[..]);
        assert_eq!(province_rows, &province_row(4, 5, 1, 3)[..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // Any length that does not match the two counts, in both directions.
        let mut short = good.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());

        // A header-only payload with both counts zero is well-formed: the
        // reference capture contains such records (a shipping delta whose hour
        // changed nothing).
        let empty = payload(0, &[], &[]);
        assert_eq!(empty.len(), HEADER_LEN);
        let (h, s, p) = split_payload(&empty).expect("an empty delta is well-formed");
        assert_eq!((h.state_count, h.province_count), (0, 0));
        assert!(s.is_empty() && p.is_empty());

        // Counts whose byte length overflows `usize` must be rejected, not wrapped
        // into a plausible small length. The fold multiplies unchecked (`:22234`).
        for offset in [8usize, 20] {
            let mut huge = good.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&huge).is_err(), "count at +{offset}");
        }
        // And a count that is merely far too large for the payload.
        let mut wrong = good.clone();
        wrong[8..12].copy_from_slice(&1000u32.to_le_bytes());
        assert!(split_payload(&wrong).is_err());

        // The state/province split is by count, not by guesswork: two state rows
        // and no province rows must not read the second state row as a province.
        let two_states = payload(0, &[state_row(1, 2, 3, 3), state_row(7, 8, 9, 3)], &[]);
        let (h, s, p) = split_payload(&two_states).expect("well-formed");
        assert_eq!(h.state_count, 2);
        assert_eq!(s.len(), 2 * STATE_ROW_LEN);
        assert!(p.is_empty());
    }

    // --- The shipping incremental layer -------------------------------------

    fn fold(acc: &mut Accumulator, bytes: &[u8], is_delta: bool) -> FoldOutcome {
        acc.fold_payload(&states(), &provinces(), bytes, is_delta)
            .expect("folds")
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing() {
        // `world_fold.rs::fold_map_control`: the domain-hour is marked unknown, the
        // accumulator is retained, and the rows in the payload are NOT applied.
        let mut acc = Accumulator::new();
        assert!(!acc.baseline_seen());
        let outcome = fold(
            &mut acc,
            &payload(0, &[state_row(1, 2, 3, 3)], &[province_row(4, 5, 1, 3)]),
            DELTA_BIT,
        );
        assert_eq!(outcome, FoldOutcome::DeltaBeforeBaseline);
        assert!(acc.states.is_empty(), "a pre-baseline delta applies no rows");
        assert!(acc.provinces.is_empty());
        assert!(!acc.baseline_seen());

        // And it stays that way for as many deltas as arrive.
        assert_eq!(
            fold(&mut acc, &payload(0, &[state_row(9, 9, 9, 3)], &[]), DELTA_BIT),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(acc.states.is_empty());
    }

    #[test]
    fn a_baseline_clears_both_maps_and_a_delta_upserts() {
        let mut acc = Accumulator::new();
        assert_eq!(
            fold(
                &mut acc,
                &payload(
                    0,
                    &[state_row(1, 10, 10, 3), state_row(2, 20, 20, 3)],
                    &[province_row(5, 10, 1, 3), province_row(6, 20, 2, 3)],
                ),
                BASELINE_BIT,
            ),
            FoldOutcome::Baseline
        );
        assert!(acc.baseline_seen());
        assert_eq!(acc.states.len(), 2);
        assert_eq!(acc.provinces.len(), 2);

        // A delta touches only the rows it carries.
        assert_eq!(
            fold(
                &mut acc,
                &payload(0, &[state_row(2, 30, 30, 3)], &[]),
                DELTA_BIT,
            ),
            FoldOutcome::Delta
        );
        assert_eq!(acc.states.len(), 2, "a delta never removes a key");
        assert_eq!(acc.provinces.len(), 2, "and never touches the other collection");
        let s = states();
        assert_eq!(
            acc.states[&key(&[2])].values[index_of(&s, field::STATE_OWNER)],
            Value::ContentId(30)
        );
        assert_eq!(
            acc.states[&key(&[1])].values[index_of(&s, field::STATE_OWNER)],
            Value::ContentId(10),
            "the untouched row keeps its value"
        );

        // A delta may also introduce a key that was never seen.
        fold(
            &mut acc,
            &payload(0, &[], &[province_row(7, 40, 2, 3)]),
            DELTA_BIT,
        );
        assert_eq!(acc.provinces.len(), 3);

        // A later BASELINE clears BOTH maps first (`:22253`-`:22256`), so keys
        // absent from it are gone — the only way a key ever leaves.
        assert_eq!(
            fold(&mut acc, &payload(0, &[state_row(1, 99, 99, 3)], &[]), BASELINE_BIT),
            FoldOutcome::Baseline
        );
        assert_eq!(acc.states.keys().cloned().collect::<Vec<_>>(), vec![key(&[1])]);
        assert!(
            acc.provinces.is_empty(),
            "the baseline cleared the province map too, even carrying no province rows"
        );
    }

    #[test]
    fn a_key_repeated_in_one_frame_is_last_writer_wins() {
        // `.entry(..).or_default()` re-inserting both fields (`:22264`-`:22283`)
        // makes a repeat a whole-row overwrite, not a rejected frame. d63's fold
        // rejects duplicates (`:9383`); this one does not, and the fold wins.
        let mut acc = Accumulator::new();
        fold(
            &mut acc,
            &payload(0, &[state_row(1, 10, 10, 3), state_row(1, 20, 20, 3)], &[]),
            BASELINE_BIT,
        );
        assert_eq!(acc.states.len(), 1);
        let s = states();
        assert_eq!(
            acc.states[&key(&[1])].values[index_of(&s, field::STATE_OWNER)],
            Value::ContentId(20),
            "the last row for a key wins"
        );
    }

    #[test]
    fn a_structural_error_leaves_the_accumulator_untouched() {
        let mut acc = Accumulator::new();
        fold(&mut acc, &payload(0, &[state_row(1, 10, 10, 3)], &[]), BASELINE_BIT);
        let before = acc.states.clone();
        let mut broken = payload(0, &[state_row(2, 20, 20, 3)], &[]);
        broken.pop();
        assert!(acc
            .fold_payload(&states(), &provinces(), &broken, BASELINE_BIT)
            .is_err());
        assert_eq!(acc.states, before, "a refused frame clears nothing");
    }

    #[test]
    fn the_wire_layer_carries_what_the_shipping_layer_cannot() {
        // The end-to-end property this domain exists to prove: two hours of
        // shipping records fold into two full states, and the wire BASELINE +
        // DELTA pair round-trips through the reader's own reconstructed prior —
        // including a tombstone, which the shipping grammar has no row for.
        let schema = schema();
        let (s, p) = (states(), provinces());
        let mut acc = Accumulator::new();

        fold(
            &mut acc,
            &payload(
                0,
                &[state_row(1, 10, 10, 3), state_row(2, 20, 0, 1)],
                &[province_row(5, 10, 1, 3), province_row(6, 0, 0, 0)],
            ),
            BASELINE_BIT,
        );
        let hour0 = [acc.states.clone(), acc.provinces.clone()];

        let mut wire = Vec::new();
        encode_baseline(&schema, &hour0, &mut wire).expect("baseline encodes");
        let (decoded, consumed) = decode_baseline(&schema, &wire, 0).expect("baseline decodes");
        assert_eq!(consumed, wire.len());
        assert_eq!(decoded.len(), 2, "two collections in the body");
        let mut reader = [decoded[0].clone(), decoded[1].clone()];
        assert_eq!(reader, hour0, "the reader reconstructs the fold's own state");
        // The null state survived the round trip, not collapsed into a zero.
        assert_eq!(
            reader[1][&key(&[6])].values[index_of(&p, field::PROVINCE_CONTROLLER)],
            Value::Null
        );
        assert_eq!(
            reader[0][&key(&[2])].values[index_of(&s, field::STATE_CONTROLLER)],
            Value::Null
        );

        // Hour 1: a shipping BASELINE that drops state 2 and province 6, and
        // changes province 5's controller. Under the shipping grammar the drop is
        // expressible only this way; under the wire grammar it is a tombstone.
        fold(
            &mut acc,
            &payload(
                0,
                &[state_row(1, 10, 10, 3)],
                &[province_row(5, 99, 1, 3)],
            ),
            BASELINE_BIT,
        );
        let hour1 = [acc.states.clone(), acc.provinces.clone()];

        let ds = diff(&s, &hour0[0], &hour1[0]);
        let dp = diff(&p, &hour0[1], &hour1[1]);
        assert_eq!(ds.changed.len(), 0, "state 1 did not move");
        assert_eq!(ds.tombstones, vec![key(&[2])]);
        assert_eq!(dp.changed.len(), 1, "province 5's controller moved");
        assert_eq!(dp.tombstones, vec![key(&[6])]);

        let mut delta_bytes = Vec::new();
        encode_delta(&schema, &hour0, &[Some(ds), Some(dp)], &mut delta_bytes)
            .expect("delta encodes");
        let (deltas, consumed) =
            decode_delta(&schema, &reader, &delta_bytes, 0).expect("delta decodes");
        assert_eq!(consumed, delta_bytes.len());
        for (i, d) in deltas.iter().enumerate() {
            if let Some(d) = d {
                apply(&mut reader[i], d);
            }
        }
        assert_eq!(reader, hour1, "the reader's state is the fold's state");
    }

    #[test]
    fn an_hour_that_changes_one_collection_only_encodes_one_contribution() {
        // The collection change bitmap is what makes a two-collection domain cheap
        // when only one side moves — the common case here (states move ~12 times
        // in 902 hours, provinces ~140).
        let schema = schema();
        let (s, p) = (states(), provinces());
        let mut acc = Accumulator::new();
        fold(
            &mut acc,
            &payload(0, &[state_row(1, 10, 10, 3)], &[province_row(5, 10, 1, 3)]),
            BASELINE_BIT,
        );
        let hour0 = [acc.states.clone(), acc.provinces.clone()];
        fold(&mut acc, &payload(0, &[], &[province_row(5, 11, 1, 3)]), DELTA_BIT);
        let hour1 = [acc.states.clone(), acc.provinces.clone()];

        let ds = diff(&s, &hour0[0], &hour1[0]);
        let dp = diff(&p, &hour0[1], &hour1[1]);
        assert!(ds.is_empty(), "nothing happened to the states");
        assert!(!dp.is_empty());
        let mut bytes = Vec::new();
        encode_delta(&schema, &hour0, &[None, Some(dp)], &mut bytes).expect("delta encodes");
        let (deltas, consumed) = decode_delta(&schema, &hour0, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert!(deltas[0].is_none(), "the states bit is clear");
        assert!(deltas[1].is_some());
        let mut reader = hour0.clone();
        apply(&mut reader[1], deltas[1].as_ref().unwrap());
        assert_eq!(reader, hour1);

        // And an hour where nothing at all moved has no DELTA to emit: the wire
        // grammar's UNCHANGED, which `encode_delta` refuses to fake.
        assert!(encode_delta(&schema, &hour0, &[None, None], &mut Vec::new()).is_err());
    }
}
