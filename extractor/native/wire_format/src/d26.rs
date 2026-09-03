//! d26 `division_equipment_live` — per-division equipment inventories.
//!
//! Derived from `world_fold.rs:27312`-`:27618`
//! (`fold_division_equipment`), its payload constants at `:810`-`:816`, the
// @world-fold-authority domain=26 functions=fold_division_equipment lines=27312-27618 sha256=0725f8cae2fdd737e1b5c288762be80cc2e5d6b031d776f22a36881ef44323a0 closure=e87e36f1c9507c21eab4ce05ee6077fdb57856bf0309871f4b75383e013336ed
//! accumulator's own JSON projection (`:24160`-`:24176`), and the collector read
//! that produces the row flags and the entry rows
//! (`extractor/native/collector/src/attach_log.rs:45789`-`:46005`,
//! `:58210`-`:58401`, `:69877`-`:69930`).
//! The fold and collector read are the field-level basis; where another account
//! and the fold could be read differently, the fold wins.
//!
//! **Payload grammar** (`world_fold.rs::fold_division_equipment`): a 40-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `scale_raw` u32 at `+20`, `root_rows` u32
//! at `+24`, `observed_division_rows` u32 at `+28`, `equipment_entry_count` u32
//! at `+32`, `observed_equipment_rows` u32 at `+36` — followed by `row_count`
//! fixed 24-byte **root rows** and then `equipment_entry_count` fixed 16-byte
//! **entry rows**. The fold requires the length to be exactly
//! `40 + row_count * 24 + equipment_entry_count * 16` (`world_fold.rs::fold_division_equipment`), so no
//! legacy short row exists and the length selects nothing.
//!
//! Root row (`world_fold.rs::fold_division_equipment`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id` | raw | none (key; zero rejected, `world_fold.rs::fold_division_equipment`) |
//! | `+4`  | u32 | `div_id`          | raw | none (key; zero rejected, `world_fold.rs::fold_division_equipment`) |
//! | `+8`  | u32 | `flags`           | —   | — |
//! | `+12` | u32 | `template_id`     | raw | `TEMPLATE_PRESENT` (`1<<0`) |
//! | `+16` | u32 | `entry_first`     | —   | transport, not state |
//! | `+20` | u32 | `entry_count`     | —   | transport, not state |
//!
//! Entry row (`world_fold.rs::fold_division_equipment`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0` | u32 | `variant_id` | raw     | none (key; `0` and `> i32::MAX` rejected, `world_fold.rs::fold_division_equipment`) |
//! | `+4` | u32 | `reserved`   | —       | must be zero (`world_fold.rs::fold_division_equipment`) |
//! | `+8` | i64 | `amount_raw` | /100000 | none (`0 < x <= 1e15` enforced, `world_fold.rs::fold_division_equipment`) |
//!
//! **Two collections, because `(entry_first, entry_count)` is transport.** A
//! root addresses its equipment as a half-open range into one shared entry array
//! (`world_fold.rs::fold_division_equipment`); the ranges are re-packed by the collector on every
//! frame from the emission order (`attach_log.rs:69904`-`:69920`) and carry no
//! state a reader could ask about. What is state is the *set* of
//! `(owner_tag_db_id, div_id, variant_id) -> amount` facts, which is what the
//! fold's accumulator holds (`world_fold.rs:1547`) and what its duplicate check
//! is keyed by (`world_fold.rs::fold_division_equipment`). So the ranges are flattened away here: collection
//! [`DIVISIONS`] carries one row per division root and collection [`EQUIPMENT`]
//! one row per equipment line, keyed by the root key plus the variant.
//!
//! **`amount` is divided here, `template_id` is not.** `scale_raw` is asserted
//! equal to `FIXED_SCALE` by the fold (`:17414`) and the accumulator stores the
//! raw i64 with its scale beside it (`:17608`-`:17613`) — but the consumer-facing
//! projection divides: `DivisionEquipmentEntryState::to_json` emits
//! `"amount": amount_raw / scale_raw` (`:24170`), and the replay layer reads that
//! divided `amount` as an `f64` and nothing else for this domain
//! (`replay_interface/native/replay_core/src/world.rs:472`-`:477`, `:5676` —
//! `EquipmentAmount::amount`; the `amount_raw` sibling is read only by the
//! market-stockpile domain, `:21121`). The division therefore belongs at this
//! boundary. `template_id` has no scale anywhere and is carried raw.
//!
//! **Four states.** The schema declares exactly one empty state, on exactly one
//! field:
//!
//! * `template_id` is `unknown` when `TEMPLATE_PRESENT` is clear, and nothing
//!   else. The collector's `template_id` is an `Option<u32>` and the flag is
//!   exactly `Option::is_some()` (`attach_log.rs:69908`). That option is `None`
//!   only when the template pointer is not a map-control heap pointer, its
//!   vtable is not `DIVISION_TEMPLATE_VTABLE_RVA`, its type word is not
//!   `DIVISION_TEMPLATE_TYPE`, or its id fails the range filter
//!   (`:16697`-`:16717`) — four read/validation failures, which is record-state contract's
//!   "applicable, not captured", not a relation pointing at nothing and not a
//!   concept that fails to apply. Every division in the D20 root bridge has a
//!   template, so `absent` is wrong; and the recipe's tie-break points the same
//!   way for the one reading that could be `null` (a genuinely null template
//!   pointer, which is indistinguishable from an unreadable one at this
//!   boundary). See the deliverable's open questions.
//! * `amount` reaches no empty state at all. The entry row has no gate, the
//!   collector drops any line whose `amount_raw` is `0`
//!   (`attach_log.rs:58388`), and the fold rejects `amount_raw <= 0` outright
//!   (`world_fold.rs::fold_division_equipment`). A division that holds none of a variant has **no
//!   row**, so "zero of this equipment" is an absent *key*, not a zero value —
//!   which is why `amount` declares no capability and its mapper can emit only
//!   a value.
//! * A **deleted** root (`DELETED`, `1<<1`) is a removal, not a row: the fold
//!   erases the key from the accumulator (`:17593`-`:17596`) and requires the
//!   rest of the row to be empty (`:17501`-`:17509`). It maps to a wire
//!   tombstone in both collections, so it never reaches a record and never makes
//!   `template_id` `absent`. [`record_from_division_row`] refuses a deleted row
//!   rather than inventing one.
//!
//! Corroborated against the capture the measurement bin reads
//! (`runs/batch1/deepwar_t1`, 903 d26 hours; census in
//! `runs/batch1/d26_division_equipment/measurement.txt`). Of 63,166 serialized
//! root rows, 87 are `DELETED` and necessarily have the `TEMPLATE_PRESENT` gate
//! clear — the fold requires a delete row to carry nothing else
//! (`world_fold.rs:17501`), and those rows become tombstones rather than
//! records. Across the other 63,079 rows the gate is **never** clear, and
//! `template_id` is never `0` under a set gate: exactly what a read-failure path
//! looks like when the reads all succeed, the same shape d63 found for its
//! gates. That is a fact about one capture and not a reason to drop the state —
//! the state exists because the collector's read can fail
//! (`attach_log.rs:16697`-`:16717`), and a schema that could not say so would
//! have to lie the first time it does. Across 470,372 entry rows there are 825
//! distinct variants, `amount_raw` runs 5,100 to 160,000,000 (0.051 to 1,600
//! units) and 1,321 of them are not a whole unit, so the fixed-point scale is
//! load-bearing rather than decorative.
//!
//! **The `sparse_partial` status mode does not change any of that.** d26 is one
//! of four domains with an expected-partial tolerance
//! (`compact_contract/src/lib.rs:137`-`:142`): a frame carrying rows under
//! `COMPACT_STATUS_PARTIAL_DATA` and no other failure bit is applied to the
//! retained accumulator and then marks the **domain-hour** unknown
//! (`world_fold.rs::fold_division_equipment`), because the omitted
//! division identities are not serialized. That unknown is a statement about
//! *which keys the frame failed to mention*, not about any field of any row it
//! did mention — the rows in a sparse frame are ordinary measured rows. A keyed
//! set has no way to say "keys I did not look at may exist"; gap
//! normalisation, which has no implementation in this crate (`lib.rs`). So no
//! field becomes `unknown_capable` because of it, and the gap is reported
//! instead. See [`FrameMode`] for where the mode does bite: it relaxes three of
//! the fold's header invariants.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock; `frame_tick` is taken and ignored so every domain's boundary has one
//! shape.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 26;
/// One row per division root: the identity that owns an equipment inventory.
pub const DIVISIONS: u32 = 1;
/// One row per equipment line, keyed by its root plus the variant.
pub const EQUIPMENT: u32 = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different
/// field. `OWNER_TAG_DB_ID` and `DIV_ID` appear in both collections because they
/// are the same field in both — the division's identity — which is what schema-identity contract
/// forbids reusing an id *against*.
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;
    pub const DIV_ID: u32 = 2;
    pub const TEMPLATE_ID: u32 = 3;
    pub const VARIANT_ID: u32 = 4;
    pub const AMOUNT: u32 = 5;
}

