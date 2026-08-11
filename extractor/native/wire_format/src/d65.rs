//! d65 `special_project_programs_live` — per-country special-project facilities.
//!
//! Derived from `world_fold.rs:14965`-`:15059`
//! (`fold_special_project_programs`), its payload constants at `:191`-`:196`, the
// @world-fold-authority domain=65 functions=fold_special_project_programs lines=14965-15059 sha256=5cd8a0d5f4dc1a3cb1d55efb7b0e3855e51f1d01748c14e1da6f9b2bd1402e8b closure=289dc127cb60d7de610641a103f2af16b39af21daca0bdcb5795122ba7ca3238
//! status-independent payload authority
//! `decode_compact_special_project_programs_data_payload`
//! (`recorder_host/src/domain_data_authority.rs:12305`-`:12384`), and the
//! collector read that produces the rows
//! (`collector/src/attach_log.rs:64359`-`:64876`, per-row decode at `:64932`).
//! There is no `domains/d65_*.md` dossier; the fold, the authority and the
//! collector read are the whole basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_special_project_programs`): a 24-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `dropped_ambiguous` u32 at `+20` —
//! followed by `row_count` fixed 16-byte rows. The fold requires
//! `payload.len() == 24 + row_count * 16` exactly (`world_fold.rs::fold_special_project_programs`), so there is no
//! legacy short row and the length selects nothing.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id`           | raw | none (key; zero rejected, `world_fold.rs::fold_special_project_programs`) |
//! | `+4`  | u32 | `program_id`                | raw | none (key; zero rejected, `world_fold.rs::fold_special_project_programs`) |
//! | `+8`  | u32 | `province`                  | raw | none |
//! | `+12` | u32 | `unread_prototype_rewards`  | raw | none |
//!
//! **No scale factors anywhere.** Every one of the four words is folded raw into
//! the JSON row (`world_fold.rs:12308`-`:12313`) — there is no `FIXED_SCALE`, no
//! `/32768`, no function-local divisor. The step-6 offset test below is still
//! written field by field, because a transposition between two adjacent u32s is
//! the same failure class a wrong scale would be and no round-trip can see it.
//!
//! **No field can reach any of §3's three empty states, so the schema declares
//! no capabilities** — no `.optional()`, no `.nullable()`, no
//! `.unknown_capable()`. This is not an assumption about the fold's uniform
//! `Null`; it is a property of the collector, which is fail-closed **at row
//! granularity** rather than per field. `read_special_project_program_fields_detailed`
//! reads one CProgram snapshot twice, refuses the row if the two copies differ
//! (`attach_log.rs:64938`), and then takes all four words out of that one
//! coherent snapshot with `?` on every read (`:64973`-`:65014`): an unreadable
//! word, a mismatched owner copy, an out-of-range owner or a mismatched province
//! copy abandons the **whole row**. There is no flag word, no presence bit and no
//! per-field `None` anywhere in the domain. A row that reaches the payload
//! therefore carries four captured values, and a `0` in either value field is a
//! measured zero, never an empty state. Rejecting a row additionally sets
//! `walk_partial`, which clears the row set entirely and raises
//! `COMPACT_STATUS_PARTIAL_DATA` (`:64856`-`:64866`) — an excluded hour, not a
//! partial row.
//!
//! **`changed_rows` is parsed and bounded, never used as a full-replace
//! assertion.** The fold reads `row_count` (`+8`), `status_flags` (`+16`) and
//! `dropped_ambiguous` (`+20`) and **never reads `+12` at all**. d63's
//! `changed_rows == row_count` check exists because `fold_country_core_scalars`
//! reads that word (`world_fold.rs::fold_country_core_scalars`); copying it here would invent an
//! invariant this fold does not state. The one authority that does read `+12` is
//! the status-independent payload decoder, and it requires only
//! `changed_rows <= row_count` (`domain_data_authority.rs:12335`) — that bound,
//! and nothing stronger, is what [`split_payload`] enforces. (The collector
//! happens to write the same word twice, `attach_log.rs:65081`-`:65082`, and all
//! 514 records of the measured capture have them equal; an emitter property is
//! not a grammar rule.)
//!
//! **The `dropped_ambiguous` word is a diagnostic, not a field.** The fold reads
//! it only to emit a note (`world_fold.rs::fold_special_project_programs`) and current exact
//! production always writes zero (`attach_log.rs:64837` assigns the constant `0`).
//! It describes facilities the collector chose not to publish, so it is a
//! property of the read rather than of any row, and there is no row to hang it
//! on. It is parsed into [`PayloadHeader`] and reported, never encoded.
//!
//! **FINDING — a dead fold branch.** `fold_special_project_programs` calls
//! `compact_expected_partial_tolerance_applies(65, status_flags)` at
//! `world_fold.rs::fold_special_project_programs` and, on a true result, marks the d65 leaf unknown for
//! the hour. `COMPACT_EXPECTED_PARTIAL_TOLERANCES`
//! (`compact_contract/src/lib.rs:136`-`:158`) contains **only domains 26, 27, 28
//! and 54**, so the call is constantly false and the branch is unreachable. It is
//! also redundant: the tolerance that would make it fire is
//! `COMPACT_STATUS_PARTIAL_DATA`, which is in `COMPACT_STATUS_FAILURE_MASK`
//! (`compact_contract/src/lib.rs:117`), so the preceding
//! `handle_compact_failure` (`world_fold.rs:12265`) already returns on exactly
//! that status and already marks the domain unknown for the hour
//! (`mark_fold_domain_unknown`, `world_fold.rs::mark_fold_domain_unknown`). Nothing in this module
//! implements the branch, and the measurement harness classifies a
//! `PARTIAL_DATA` record as an excluded failure hour, which is what the code
//! actually does today. Reported to the orchestrator; if a d65 tolerance entry
//! is ever added, the branch becomes live and this module's exclusion rule must
//! be revisited with it.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: `unread_prototype_rewards` is a pending-notification count the player
//! consumes, and the other three words are identities. The mapper still takes
//! `frame_tick` so every domain's boundary has one shape.
//!
//! **The d18 roster is a soft witness that cannot move a row.** The fold seeds
//! `Value::Array(Vec::new())` for every country in the current d18 roster (or, if
//! country identity is not known this hour, for every key already in
//! `self.countries`) and *then* overwrites the owners that actually have rows
//! (`world_fold.rs::fold_special_project_programs`). The seed only ever writes an **empty** array
//! and the row loop runs after it, so no roster state can add, drop or alter a
//! row: the witness decides which countries are recorded as *having no
//! facilities*, which is d18's fact and carries no d65 row. That is why this
//! module needs no roster input, and why the keyed set — whose members are rows —
//! reproduces the fold's row content exactly.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 65;
pub const PROGRAMS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;
    pub const PROGRAM_ID: u32 = 2;
    pub const PROVINCE: u32 = 3;
    pub const UNREAD_PROTOTYPE_REWARDS: u32 = 4;
}

// --- Payload grammar, from `world_fold.rs:191`-`:196` and `:12249`-`:12264`. --

/// `SPECIAL_PROJECT_PROGRAM_HEADER_LEN`, `world_fold.rs:196`. The fold's
/// `need(payload, 24, ...)` at `:12249`, then rows at `24 + i * 16`.
pub const HEADER_LEN: usize = 24;
/// `SPECIAL_PROJECT_PROGRAM_ROW_LEN`, `world_fold.rs:195`. Fixed; there is no
/// legacy form.
pub const ROW_LEN: usize = 16;

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // The key is the fold's own pair, in the fold's own order:
        // `(owner_tag_db_id, program_id)` (`world_fold.rs::fold_special_project_programs`). A duplicate
        // is a rejected frame there — `seen.insert` failing returns `Err`
        // (`world_fold.rs::fold_special_project_programs`) — never a last-writer-wins overwrite, which is
        // what makes the pair a real identity rather than a convenience.
        //
        // `owner_tag_db_id` is `ContentId`: a country tag-db id is
        // content-defined (§5), a manifest must resolve it for it to keep its
        // meaning under another mod set, and d20's `owner`, d62's
        // `owner_tag_db_id` and d63's `tag_db_id` are all classed this way.
        // The §5 assignment now exists, but this mapper still casts the raw u32
        // until its resolver retrofit; that is recorded in the deliverable, and a ContentId
        // **key** is the heavier retrofit class because key components sort and
        // delta-encode by raw value.
        Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
        // `program_id` is `Unsigned`, NOT `ContentId`: it identifies the running
        // program instance, not the special-project definition the instance was
        // started from. The deciding question of §5 — must a content manifest
        // resolve this id for it to keep its meaning? — is answered no: a
        // program is created during play, and program 5 next campaign is
        // whichever program was created fifth. The collector reads it out of the
        // CProgram object itself (`attach_log.rs:64973`), beside that object's
        // own duplicated owner and province words. The capture agrees: all 27
        // rows of every hour carry 27 **distinct** program ids densely filling
        // `1..=29` across 10 different owners, which is a global allocation
        // counter and not a definition id — under a definition id, ten countries
        // choosing 27 pairwise-distinct projects out of a mod set's whole
        // special-project table would be a coincidence. Recorded as an open
        // question with the evidence that would settle it.
        Field::new(f::PROGRAM_ID, Kind::Unsigned).key(1),
        // The province the facility sits in. `ContentId` for the same reason d20
        // classes `location_province` and d62 classes `province_id` that way: a
        // province id is whatever province the mod set defines. Not part of the
        // fold's key, so it rides as a value.
        Field::new(f::PROVINCE, Kind::ContentId),
        // `Unsigned`, not `Monotone`. §4 makes `Monotone` a hint for a counter
        // that *usually* rises, and this one does not: it is a pending-rewards
        // queue that the player consumes, so it is reset as often as it is
        // incremented. Measured on the capture, all five of its transitions in
        // 514 hours are 2 increments (`2->3`, `3->4`) and 3 resets to zero
        // (`4->0`, `4->0`, `6->0`) — declaring `Monotone` would hint the wrong
        // direction on the majority of the domain's only real movement.
        Field::new(f::UNREAD_PROTOTYPE_REWARDS, Kind::Unsigned),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: PROGRAMS,
            // Full replace per country each clean frame
            // (`world_fold.rs::fold_special_project_programs` assigns each owner's whole array),
            // but `(owner, program_id)` is a stable identity that persists across
            // hours — measured churn on the real capture is 5 changed rows in 513
            // transitions with no insert and no delete — and only a keyed set has
            // a delta grammar (§7.3).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 24-byte payload header (`world_fold.rs::fold_special_project_programs`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    /// `+12`. **The fold never reads this word.** Parsed and bounded here
    /// because the payload authority does (`domain_data_authority.rs:12322`,
    /// `:12335`); see the module docs.
    pub changed_rows: u32,
    pub status_flags: u32,
    /// `+20`. A diagnostic count of facilities the collector declined to
    /// publish, not a row field; see the module docs.
    pub dropped_ambiguous: u32,
}