// --- Payload grammar, from `world_fold.rs:810`-`:816` and `:17362`-`:17423`. --

/// `DIVISION_EQUIPMENT_HEADER_LEN`, `world_fold.rs::DIVISION_EQUIPMENT_HEADER_LEN`.
pub const HEADER_LEN: usize = 40;
/// `DIVISION_EQUIPMENT_ROOT_ROW_LEN`, `world_fold.rs::DIVISION_EQUIPMENT_ROOT_ROW_LEN`.
pub const ROOT_ROW_LEN: usize = 24;
/// `DIVISION_EQUIPMENT_ENTRY_ROW_LEN`, `world_fold.rs::DIVISION_EQUIPMENT_ENTRY_ROW_LEN`.
pub const ENTRY_ROW_LEN: usize = 16;

/// `DIVISION_EQUIPMENT_ROOT_FLAG_TEMPLATE_PRESENT`, `world_fold.rs::DIVISION_EQUIPMENT_ROOT_FLAG_TEMPLATE_PRESENT`.
pub const ROOT_FLAG_TEMPLATE_PRESENT: u32 = 1;
/// `DIVISION_EQUIPMENT_ROOT_FLAG_DELETED`, `world_fold.rs::DIVISION_EQUIPMENT_ROOT_FLAG_DELETED`.
pub const ROOT_FLAG_DELETED: u32 = 2;
/// `DIVISION_EQUIPMENT_ROOT_KNOWN_FLAGS`, `world_fold.rs::DIVISION_EQUIPMENT_ROOT_KNOWN_FLAGS`. Any other bit is
/// a hard error (`:17495`) — an unrecognised flag word means the row is not this
/// grammar, so nothing in it can be trusted.
pub const ROOT_KNOWN_FLAGS: u32 = ROOT_FLAG_TEMPLATE_PRESENT | ROOT_FLAG_DELETED;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`. The header's `scale_raw` is asserted
/// equal to it (`ensure_fixed_scale`, `:17414`/`:30245`), so this is the only
/// scale d26 ever uses and a frame declaring another is rejected rather than
/// read at its own scale.
pub const FIXED_SCALE: f64 = 100_000.0;
/// The `scale_raw` word the collector packs (`attach_log.rs:69896`) and the fold
/// requires. Kept as an integer so the header check is exact.
pub const FIXED_SCALE_RAW: u32 = 100_000;

/// `world_fold.rs::fold_division_equipment`: an entry amount must satisfy `0 < x <= 1e15`.
pub const MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
/// `world_fold.rs::fold_division_equipment`: a variant id must satisfy `0 < x <= i32::MAX`.
pub const MAX_VARIANT_ID: u32 = i32::MAX as u32;

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: DIVISIONS,
                // Roots persist across hours and the frame grammar is
                // baseline-clears / delta-upserts / DELETED-removes
                // (`world_fold.rs::fold_division_equipment`), which is exactly
                // a keyed set's grammar — and only a keyed set has deltas (collection-shape contract).
                shape: Shape::KeyedSet,
                fields: vec![
                    // The fold's key is the `(owner_tag_db_id, div_id)` pair
                    // (`world_fold.rs::fold_division_equipment`), both zero-rejected, so
                    // both are concrete in every row.
                    //
                    // `owner_tag_db_id` is a country tag db id — content-defined
                    // (content-identity contract): a manifest must resolve it for it to keep its
                    // meaning under another mod set. d63 keys its countries the
                    // same way and d20 classes its `owner` the same way. This is
                    // a ContentId **key**, the heavier retrofit class.
                    Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
                    // A runtime instance identity created during play: division 5
                    // is a different division next campaign and no manifest
                    // resolves it. `Unsigned`, exactly as d20 classes `div_id`.
                    Field::new(f::DIV_ID, Kind::Unsigned).key(1),
                    // Also a runtime instance identity (d20 classes `template_id`
                    // `Unsigned` too). Unknown-capable and nothing else — see the
                    // module docs.
                    Field::new(f::TEMPLATE_ID, Kind::Unsigned).unknown_capable(),
                ],
            },
            Collection {
                collection_id: EQUIPMENT,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
                    Field::new(f::DIV_ID, Kind::Unsigned).key(1),
                    // An equipment variant id. `ContentId`, not `Unsigned`: the
                    // replay layer cannot say what a variant *is* without a
                    // content-side catalog — the id resolves through the
                    // `templates_equipment_catalog` static artifact
                    // (`compact_contract/src/lib.rs:193`) and the archetype
                    // sidecar that carries `archetype_token` and a
                    // `static_catalog_hash`
                    // (`replay_core/src/world.rs:17145`-`:17160`, `:23277`-`:23292`),
                    // and the fold itself leaves `"archetype"` and `"equip_type"`
                    // null because it has no catalog (`world_fold.rs:24171`).
                    // Under another mod set the same number names other
                    // equipment, which is content-identity contract's deciding question. A ContentId
                    // **key**, so the heavier retrofit class again.
                    Field::new(f::VARIANT_ID, Kind::ContentId).key(2),
                    // `Continuous` because the fold's projection is
                    // `amount_raw as f64 / scale_raw` (`world_fold.rs:24170`) and
                    // the replay leaf is an `f64` (`world.rs:476`).
                    //
                    // `Quant::Exact` on derived-field contract's own default: "a field is exact
                    // unless its quantization is justified in the domain
                    // evidence",
                    // records no precision for the amount at all. The outcome
                    // test has a live edge here too — that document's stated
                    // replay use is exact per-variant old/new/delta arithmetic
                    // with an `unexplained_influx` conservation badge, which a
                    // rounded amount would falsify. See the deliverable's open
                    // questions for the `Fixed(5)` alternative and what would
                    // settle it.
                    Field::new(f::AMOUNT, Kind::Continuous).quant(Quant::Exact),
                ],
            },
        ],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 40-byte payload header (`world_fold.rs::fold_division_equipment`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub scale_raw: u32,
    pub root_rows: u32,
    pub observed_division_rows: u32,
    pub equipment_entry_count: u32,
    pub observed_equipment_rows: u32,
}

/// Which of the fold's three frame modes this payload is in.
///
/// The classification is the **caller's**, deliberately: it is decided from
/// `status_flags` by `compact_contract`'s own predicates
/// (`COMPACT_STATUS_DELTA_DATA` for `delta`, and
/// `expected_partial_tolerance_applies` together with `row_count > 0` for
/// `sparse_partial`, `world_fold.rs::fold_division_equipment`), and this crate's library half must
/// not depend on `compact_contract` (`Cargo.toml`) — nor copy its constants.
/// Passing the mode in keeps one derivation of the status vocabulary in the
/// crate that owns it.
///
/// The mode selects three of the fold's header invariants and nothing else. A
/// full baseline serializes every root, so `row_count == root_rows`,
/// `root_rows == observed_division_rows` and
/// `equipment_entry_count == observed_equipment_rows` all hold (`world_fold.rs::fold_division_equipment`).
/// A delta serializes only changed roots and a sparse partial only the roots it
/// managed to sample, so for those the equalities are not invariants and
/// asserting them would reject healthy frames.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameMode {
    /// `status_flags & COMPACT_STATUS_DELTA_DATA != 0` (`world_fold.rs::fold_division_equipment`).
    pub delta: bool,
    /// `row_count > 0 && compact_expected_partial_tolerance_applies(26, status_flags)`
    /// (`world_fold.rs::fold_division_equipment`).
    pub sparse_partial: bool,
}

impl FrameMode {
    /// A clean full baseline: every root serialized, no tolerated partial.
    pub const FULL: Self = Self { delta: false, sparse_partial: false };
    /// A clean delta: only changed roots serialized.
    pub const DELTA: Self = Self { delta: true, sparse_partial: false };
}

/// One parsed root row. `entry_first`/`entry_count` are kept because the caller
/// needs them to find the row's entries, not because they are state: they index
/// the frame's shared entry array and are re-packed from emission order every
/// frame (`attach_log.rs:69904`-`:69920`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DivisionRow {
    pub owner_tag_db_id: u32,
    pub div_id: u32,
    pub flags: u32,
    /// `None` when `TEMPLATE_PRESENT` is clear — the collector could not read
    /// the template, which is record-state contract's `unknown`.
    pub template_id: Option<u32>,
    pub entry_first: usize,
    pub entry_count: usize,
    /// `DELETED`: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
}

/// One parsed entry row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EquipmentRow {
    pub variant_id: u32,
    pub amount_raw: i64,
}

/// Read the 40-byte payload header and nothing else
/// (`world_fold.rs::fold_division_equipment`).
///
/// [`FrameMode`] is decided from `status_flags` and `row_count`, both of which
/// live in this header, so a caller cannot hand [`split_payload`] a mode without
/// first reading it. Exposing the header read is what keeps that from becoming a
/// second, hand-rolled copy of the header layout in every caller. It applies no
/// invariant beyond the length needed to read the words themselves.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "division_equipment payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        scale_raw: u32_at(payload, 20),
        root_rows: u32_at(payload, 24),
        observed_division_rows: u32_at(payload, 28),
        equipment_entry_count: u32_at(payload, 32),
        observed_equipment_rows: u32_at(payload, 36),
    })
}

/// Split a d26 DATA payload into its header, its root rows and its entry rows,
/// applying every structural check `fold_division_equipment` applies before it
/// starts folding (`world_fold.rs::fold_division_equipment`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing. It is fail-closed
/// and total — no input of any length panics.
pub fn split_payload(
    payload: &[u8],
    mode: FrameMode,
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;

    // `ensure_fixed_scale`, `world_fold.rs:17414`. A frame declaring another
    // scale is not this grammar; reading it at 100000 anyway would be wrong by
    // whatever factor the two differ.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "division_equipment scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }

    // Checked arithmetic throughout: both counts are attacker-shaped words from
    // a file on disk, and either product overflows `usize` for a large enough
    // one. `world_fold.rs::fold_division_equipment` uses saturating arithmetic, which turns an
    // overflowing claim into a length that cannot match; rejecting is the same
    // outcome reached without relying on that.
    let roots_len = (header.row_count as usize)
        .checked_mul(ROOT_ROW_LEN)
        .ok_or_else(|| format!("division_equipment row_count {} overflows", header.row_count))?;
    let entries_len = (header.equipment_entry_count as usize)
        .checked_mul(ENTRY_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "division_equipment equipment_entry_count {} overflows",
                header.equipment_entry_count
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(roots_len)
        .and_then(|len| len.checked_add(entries_len))
        .ok_or_else(|| "division_equipment payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!(
            "division_equipment len {} != {expected}",
            payload.len()
        ));
    }

    // Header count invariants. `world_fold.rs::fold_division_equipment`; the three
    // equalities that hold only for a clean full frame are gated on the mode.
    if !mode.delta && !mode.sparse_partial && header.row_count != header.root_rows {
        return Err(format!(
            "division_equipment row_count {} != root_rows {}",
            header.row_count, header.root_rows
        ));
    }
    if header.changed_rows > header.row_count {
        return Err(format!(
            "division_equipment changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if mode.delta && header.changed_rows != header.row_count {
        return Err(format!(
            "division_equipment delta changed_rows {} != row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.observed_division_rows > header.root_rows {
        return Err(format!(
            "division_equipment observed_division_rows {} > root_rows {}",
            header.observed_division_rows, header.root_rows
        ));
    }
    if !mode.delta && !mode.sparse_partial && header.root_rows != header.observed_division_rows {
        return Err(format!(
            "division_equipment full root_rows {} != observed_division_rows {}",
            header.root_rows, header.observed_division_rows
        ));
    }
    if !mode.delta
        && !mode.sparse_partial
        && header.equipment_entry_count != header.observed_equipment_rows
    {
        return Err(format!(
            "division_equipment full equipment_entry_count {} != observed_equipment_rows {}",
            header.equipment_entry_count, header.observed_equipment_rows
        ));
    }
    if header.equipment_entry_count > header.observed_equipment_rows {
        return Err(format!(
            "division_equipment emitted entries {} > observed_equipment_rows {}",
            header.equipment_entry_count, header.observed_equipment_rows
        ));
    }

    let roots = &payload[HEADER_LEN..HEADER_LEN + roots_len];
    let entries = &payload[HEADER_LEN + roots_len..];

    // The entry-range arithmetic, `world_fold.rs::fold_division_equipment`. Every root's
    // range must land inside the shared array and the ranges must account for
    // every entry row exactly once in total — a frame whose declared entries do
    // not sum to the header's count is malformed, not a frame with spare rows.
    let mut declared_entries = 0usize;
    for (index, row) in roots.chunks_exact(ROOT_ROW_LEN).enumerate() {
        let parsed =
            parse_division_row(row).map_err(|e| format!("division_equipment row {index}: {e}"))?;
        entry_index_range(&parsed, header.equipment_entry_count as usize)
            .map_err(|e| format!("division_equipment row {index}: {e}"))?;
        declared_entries = declared_entries.saturating_add(parsed.entry_count);
    }
    if declared_entries != header.equipment_entry_count as usize {
        return Err(format!(
            "division_equipment entry_count {} != declared {declared_entries}",
            header.equipment_entry_count
        ));
    }

    Ok((header, roots, entries))
}

/// Parse one 24-byte root row, applying every per-row check the fold applies
/// (`world_fold.rs::fold_division_equipment`) except the entry-range bound, which needs
/// the frame's `equipment_entry_count` and lives in [`split_payload`].
pub fn parse_division_row(row: &[u8]) -> core::result::Result<DivisionRow, String> {
    if row.len() != ROOT_ROW_LEN {
        return Err(format!(
            "division_equipment root row length {} != {ROOT_ROW_LEN}",
            row.len()
        ));
    }
    let owner_tag_db_id = u32_at(row, 0);
    let div_id = u32_at(row, 4);
    let flags = u32_at(row, 8);
    let template_word = u32_at(row, 12);
    let entry_first = u32_at(row, 16) as usize;
    let entry_count = u32_at(row, 20) as usize;

    if owner_tag_db_id == 0 {
        return Err("zero owner_tag_db_id".into());
    }
    if div_id == 0 {
        return Err("invalid div_id 0".into());
    }
    if flags & !ROOT_KNOWN_FLAGS != 0 {
        return Err(format!("unknown flags {flags}"));
    }
    let deleted = flags & ROOT_FLAG_DELETED != 0;
    let has_template = flags & ROOT_FLAG_TEMPLATE_PRESENT != 0;
    if deleted {
        // `world_fold.rs::fold_division_equipment`: the only permitted flag on a delete row is
        // DELETED itself, and the rest of the row must be empty.
        if flags != ROOT_FLAG_DELETED || template_word != 0 || entry_count != 0 {
            return Err("delete row has non-delete payload".into());
        }
    } else if !has_template && template_word != 0 {
        // `world_fold.rs::fold_division_equipment`: a gated-off template must pack a zero, which is
        // what makes an ungated non-zero word a malformed row rather than data.
        return Err("template_id without flag".into());
    }

    Ok(DivisionRow {
        owner_tag_db_id,
        div_id,
        flags,
        // `world_fold.rs::fold_division_equipment`: the flag is the whole gate. Note the fold does
        // NOT reject `template_id == 0` with the flag set, and neither does
        // this: the collector cannot produce it (`attach_log.rs:16716` returns
        // `Some` only for `id > 0`), but the fold is the authority and it reads
        // that word as a value. It becomes a real `Zero`, not an empty state.
        template_id: has_template.then_some(template_word),
        entry_first,
        entry_count,
        deleted,
    })
}

/// Parse one 16-byte entry row, applying the fold's per-row checks
/// (`world_fold.rs::fold_division_equipment`).
pub fn parse_equipment_row(row: &[u8]) -> core::result::Result<EquipmentRow, String> {
    if row.len() != ENTRY_ROW_LEN {
        return Err(format!(
            "division_equipment entry row length {} != {ENTRY_ROW_LEN}",
            row.len()
        ));
    }
    let variant_id = u32_at(row, 0);
    let reserved = u32_at(row, 4);
    let amount_raw = i64_at(row, 8);
    if variant_id == 0 || variant_id > MAX_VARIANT_ID {
        return Err(format!("invalid variant_id {variant_id}"));
    }
    if amount_raw <= 0 || amount_raw > MAX_AMOUNT_RAW {
        return Err(format!("invalid amount_raw {amount_raw}"));
    }
    if reserved != 0 {
        return Err("entry reserved field is nonzero".into());
    }
    Ok(EquipmentRow {
        variant_id,
        amount_raw,
    })
}

/// The half-open index range of one root's entry rows within the frame's shared
/// entry array, checked exactly as the fold checks it
/// (`world_fold.rs::fold_division_equipment`).
///
/// [`split_payload`] runs this over every root, and any caller that wants a
/// root's entries runs it again to find them: one derivation of the range rule,
/// so the check and the use cannot disagree. Ranges may legally overlap — the
/// fold constrains only their bounds and their **sum**, never that each entry
/// row is referenced exactly once.
pub fn entry_index_range(
    row: &DivisionRow,
    equipment_entry_count: usize,
) -> core::result::Result<core::ops::Range<usize>, String> {
    if row.deleted {
        // `world_fold.rs::fold_division_equipment`: a delete row's `entry_count` is
        // already zero (`parse_division_row`), and its `entry_first` still has
        // to be in range. The range it names is empty either way.
        if row.entry_first > equipment_entry_count {
            return Err("delete row has non-delete payload".into());
        }
        return Ok(row.entry_first..row.entry_first);
    }
    let end = row
        .entry_first
        .checked_add(row.entry_count)
        .ok_or_else(|| "entry range overflows".to_string())?;
    if end > equipment_entry_count {
        return Err(format!(
            "entry range {}+{} exceeds {equipment_entry_count}",
            row.entry_first, row.entry_count
        ));
    }
    Ok(row.entry_first..end)
}

/// The [`DIVISIONS`] key of one root: `(owner_tag_db_id, div_id)`
/// (`world_fold.rs::fold_division_equipment`).
pub fn division_key(owner_tag_db_id: u32, div_id: u32) -> Key {
    key(&[owner_tag_db_id as u64, div_id as u64])
}

/// The [`EQUIPMENT`] key of one entry: its root's key plus the variant
/// (`world_fold.rs::fold_division_equipment`, the fold's own duplicate-entry key).
pub fn equipment_key(owner_tag_db_id: u32, div_id: u32, variant_id: u32) -> Key {
    key(&[
        owner_tag_db_id as u64,
        div_id as u64,
        variant_id as u64,
    ])
}

/// The [`DIVISIONS`] key of one raw root row.
pub fn key_from_division_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_division_row(row)?;
    Ok(division_key(parsed.owner_tag_db_id, parsed.div_id))
}

/// The [`EQUIPMENT`] key of one raw entry row under one raw root row.
pub fn key_from_equipment_row(
    division_row: &[u8],
    entry_row: &[u8],
) -> core::result::Result<Key, String> {
    let root = parse_division_row(division_row)?;
    let entry = parse_equipment_row(entry_row)?;
    Ok(equipment_key(
        root.owner_tag_db_id,
        root.div_id,
        entry.variant_id,
    ))
}

/// One compact root row of the `divisions` table, as the collector emits it
/// today, onto its stored Record. `row` is the raw 24-byte root row slice (the
/// caller has already consumed the payload header via [`split_payload`]); d26
/// has exactly one root row form, so length selects nothing and a wrong length
/// is rejected. `frame_tick` is derived-field contract's derived-field input
/// (stored = v + c * (frame_tick >> u)); d26 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_division_equipment`: a row the fold
/// would reject is an `Err` here, never a coerced value. A **deleted** row is an
/// `Err` too — it is a removal (`world_fold.rs::fold_division_equipment`) and has no
/// record; a caller must map it to a wire tombstone, and quietly handing one
/// back a template-less row would turn a delete into an upsert. Nothing in this
/// function can panic on any input.
pub fn record_from_division_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d26 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_division_row(row)?;
    if parsed.deleted {
        return Err(format!(
            "division_equipment root owner={} div_id={} is a delete row, not a record",
            parsed.owner_tag_db_id, parsed.div_id
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // A clear TEMPLATE_PRESENT gate means the collector's template read
            // failed, not that the division has no template — record-state contract's `unknown`.
            field::TEMPLATE_ID => match parsed.template_id {
                Some(id) => Value::Unsigned(id as u64),
                None => Value::Unknown,
            },
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "division_equipment divisions has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact entry row of the `equipment` table onto its stored Record. `row`
/// is the raw 16-byte entry row slice; see [`record_from_division_row`] for the
/// `frame_tick` contract.
///
/// `amount` is `amount_raw / FIXED_SCALE` — the fold's own projection
/// (`world_fold.rs:24170`) and the only form the replay layer reads
/// (`replay_core/src/world.rs:476`). The fold's range check runs first, so a
/// row the fold rejects never reaches the division.
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
                    "division_equipment equipment has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
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

    fn divisions() -> Collection {
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

    /// A raw 24-byte root row, built the way the collector packs one
    /// (`attach_log.rs:69905`-`:69920`).
    fn root_row(
        owner_tag_db_id: u32,
        div_id: u32,
        flags: u32,
        template_id: u32,
        entry_first: u32,
        entry_count: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROOT_ROW_LEN);
        for word in [
            owner_tag_db_id,
            div_id,
            flags,
            template_id,
            entry_first,
            entry_count,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), ROOT_ROW_LEN);
        row
    }

    /// A raw 16-byte entry row (`attach_log.rs:69923`-`:69926`).
    fn entry_row(variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(ENTRY_ROW_LEN);
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        row.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(row.len(), ENTRY_ROW_LEN);
        row
    }

    /// A whole payload: header, then root rows, then entry rows.
    fn payload(
        mode: FrameMode,
        roots: &[Vec<u8>],
        entries: &[Vec<u8>],
        overrides: &[(usize, u32)],
    ) -> Vec<u8> {
        let row_count = roots.len() as u32;
        let entry_count = entries.len() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        p.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        // A clean full frame must have root_rows == row_count ==
        // observed_division_rows; a delta leaves the roster larger.
        let root_rows = if mode.delta || mode.sparse_partial {
            row_count + 7
        } else {
            row_count
        };
        p.extend_from_slice(&root_rows.to_le_bytes());
        p.extend_from_slice(&root_rows.to_le_bytes()); // observed_division_rows
        p.extend_from_slice(&entry_count.to_le_bytes());
        let observed_equipment = if mode.delta || mode.sparse_partial {
            entry_count + 11
        } else {
            entry_count
        };
        p.extend_from_slice(&observed_equipment.to_le_bytes());
        for (offset, word) in overrides {
            p[*offset..*offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        for row in roots {
            p.extend_from_slice(row);
        }
        for row in entries {
            p.extend_from_slice(row);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d26 schema is well-formed");
    }

    #[test]
    fn it_is_two_keyed_sets_keyed_by_the_division_and_the_variant() {
        let s = schema();
        assert_eq!(s.collections.len(), 2, "roots and their equipment lines");

        let d = divisions();
        assert_eq!(d.collection_id, DIVISIONS);
        assert_eq!(d.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = d.key_fields();
        assert_eq!(keys.len(), 2, "the fold keys roots by (owner, div_id)");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (content-identity contract), so a manifest must resolve it"
        );
        assert_eq!(keys[1].field_id, field::DIV_ID);
        assert_eq!(
            keys[1].kind,
            Kind::Unsigned,
            "a division id is a runtime instance identity, not content"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(d.fields.len(), 3, "2 key components + template_id");

        let e = equipment();
        assert_eq!(e.collection_id, EQUIPMENT);
        assert_eq!(e.shape, Shape::KeyedSet);
        let keys = e.key_fields();
        assert_eq!(keys.len(), 3, "the root key plus the variant");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[1].field_id, field::DIV_ID);
        assert_eq!(keys[2].field_id, field::VARIANT_ID);
        assert_eq!(
            keys[2].kind,
            Kind::ContentId,
            "an equipment variant needs a content catalog to mean anything"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(e.fields.len(), 4, "3 key components + amount");
    }

    #[test]
    fn exactly_one_field_declares_exactly_one_empty_state() {
        // The schema declares one capability on one field, so the mappers may
        // emit one empty state and only there. Absent would claim the concept
        // does not apply; Null would claim we know there is nothing. The
        // collector clears the template gate only when the read failed
        // (`attach_log.rs:16697`-`:16717`), which is record-state contract's "applicable, not
        // captured".
        for c in [divisions(), equipment()] {
            assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
            assert!(
                c.value_fields().all(|f| !f.nullable),
                "no field is a relation that can point at nothing"
            );
        }
        let unknown_capable: Vec<u32> = divisions()
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(unknown_capable, vec![field::TEMPLATE_ID]);
        assert!(
            equipment().value_fields().all(|f| !f.unknown_capable),
            "an equipment line has no gate: it exists or its key does not"
        );
    }

    #[test]
    fn no_continuous_field_is_quantized() {
        // Step 4's "quantization saves bytes" test does not apply: d26 declares
        // no quantized field. derived-field contract makes `exact` the default and requires the
        // domain document to justify anything else, and
        // No retained evidence specifies reduced precision for the
        // equipment amount — while the replay use it does record (exact
        // per-variant old/new/delta arithmetic and an `unexplained_influx`
        // conservation badge) is an outcome test a rounded amount would falsify.
        let c = equipment();
        let continuous: Vec<&Field> = c
            .value_fields()
            .filter(|f| matches!(f.kind, Kind::Continuous))
            .collect();
        assert_eq!(continuous.len(), 1, "amount is the only continuous field");
        assert_eq!(continuous[0].quant, Quant::Exact);
        assert!(
            divisions()
                .value_fields()
                .all(|f| !matches!(f.kind, Kind::Continuous)),
            "a root row carries no continuous field at all"
        );
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d26 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        for c in schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    #[test]
    fn a_full_root_row_roundtrips_byte_for_byte() {
        let c = divisions();
        let row = root_row(7, 4242, ROOT_FLAG_TEMPLATE_PRESENT, 99, 0, 3);
        let record = record_from_division_row(&c, &row, 0).expect("maps");
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
    fn a_template_less_root_row_roundtrips_byte_for_byte() {
        let c = divisions();
        let record = record_from_division_row(&c, &root_row(7, 4242, 0, 0, 0, 0), 0).expect("maps");
        assert_eq!(record.values[index_of(&c, field::TEMPLATE_ID)], Value::Unknown);
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
    fn an_equipment_row_roundtrips_byte_for_byte() {
        let c = equipment();
        let record = record_from_equipment_row(&c, &entry_row(5081, 123_456_789), 0).expect("maps");
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
    fn the_mappers_emit_the_exact_value_for_each_declared_state() {
        // Every state the schema declares, through the public mappers, from raw
        // row bytes. The schema declares `unknown_capable` on `template_id` and
        // nothing else, so those are the only cases that exist:
        //
        //   * a real value (`template_id` under a set gate, `amount` always),
        //   * a real ZERO — `template_id == 0` with the gate SET. The fold does
        //     not reject it (`world_fold.rs::fold_division_equipment` only rejects a
        //     non-zero word with the gate CLEAR), so it is a value the grammar
        //     admits and record-state contract keeps zero out of the three empty states,
        //   * `unknown` — the gate clear.
        //
        // There is no absent case and no null case because no field declares
        // `.optional()` or `.nullable()`; `Record::encode` would reject either.
        // There is no zero case for `amount`: the collector never emits a
        // zero-amount line (`attach_log.rs:58388`) and the fold rejects
        // `amount_raw <= 0` (`world_fold.rs::fold_division_equipment`), so "none of this variant"
        // is an absent KEY, tested in `an_absent_variant_is_an_absent_key`.
        let d = divisions();
        let template = index_of(&d, field::TEMPLATE_ID);

        let r = record_from_division_row(&d, &root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 77, 0, 0), 0)
            .expect("maps");
        assert_eq!(r.values[template], Value::Unsigned(77), "a real value");

        let r = record_from_division_row(&d, &root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 0, 0, 0), 0)
            .expect("maps");
        assert_eq!(
            r.values[template],
            Value::Unsigned(0),
            "a zero under a SET gate is a measured value, not an empty state"
        );

        let r = record_from_division_row(&d, &root_row(3, 9, 0, 0, 0, 0), 0).expect("maps");
        assert_eq!(
            r.values[template],
            Value::Unknown,
            "a clear gate is an unreadable template, not a null one"
        );

        let e = equipment();
        let amount = index_of(&e, field::AMOUNT);
        let r = record_from_equipment_row(&e, &entry_row(1, 250_000), 0).expect("maps");
        assert_eq!(r.values[amount], Value::Continuous(2.5));
        // The smallest amount the grammar admits is one raw unit, not zero.
        let r = record_from_equipment_row(&e, &entry_row(1, 1), 0).expect("maps");
        assert_eq!(r.values[amount], Value::Continuous(1.0 / FIXED_SCALE));
    }

    #[test]
    fn the_unknown_state_survives_a_roundtrip() {
        let c = divisions();
        let record = record_from_division_row(&c, &root_row(3, 9, 0, 0, 0, 0), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back.values[index_of(&c, field::TEMPLATE_ID)], Value::Unknown);
    }

    #[test]
    fn an_absent_variant_is_an_absent_key() {
        // The fidelity claim behind `amount` declaring no empty state: a zero
        // amount cannot be encoded because it cannot be produced. Both the
        // collector's filter and the fold's range check refuse it.
        let c = equipment();
        for amount_raw in [0i64, -1, MAX_AMOUNT_RAW + 1, i64::MIN, i64::MAX] {
            assert!(
                record_from_equipment_row(&c, &entry_row(1, amount_raw), 0).is_err(),
                "amount_raw {amount_raw} is outside the grammar"
            );
        }
        assert!(record_from_equipment_row(&c, &entry_row(1, MAX_AMOUNT_RAW), 0).is_ok());
        assert!(record_from_equipment_row(&c, &entry_row(1, 1), 0).is_ok());
    }

    #[test]
    fn amount_uses_the_100000_scale_and_template_id_uses_none() {
        // The test that catches a 100000-vs-32768 mapping error, and the one
        // that catches dividing a field the fold emits raw: known fixed-point
        // words against their exact f64 readings.
        let e = equipment();
        let amount = index_of(&e, field::AMOUNT);
        let cases: [(i64, f64); 7] = [
            (100_000, 1.0),
            (1, 1.0 / 100_000.0),
            (250_000, 2.5),
            (50_000, 0.5),
            (100_000_000, 1_000.0),
            (32_768, 32_768.0 / 100_000.0),
            (MAX_AMOUNT_RAW, 1e10),
        ];
        for (raw, expected) in cases {
            let r = record_from_equipment_row(&e, &entry_row(1, raw), 0).expect("maps");
            assert_eq!(
                r.values[amount],
                Value::Continuous(expected),
                "amount_raw {raw} must read {expected} (scale {FIXED_SCALE})"
            );
        }

        let d = divisions();
        let template = index_of(&d, field::TEMPLATE_ID);
        for raw in [1u32, 100_000, 32_768, u32::MAX] {
            let r =
                record_from_division_row(&d, &root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, raw, 0, 0), 0)
                    .expect("maps");
            assert_eq!(
                r.values[template],
                Value::Unsigned(raw as u64),
                "template_id is emitted raw by the fold, never divided"
            );
        }
    }

    #[test]
    fn the_root_mapper_rejects_every_row_the_fold_rejects() {
        let c = divisions();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROOT_ROW_LEN - 1, ROOT_ROW_LEN + 1, 48] {
            assert!(
                record_from_division_row(&c, &vec![0u8; len], 0).is_err(),
                "length {len} must be rejected, not read"
            );
        }
        // Zero owner_tag_db_id and zero div_id (`world_fold.rs::fold_division_equipment`).
        assert!(record_from_division_row(&c, &root_row(0, 9, 0, 0, 0, 0), 0).is_err());
        assert!(record_from_division_row(&c, &root_row(3, 0, 0, 0, 0, 0), 0).is_err());
        // An unknown flag bit (`:17495`) — every one of them, not one sample.
        for bit in 2..32u32 {
            assert!(
                record_from_division_row(&c, &root_row(3, 9, 1 << bit, 0, 0, 0), 0).is_err(),
                "flag bit {bit} is not in ROOT_KNOWN_FLAGS and must be rejected"
            );
        }
        // A gated-off template carrying a non-zero word (`:17511`).
        assert!(record_from_division_row(&c, &root_row(3, 9, 0, 1, 0, 0), 0).is_err());
        // A delete row carrying anything at all (`:17501`-`:17509`).
        assert!(
            record_from_division_row(&c, &root_row(3, 9, ROOT_FLAG_DELETED, 0, 0, 0), 0).is_err(),
            "a delete row is a tombstone, never a record"
        );
        for bad in [
            root_row(3, 9, ROOT_FLAG_DELETED | ROOT_FLAG_TEMPLATE_PRESENT, 0, 0, 0),
            root_row(3, 9, ROOT_FLAG_DELETED, 5, 0, 0),
            root_row(3, 9, ROOT_FLAG_DELETED, 0, 0, 1),
        ] {
            assert!(parse_division_row(&bad).is_err(), "delete row with a payload");
        }
        // The delete row the fold DOES accept still parses, and still maps to no
        // record — the two are different failures and must stay different.
        let deleted = parse_division_row(&root_row(3, 9, ROOT_FLAG_DELETED, 0, 4, 0))
            .expect("a bare delete row is well-formed");
        assert!(deleted.deleted);
        assert_eq!(deleted.template_id, None);
    }

    #[test]
    fn the_entry_mapper_rejects_every_row_the_fold_rejects() {
        let c = equipment();
        for len in [0usize, 1, ENTRY_ROW_LEN - 1, ENTRY_ROW_LEN + 1, 24] {
            assert!(
                record_from_equipment_row(&c, &vec![0u8; len], 0).is_err(),
                "length {len} must be rejected, not read"
            );
        }
        // variant_id 0 and above i32::MAX (`world_fold.rs::fold_division_equipment`).
        assert!(record_from_equipment_row(&c, &entry_row(0, 1), 0).is_err());
        assert!(record_from_equipment_row(&c, &entry_row(MAX_VARIANT_ID + 1, 1), 0).is_err());
        assert!(record_from_equipment_row(&c, &entry_row(u32::MAX, 1), 0).is_err());
        assert!(record_from_equipment_row(&c, &entry_row(MAX_VARIANT_ID, 1), 0).is_ok());
        // A non-zero reserved word (`:17556`).
        let mut reserved = entry_row(1, 1);
        reserved[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(record_from_equipment_row(&c, &reserved, 0).is_err());
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        // A caller handing over the other collection — or another domain's —
        // must not get a record full of placeholders under this domain's field
        // ids. The two collections are each other's negative case.
        let row = root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 0);
        assert!(record_from_division_row(&equipment(), &row, 0).is_err());
        assert!(record_from_equipment_row(&divisions(), &entry_row(1, 1), 0).is_err());
        let mut c = divisions();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_division_row(&c, &row, 0).is_err());
        let mut c = equipment();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_equipment_row(&c, &entry_row(1, 1), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let roots = vec![root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 2)];
        let entries = vec![entry_row(11, 100_000), entry_row(12, 200_000)];
        let good = payload(FrameMode::FULL, &roots, &entries, &[]);
        let (header, root_bytes, entry_bytes) =
            split_payload(&good, FrameMode::FULL).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.equipment_entry_count, 2);
        assert_eq!(root_bytes, &roots[0][..]);
        assert_eq!(entry_bytes.len(), 2 * ENTRY_ROW_LEN);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len], FrameMode::FULL).is_err(), "len {len}");
            assert!(peek_header(&good[..len]).is_err(), "peek at len {len}");
        }
        // `peek_header` reads the same words `split_payload` returns, which is
        // what lets the caller decide the frame mode without a second copy of
        // the header layout.
        assert_eq!(peek_header(&good).expect("header"), header);
        // A length that does not match the declared counts, both directions.
        assert!(split_payload(&good[..good.len() - 1], FrameMode::FULL).is_err());
        let mut longer = good.clone();
        longer.push(0);
        assert!(split_payload(&longer, FrameMode::FULL).is_err());

        // A wrong scale (`world_fold.rs::fold_division_equipment`).
        let wrong_scale = payload(FrameMode::FULL, &roots, &entries, &[(20, 32_768)]);
        assert!(split_payload(&wrong_scale, FrameMode::FULL).is_err());

        // `changed_rows > row_count` (`:17439`).
        let over = payload(FrameMode::FULL, &roots, &entries, &[(12, 9)]);
        assert!(split_payload(&over, FrameMode::FULL).is_err());
        // `observed_division_rows > root_rows` (`:17451`).
        let over = payload(FrameMode::FULL, &roots, &entries, &[(28, 9)]);
        assert!(split_payload(&over, FrameMode::FULL).is_err());
        // `equipment_entry_count > observed_equipment_rows` (`:17467`).
        let over = payload(FrameMode::FULL, &roots, &entries, &[(36, 1)]);
        assert!(split_payload(&over, FrameMode::FULL).is_err());

        // Counts whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        for offset in [8usize, 32] {
            let mut huge = good.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&huge, FrameMode::FULL).is_err(), "count at +{offset}");
        }

        // A root whose entry range leaves the shared array (`:17516`).
        let bad = payload(
            FrameMode::FULL,
            &[root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 1, 2)],
            &entries,
            &[],
        );
        assert!(split_payload(&bad, FrameMode::FULL).is_err());
        // Declared entries that do not sum to the header count (`:17530`).
        let bad = payload(
            FrameMode::FULL,
            &[root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 1)],
            &entries,
            &[],
        );
        assert!(split_payload(&bad, FrameMode::FULL).is_err());
        // A delete row whose `entry_first` leaves the array (`:17505`).
        let bad = payload(
            FrameMode::DELTA,
            &[root_row(3, 9, ROOT_FLAG_DELETED, 0, 5, 0)],
            &[],
            &[],
        );
        assert!(split_payload(&bad, FrameMode::DELTA).is_err());
        // And one that stays inside it is accepted, so the bound is not off by
        // one: `entry_first == equipment_entry_count` is legal.
        let ok = payload(
            FrameMode::DELTA,
            &[root_row(3, 9, ROOT_FLAG_DELETED, 0, 0, 0)],
            &[],
            &[],
        );
        assert!(split_payload(&ok, FrameMode::DELTA).is_ok());
    }

    #[test]
    fn the_frame_mode_selects_exactly_the_three_gated_invariants() {
        // A delta serializes only the roots that changed, so its `row_count`,
        // `observed_division_rows` and `observed_equipment_rows` legitimately
        // differ from the roster totals. Reading that frame with the full-frame
        // rules would reject a healthy capture — 902 of the 903 d26 hours in
        // `runs/batch1/deepwar_t1` are deltas — and reading a full frame with
        // the delta rules would accept a truncated roster as a baseline.
        let roots = vec![root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 1)];
        let entries = vec![entry_row(11, 100_000)];
        for mode in [FrameMode::DELTA, FrameMode { delta: false, sparse_partial: true }] {
            let p = payload(mode, &roots, &entries, &[]);
            assert!(split_payload(&p, mode).is_ok(), "{mode:?} accepts its own frame");
            assert!(
                split_payload(&p, FrameMode::FULL).is_err(),
                "{mode:?} frame must not pass the full-frame invariants"
            );
        }
        // A delta must still serialize exactly the rows it counts as changed
        // (`world_fold.rs::fold_division_equipment`).
        let p = payload(FrameMode::DELTA, &roots, &entries, &[(12, 0)]);
        assert!(split_payload(&p, FrameMode::DELTA).is_err());
        // A full frame with changed_rows < row_count is legal (`:17439` bounds
        // it, `:17445` only constrains deltas).
        let p = payload(FrameMode::FULL, &roots, &entries, &[(12, 0)]);
        assert!(split_payload(&p, FrameMode::FULL).is_ok());
    }

    #[test]
    fn entry_index_range_names_exactly_the_roots_own_lines() {
        let roots = vec![
            root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 1),
            root_row(3, 10, ROOT_FLAG_TEMPLATE_PRESENT, 2, 1, 2),
        ];
        let entries = vec![
            entry_row(11, 100_000),
            entry_row(12, 200_000),
            entry_row(13, 300_000),
        ];
        let p = payload(FrameMode::FULL, &roots, &entries, &[]);
        let (header, root_bytes, entry_bytes) =
            split_payload(&p, FrameMode::FULL).expect("well-formed");
        let limit = header.equipment_entry_count as usize;
        let parsed: Vec<DivisionRow> = root_bytes
            .chunks_exact(ROOT_ROW_LEN)
            .map(|r| parse_division_row(r).expect("well-formed"))
            .collect();
        assert_eq!(entry_index_range(&parsed[0], limit).expect("in range"), 0..1);
        assert_eq!(entry_index_range(&parsed[1], limit).expect("in range"), 1..3);
        assert_eq!(entry_bytes.len(), 3 * ENTRY_ROW_LEN);

        // A delete row names an empty range at its own `entry_first`.
        let deleted = parse_division_row(&root_row(3, 9, ROOT_FLAG_DELETED, 0, 3, 0))
            .expect("well-formed");
        assert_eq!(entry_index_range(&deleted, limit).expect("in range"), 3..3);

        // Out of range is an error, never a panic and never a short range.
        let out = DivisionRow { entry_first: 3, entry_count: 1, ..parsed[0] };
        assert!(entry_index_range(&out, limit).is_err());
        let overflow = DivisionRow { entry_first: usize::MAX, entry_count: 1, ..parsed[0] };
        assert!(entry_index_range(&overflow, limit).is_err());
        let deleted_out = DivisionRow { entry_first: limit + 1, ..deleted };
        assert!(entry_index_range(&deleted_out, limit).is_err());
    }

    #[test]
    fn overlapping_entry_ranges_are_legal_and_shared_rows_are_still_checked() {
        // The fold constrains each range's bounds and the SUM of the counts
        // (`world_fold.rs::fold_division_equipment`) — never that every entry row is
        // referenced exactly once. Two roots may therefore name the same rows,
        // which leaves other rows referenced by nobody. Those rows are still
        // part of the frame and the fold still decodes and range-checks every
        // one of them (`world_fold.rs::fold_division_equipment`), so `split_payload` must accept the
        // shape and a caller must not treat "unreferenced" as "unchecked".
        let roots = vec![
            root_row(3, 9, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 1),
            root_row(3, 10, ROOT_FLAG_TEMPLATE_PRESENT, 2, 0, 1),
        ];
        let entries = vec![entry_row(11, 100_000), entry_row(12, 200_000)];
        let p = payload(FrameMode::FULL, &roots, &entries, &[]);
        let (header, root_bytes, _) = split_payload(&p, FrameMode::FULL).expect("well-formed");
        assert_eq!(header.equipment_entry_count, 2);
        let parsed: Vec<DivisionRow> = root_bytes
            .chunks_exact(ROOT_ROW_LEN)
            .map(|r| parse_division_row(r).expect("well-formed"))
            .collect();
        assert_eq!(entry_index_range(&parsed[0], 2).expect("in range"), 0..1);
        assert_eq!(entry_index_range(&parsed[1], 2).expect("in range"), 0..1);
    }

    #[test]
    fn the_keys_are_the_folds_own_keys() {
        let row = root_row(7, 4242, ROOT_FLAG_TEMPLATE_PRESENT, 1, 0, 1);
        assert_eq!(key_from_division_row(&row).expect("keys"), key(&[7, 4242]));
        assert_eq!(
            key_from_equipment_row(&row, &entry_row(5081, 1)).expect("keys"),
            key(&[7, 4242, 5081])
        );
        // A row either mapper rejects has no key either.
        assert!(key_from_division_row(&root_row(0, 1, 0, 0, 0, 0)).is_err());
        assert!(key_from_division_row(&[]).is_err());
        assert!(key_from_equipment_row(&row, &entry_row(0, 1)).is_err());
        assert!(key_from_equipment_row(&row, &[]).is_err());
        // The equipment key really is prefixed by its root's key, which is what
        // lets one root's lines be found as a contiguous key range.
        let root_key = key_from_division_row(&row).expect("keys");
        let entry_key = key_from_equipment_row(&row, &entry_row(5081, 1)).expect("keys");
        assert_eq!(&entry_key.0[..2], &root_key.0[..]);
    }

    #[test]
    fn the_boundary_never_panics_on_arbitrary_bytes() {
        // Every public entry point is reachable from a file on disk and must
        // return a diagnosis, never unwind.
        let d = divisions();
        let e = equipment();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 96;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let _ = parse_division_row(&bytes);
            let _ = parse_equipment_row(&bytes);
            let _ = peek_header(&bytes);
            let _ = record_from_division_row(&d, &bytes, state);
            let _ = record_from_equipment_row(&e, &bytes, state);
            let _ = key_from_division_row(&bytes);
            let _ = split_payload(&bytes, FrameMode::FULL);
            let _ = split_payload(&bytes, FrameMode::DELTA);
            let _ = split_payload(
                &bytes,
                FrameMode { delta: true, sparse_partial: true },
            );
        }
    }
}