/// Split a d65 DATA payload into its header and its rows slice.
///
/// Carries the fold's structural checks (`world_fold.rs::fold_special_project_programs`: the
/// 24-byte minimum and the exact `24 + row_count * 16` length) plus the one
/// header predicate the status-independent payload authority adds,
/// `changed_rows <= row_count` (`domain_data_authority.rs:12335`). It does
/// **not** require `changed_rows == row_count`: no authority states that, and
/// asserting it would refuse payloads the recorder folds. See the module docs.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "special_project_programs payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        dropped_ambiguous: u32_at(payload, 20),
    };
    // Checked arithmetic: `row_count` is attacker-shaped input from a stream on
    // disk, and `row_count * ROW_LEN` overflows `usize` for a large enough word.
    // The fold multiplies unchecked (`world_fold.rs::fold_special_project_programs`); refusing the
    // overflow is strictly narrower than wrapping into a plausible small length.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "special_project_programs row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "special_project_programs len {} != {expected}",
            payload.len()
        ));
    }
    if header.changed_rows > header.row_count {
        return Err(format!(
            "special_project_programs changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The key of one compact row: the fold's `(owner_tag_db_id, program_id)` pair
/// (`world_fold.rs::fold_special_project_programs`), in the fold's order.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let (owner_tag_db_id, program_id) = identity_of_row(row)?;
    Ok(key(&[owner_tag_db_id as u64, program_id as u64]))
}

/// The row's two key words, with the fold's zero checks (`world_fold.rs:12296`
/// and `:12299`). Both rejections are also the payload authority's
/// (`domain_data_authority.rs:12346`, `:12351`).
pub fn identity_of_row(row: &[u8]) -> core::result::Result<(u32, u32), String> {
    check_row_len(row)?;
    let owner_tag_db_id = u32_at(row, 0);
    if owner_tag_db_id == 0 {
        return Err("special_project_programs row with zero owner tag_db_id".into());
    }
    let program_id = u32_at(row, 4);
    if program_id == 0 {
        return Err("special_project_programs row with zero program_id".into());
    }
    Ok((owner_tag_db_id, program_id))
}

/// One compact row of the `programs` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 16-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d65 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d65 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// d65 needs no typed input beyond the fixed three. The fold reads nothing
/// outside the row to decide a row's content: there is no mode word, no
/// header-carried scale, no frame status bit that changes a field's meaning, no
/// symbol table and no ordinal. The d18 roster it consults cannot move a row —
/// see the module docs.
///
/// Fail-closed throughout, mirroring `fold_special_project_programs`: a row the
/// fold would reject is an `Err` here, never a coerced value. Nothing in this
/// function can panic on any 16-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d65 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    if collection.collection_id != PROGRAMS {
        return Err(format!(
            "special_project_programs cannot fill collection {}",
            collection.collection_id
        ));
    }
    check_row_len(row)?;
    // The key words are not carried in the record body (§3), but their validity
    // is still part of this row's validity.
    let _ = identity_of_row(row)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        // Every field is an unconditional read of one raw u32: no gate, no
        // scale, no empty state. A zero is a measured zero (§3).
        let value = match f.field_id {
            field::PROVINCE => Value::ContentId(u32_at(row, 8) as u64),
            field::UNREAD_PROTOTYPE_REWARDS => Value::Unsigned(u32_at(row, 12) as u64),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field
                // id that means something else.
                return Err(format!(
                    "special_project_programs has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The whole d65 frame grammar: header split, row walk, and the fold's
/// fail-closed duplicate rejection, in one place.
///
/// d65 is full replace, so one clean record **is** one whole state and there is
/// no accumulator to carry. The reason this boundary exists anyway is the
/// duplicate rule: `(owner, program_id)` uniqueness is enforced by the fold
/// after its status check (`world_fold.rs::fold_special_project_programs`) *and* by the
/// status-independent payload authority before it
/// (`domain_data_authority.rs:12356`), so every production reader of a d65
/// payload applies it and no caller should be re-deriving it.
///
/// **Error contract: all-or-nothing.** The fold accumulates into local
/// `per_owner`/`seen` maps and only writes `self.country_mut(..)` after the row
/// loop has completed without error (`world_fold.rs::fold_special_project_programs`), so a
/// rejected row leaves the recorder's prior state untouched. This function
/// mirrors that: on `Err` no partial state escapes.
///
/// This performs **no status classification**. Which records are excluded is the
/// caller's decision, made in the caller's own fold order — see the module docs
/// and the measurement harness.
pub fn state_from_payload(
    collection: &Collection,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<(PayloadHeader, KeyedState), String> {
    let (header, body) = split_payload(payload)?;
    let mut state = KeyedState::new();
    let mut walked = 0usize;
    for row in body.chunks_exact(ROW_LEN) {
        let (owner_tag_db_id, program_id) = identity_of_row(row)?;
        let k = key_from_row(row)?;
        let record = record_from_row(collection, row, frame_tick)?;
        // `world_fold.rs::fold_special_project_programs`: the join key must be unique, fail closed.
        if state.insert(k, record).is_some() {
            return Err(format!(
                "special_project_programs duplicate (owner {owner_tag_db_id}, program_id {program_id})"
            ));
        }
        walked += 1;
    }
    // `split_payload` already tied the payload length to the declared row count;
    // this closes the third side of the triangle against the rows actually
    // walked, so a `chunks_exact` remainder can never pass silently.
    if walked != header.row_count as usize {
        return Err(format!(
            "special_project_programs walked {walked} rows, header declares {}",
            header.row_count
        ));
    }
    Ok((header, state))
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "special_project_programs row length {} != {ROW_LEN}",
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
    use crate::body::Key;
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

    /// A raw 16-byte row, built the way the collector packs one
    /// (`attach_log.rs:65086`-`:65089`).
    fn raw_row(owner_tag_db_id: u32, program_id: u32, province: u32, unread: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [owner_tag_db_id, program_id, province, unread] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    fn full_row() -> Vec<u8> {
        raw_row(68, 14, 12_627, 4)
    }

    /// A whole d65 payload: the 24-byte header then the rows.
    fn payload(changed_rows: u32, status_flags: u32, dropped: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&7u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&changed_rows.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&dropped.to_le_bytes());
        assert_eq!(p.len(), HEADER_LEN);
        for r in rows {
            p.extend_from_slice(r);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d65 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_owner_and_program() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "the fold's join key is the pair");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[1].field_id, field::PROGRAM_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag-db id is content-defined (§5)"
        );
        assert_eq!(
            keys[1].kind,
            Kind::Unsigned,
            "a program id is a runtime instance identity, not mod-set content"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(c.fields.len(), 4, "2 key words + province + unread rewards");
    }

    #[test]
    fn the_collection_declares_no_four_state_capability_at_all() {
        // Recipe step 8 case 3, recorded rather than invented: d65 has no flag
        // word, no presence bit and no per-field failure path, so no field can
        // reach Absent, Null or Unknown and none of the three gets a raw-bytes
        // case. The collector is fail-closed at ROW granularity — an unreadable
        // or incoherent word abandons the whole row
        // (`attach_log.rs:64938`, `:64973`-`:65014`) and raises PARTIAL_DATA
        // over an emptied row set (`:64856`-`:64866`) — so a published row's
        // four words are four captured values. Declaring a capability that the
        // mapper can never emit would cost real bits: the presence bitmap is
        // sized by optional fields and the null bitmap by nullable present ones.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.fields.iter().all(|f| !f.optional),
            "not even a key field declares optional"
        );
        assert!(c.fields.iter().all(|f| !f.nullable), "no field is nullable");
        assert!(
            c.fields.iter().all(|f| !f.unknown_capable),
            "no field is unknown-capable"
        );
    }

    #[test]
    fn no_field_is_quantized_because_none_is_continuous() {
        // Recipe step 8 case 4, recorded rather than skipped: the
        // "quantization saves bytes" comparison has nothing to compare. d65 has
        // no `Kind::Continuous` field at all — all four words are identities or
        // integer counts, and §6 quantization applies only to continuous
        // fields — so there is no drawn-versus-interpolated judgement to make.
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
        // §6 derived fields are clock-linear integers; nothing in d65 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        // And the mapper proves it: the same row under wildly different ticks
        // must produce the same record, or a derived field crept in.
        let c = collection();
        let base = record_from_row(&c, &full_row(), 0).expect("maps");
        for tick in [1u64, 2, 255, 4096, u64::MAX] {
            assert_eq!(
                record_from_row(&c, &full_row(), tick).expect("maps"),
                base,
                "tick {tick} changed the record"
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
    fn every_field_offset_maps_to_its_own_value() {
        // The step-6 test. d65 has no scale factor anywhere, so this is the
        // transposition test rather than the 32768-vs-100000 test: the four
        // words are adjacent u32s and two of them are separated into the key,
        // which is exactly the shape that swaps silently. Every word is given a
        // distinct, recognisable value so a swap cannot pass.
        let c = collection();
        let row = raw_row(0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444);
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE)],
            Value::ContentId(0x3333_3333),
            "province is the u32 at +8"
        );
        assert_eq!(
            r.values[index_of(&c, field::UNREAD_PROTOTYPE_REWARDS)],
            Value::Unsigned(0x4444_4444),
            "unread_prototype_rewards is the u32 at +12"
        );
        assert_eq!(
            key_from_row(&row).expect("keys"),
            key(&[0x1111_1111, 0x2222_2222]),
            "owner at +0 leads program_id at +4"
        );
        // The value fields carry no scale: a word that looks like a fixed-point
        // reading is still emitted raw, whichever divisor a reader might guess.
        let scaled = raw_row(1, 1, 32_768, 100_000);
        let r = record_from_row(&c, &scaled, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE)],
            Value::ContentId(32_768),
            "province is folded raw (`world_fold.rs:12311`), never divided"
        );
        assert_eq!(
            r.values[index_of(&c, field::UNREAD_PROTOTYPE_REWARDS)],
            Value::Unsigned(100_000),
            "unread rewards is folded raw (`world_fold.rs:12312`), never divided"
        );
        // The full u32 range survives the widening to u64 unchanged.
        let wide = raw_row(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        let r = record_from_row(&c, &wide, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE)],
            Value::ContentId(u32::MAX as u64)
        );
        assert_eq!(
            r.values[index_of(&c, field::UNREAD_PROTOTYPE_REWARDS)],
            Value::Unsigned(u32::MAX as u64)
        );
        assert_eq!(
            key_from_row(&wide).expect("keys"),
            key(&[u32::MAX as u64, u32::MAX as u64])
        );
    }

    #[test]
    fn every_header_word_is_read_at_its_own_offset() {
        // The header half of the step-6 test: a transposed header word
        // mis-slices every row behind it, and no round-trip or replay assertion
        // can see that because both arms agree on the wrong slice.
        //
        // Every one of the five words gets a value distinct from every other, so
        // no swap of any pair passes. In particular `row_count` and
        // `changed_rows` are adjacent, both plausible small integers, and the
        // ONLY pair a same-valued fixture would hide — so this fixture uses two
        // rows with `changed_rows = 1`. (A round-1 fixture used one row and
        // `changed_rows = 1`, under which swapping `+8` and `+12` still passed
        // this test.)
        let rows = [full_row(), raw_row(2, 3, 13_070, 2)];
        let mut p = payload(1, 0, 0, &rows);
        p[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        p[16..20].copy_from_slice(&0x0000_0400u32.to_le_bytes()); // status_flags
        p[20..24].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // dropped_ambiguous
        let (h, body) = split_payload(&p).expect("well-formed");
        assert_eq!(h.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(h.row_count, 2, "row_count is the u32 at +8");
        assert_eq!(h.changed_rows, 1, "changed_rows is the u32 at +12");
        assert_eq!(h.status_flags, 0x0000_0400);
        assert_eq!(h.dropped_ambiguous, 0xDEAD_BEEF);
        assert_eq!(body.len(), 2 * ROW_LEN);
        assert_eq!(&body[..ROW_LEN], &full_row()[..]);

        // And the swap this fixture exists to catch, made explicit: a payload
        // whose two count words are exchanged must not split identically. With
        // row_count 1 / changed_rows 2 the length no longer matches the declared
        // count, which is the failure a transposed reader would produce.
        let mut swapped = p.clone();
        swapped[8..12].copy_from_slice(&1u32.to_le_bytes());
        swapped[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&swapped).is_err());
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // §3 keeps zero out of the three empty states. Both value fields can be
        // a genuine zero: a province id of 0 and an unread count of 0 are what
        // the collector publishes when it read exactly that, since neither has a
        // gate to be clear. 8,932 of the capture's 13,878 unread cells are zero.
        let c = collection();
        let r = record_from_row(&c, &raw_row(1, 1, 0, 0), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE)],
            Value::ContentId(0),
            "a zero province is a measured value, not Null"
        );
        assert_eq!(
            r.values[index_of(&c, field::UNREAD_PROTOTYPE_REWARDS)],
            Value::Unsigned(0),
            "a zero unread count is a measured value"
        );
        // And it survives the codec as a value rather than collapsing.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, r);
        assert!(back.values.iter().all(|v| !matches!(
            v,
            Value::Absent | Value::Null | Value::Unknown
        )));
    }

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
            assert!(key_from_row(&row).is_err(), "length {len} keys nothing");
        }
        // Zero owner tag_db_id (`world_fold.rs::fold_special_project_programs`) and zero program_id
        // (`world_fold.rs::fold_special_project_programs`), independently and together, whatever the value words say.
        for (owner, program) in [(0u32, 1u32), (1, 0), (0, 0)] {
            for (province, unread) in [(0u32, 0u32), (13_070, 6)] {
                let row = raw_row(owner, program, province, unread);
                assert!(
                    record_from_row(&c, &row, 0).is_err(),
                    "owner {owner} program {program} must be rejected"
                );
                assert!(key_from_row(&row).is_err());
            }
        }
        // ...and the smallest legal identity is accepted, so the bound is not
        // off by one in the other direction.
        assert!(record_from_row(&c, &raw_row(1, 1, 0, 0), 0).is_ok());
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids. Checked
        // both ways: a wrong collection id, and an unmappable extra field.
        let mut wrong_id = collection();
        wrong_id.collection_id = PROGRAMS + 1;
        assert!(record_from_row(&wrong_id, &full_row(), 0).is_err());

        let mut extra = collection();
        extra.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&extra, &full_row(), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_structural_rules_and_only_those() {
        let rows = [full_row(), raw_row(2, 3, 13_070, 2)];
        let p = payload(2, 0, 0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 2);
        assert_eq!(body.len(), 2 * ROW_LEN);

        // Short of the header, every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // A length that does not match the row count, both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());
        // A `row_count` far larger than the payload is rejected rather than
        // read. Note what this does and does not prove: on a 64-bit target
        // `u32::MAX * 16` does not overflow `usize`, so this is an ordinary
        // length mismatch and it would still fail without the `checked_mul` in
        // `split_payload`. The checked arithmetic is there for a 32-bit target,
        // where the same input wraps to a plausible small length; that branch is
        // unreachable here and is therefore not claimed to be tested.
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());

        // A zero-row payload is well-formed: the fold folds it as a clean
        // full replacement that measures "this world has no facilities"
        // (`attach_log.rs:64846`-`:64850` calls that a successful zero-row
        // replacement), so refusing it would refuse a state the recorder stores.
        let empty = payload(0, 0, 0, &[]);
        let (h, body) = split_payload(&empty).expect("a zero-row payload is legal");
        assert_eq!(h.row_count, 0);
        assert!(body.is_empty());
    }

    #[test]
    fn changed_rows_is_bounded_by_row_count_and_never_forced_to_equal_it() {
        // The fold does not read `+12` at all; the payload authority requires
        // only `changed_rows <= row_count` (`domain_data_authority.rs:12335`).
        // Enforcing d63's equality here would refuse payloads the recorder
        // folds without complaint, which is the "never gate against the fold"
        // rule of recipe step 2 applied to a header word.
        let rows = [full_row(), raw_row(2, 3, 13_070, 2)];
        for changed in 0..=2u32 {
            let p = payload(changed, 0, 0, &rows);
            let (h, _) = split_payload(&p)
                .unwrap_or_else(|e| panic!("changed_rows {changed} <= 2 must be accepted: {e}"));
            assert_eq!(h.changed_rows, changed);
        }
        // Strictly greater is the authority's rejection.
        for changed in [3u32, 4, 100, u32::MAX] {
            let p = payload(changed, 0, 0, &rows);
            assert!(
                split_payload(&p).is_err(),
                "changed_rows {changed} > row_count 2 must be refused"
            );
        }
    }

    #[test]
    fn the_key_is_the_owner_and_program_pair_in_the_folds_order() {
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[68, 14]));
        // Order is load-bearing: the two words are not interchangeable.
        assert_ne!(
            key_from_row(&raw_row(1, 2, 0, 0)).expect("keys"),
            key_from_row(&raw_row(2, 1, 0, 0)).expect("keys")
        );
        assert!(key_from_row(&[]).is_err());
        // The key must not depend on the value words at all.
        let a: Key = key_from_row(&raw_row(5, 6, 1, 1)).expect("keys");
        let b: Key = key_from_row(&raw_row(5, 6, 999, 42)).expect("keys");
        assert_eq!(a, b);
    }

    #[test]
    fn state_from_payload_walks_rows_and_rejects_a_duplicate_pair() {
        let c = collection();
        let rows = [
            full_row(),
            raw_row(2, 3, 13_070, 2),
            raw_row(68, 6, 6_707, 6),
        ];
        let (header, state) =
            state_from_payload(&c, &payload(3, 0, 0, &rows), 0).expect("well-formed");
        assert_eq!(header.row_count, 3);
        assert_eq!(state.len(), 3);
        assert_eq!(
            state.keys().next().expect("first"),
            &key(&[2, 3]),
            "the keyed set orders by the key pair, not by row order"
        );

        // `world_fold.rs::fold_special_project_programs`: the join key is unique, fail closed. Same pair
        // with identical value words and with differing ones — the fold rejects
        // on the pair alone, so both must be refused rather than collapsed.
        for second in [full_row(), raw_row(68, 14, 1, 1)] {
            let dup = payload(2, 0, 0, &[full_row(), second]);
            assert!(
                state_from_payload(&c, &dup, 0).is_err(),
                "a duplicate (owner, program_id) must refuse the frame"
            );
        }
        // A repeated owner with a different program is NOT a duplicate, and
        // neither is a repeated program id under a different owner: it is the
        // pair that is unique. Two facilities in one country is the normal case.
        let ok = payload(2, 0, 0, &[raw_row(68, 14, 1, 0), raw_row(68, 15, 2, 0)]);
        assert_eq!(state_from_payload(&c, &ok, 0).expect("legal").1.len(), 2);
        let ok = payload(2, 0, 0, &[raw_row(68, 14, 1, 0), raw_row(69, 14, 2, 0)]);
        assert_eq!(state_from_payload(&c, &ok, 0).expect("legal").1.len(), 2);

        // A rejected row leaves nothing behind: the fold builds its maps locally
        // and only writes the accumulator after a clean loop
        // (`world_fold.rs::fold_special_project_programs`), so `Err` must carry no partial state.
        let bad = payload(2, 0, 0, &[full_row(), raw_row(0, 1, 0, 0)]);
        assert!(state_from_payload(&c, &bad, 0).is_err());

        // A zero-row payload is an empty state, not an error.
        let (h, empty) = state_from_payload(&c, &payload(0, 0, 0, &[]), 0).expect("legal");
        assert_eq!(h.row_count, 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn state_from_payload_classifies_no_status_bit() {
        // Exclusion is the caller's decision, made in the caller's fold order.
        // This boundary must behave identically whatever the status word says,
        // or a harness that calls it before its own status check (which is what
        // the payload authority's status-independence requires) would silently
        // get a different answer for a failure-status record.
        let c = collection();
        let rows = [full_row()];
        let baseline = state_from_payload(&c, &payload(1, 0, 0, &rows), 0).expect("clean");
        for bit in 0..32u32 {
            let (h, state) = state_from_payload(&c, &payload(1, 1 << bit, 0, &rows), 0)
                .unwrap_or_else(|e| panic!("status bit {bit} must not change the walk: {e}"));
            assert_eq!(h.status_flags, 1 << bit);
            assert_eq!(state, baseline.1, "status bit {bit} changed the state");
        }
        // The same for the diagnostic `dropped_ambiguous` word, which describes
        // the read rather than any row (`world_fold.rs::fold_special_project_programs`).
        for dropped in [0u32, 1, 7, u32::MAX] {
            let (h, state) =
                state_from_payload(&c, &payload(1, 0, dropped, &rows), 0).expect("legal");
            assert_eq!(h.dropped_ambiguous, dropped);
            assert_eq!(state, baseline.1);
        }
    }

    #[test]
    fn the_boundary_never_panics_on_arbitrary_bytes() {
        // Every public entry point is total over its input: the harness runs
        // them on bytes read from a file, and a panic there is an unrecoverable
        // measurement rather than a diagnosis.
        let c = collection();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_096 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state % 96) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|i| (state >> ((i % 8) * 8)) as u8)
                .collect();
            let _ = split_payload(&bytes);
            let _ = key_from_row(&bytes);
            let _ = identity_of_row(&bytes);
            let _ = record_from_row(&c, &bytes, state);
            let _ = state_from_payload(&c, &bytes, state);
        }
    }
}
