//! d17 `diplomacy_live` — per-country diplomatic standing, and a **stopped**
//! war-graph collection.
//!
//! Derived from `world_fold.rs:22966`-`:23040`
//! (`fold_diplomacy`), the `ascii8` token decoder it calls (`world_fold.rs::ascii8`), and the
// @world-fold-authority domain=17 functions=fold_diplomacy lines=22966-23040 sha256=e64b09bdc4f6f1c9569251327f9be15be01460123bddfe735e967cbb40c9ce29 closure=8aaad3eb946e4a0d360d00fa59fb1763577b9d4eb40f6df1f9ee1d8445646e1d
//! collector reads and writes that produce the payload
//! (`extractor/native/collector/src/attach_log.rs:69650`-`:69710` writer,
//! `:80136`-`:80290` reads, `:59759`-`:59780` war-segment accounting).
//! Status bit 13 is d17-only informational war-segment accounting outside the
//! shared failure mask, matching the fold.
//!
//! **Payload grammar** (`world_fold.rs::fold_diplomacy`, written at
//! `attach_log.rs:69666`-`:69708`): a 24-byte header — `elapsed_wall_ms` u64 at
//! `+0`, `country_count` u32 at `+8`, `changed_rows` u32 at `+12`,
//! `status_flags` u32 at `+16`, `relation_count` u32 at `+20` — followed by
//! `country_count` fixed 24-byte country rows and then `relation_count` fixed
//! 16-byte at-war rows. There is no legacy short row in either table: the fold
//! walks both with fixed strides and then requires the cursor to land exactly on
//! the payload end (`:13902`), so a length never selects a row form.
//!
//! | table | offset | width | field | scale | gate |
//! |---|---|---|---|---|---|
//! | country | `+0`  | u32 | `tag_db_id`     | raw | none (key) |
//! | country | `+4`  | u32 | `flags`         | —   | — |
//! | country | `+8`  | i32 | `faction_group` | raw | none |
//! | country | `+12` | u32 | `at_war_count`  | raw | `1<<0` |
//! | country | `+16` | i64 | `casualties`    | raw | `1<<1` |
//! | at-war  | `+0`  | u32 | `tag_db_id`     | raw | none |
//! | at-war  | `+4`  | u32 | `token_len`     | raw | none |
//! | at-war  | `+8`  | 8×u8| `enemy_tag`     | raw | none |
//!
//! **No scale anywhere in this domain.** Every field is folded raw
//! (`:13879`-`:13886`); there is no fixed-point word here and nothing is
//! divided. `casualties` in particular is a raw lifetime count summed from
//! per-relation int32 casualty counters (`attach_log.rs:80272`-`:80278`), not a
//! scaled quantity.
//!
//! # STOP CONDITION — the at-war table is NOT ported
//!
//! The at-war table identifies the enemy by an **8-byte inline ASCII country
//! TAG** with a `u32` length prefix (`attach_log.rs:71935`-`:71942`), decoded by
//! `ascii8` (`world_fold.rs::ascii8`). There is no conformant way to store that
//! today, and both candidate encodings are wrong for the same reason:
//!
//! * schema-kind contract has **no string kind**. Every scalar kind is a number.
//! * content-identity contract says, of exactly this datum: "Country tags, equipment names, province and
//!   state ids are content-defined. The stream carries **manifest ids as
//!   varints, never text**." So the conformant field is a `ContentId` holding a
//!   manifest symbol id. This older mapper has not yet received the file-global
//!   resolver that now owns that assignment.
//! * Packing the token's bytes into a numeric word is reversible and lossless,
//!   but it is still text on the wire. Declaring that word a `ContentId` would
//!   assert it is a resolved manifest symbol id, which is false and would make
//!   every future content-identity contract retrofit sweep skip it; declaring it `Unsigned` states the
//!   truth about the container while still shipping embedded text, which is what
//!   content-identity contract forbids. Neither is conformant, so neither is shipped.
//!
//! The recipe's rule for this situation is to report rather than build, and that
//! is what happens: [`schema`] declares **one** collection, the country table,
//! and the at-war table is decoded, censused and reported by the measurement
//! harness ([`at_war_owner_of_row`], [`enemy_tag_word`], [`enemy_tag_token_len`],
//! [`enemy_tag_string`], [`enemy_tag_is_collector_shaped`]) without being
//! encoded. **Nothing captured is silently dropped**: the harness counts every
//! at-war row it cannot store and prints the count, and the fact each row
//! contributes to — *how many* countries this country is at war with — is stored
//! in full, on the country row's `at_war_count` field. What the stop costs is
//! the enemies' identities, and what it needs is its content-identity contract resolver retrofit.
//! Field ids `5` and `6` are reserved for that collection so a later port does
//! not reuse them for something else (schema-identity contract is append-only).
//!
//! A second, independent blocker sits on the same collection, recorded here so a
//! future port does not discover it late: even with a symbol id, a keyed set has
//! no way to say "this collection's membership was not measured this hour",
//! which is the state **903 of 903 hours** of the one available capture are in
//! (see the war-segment section below). That is gap normalization, which has
//! no implementation in this crate (`lib.rs`).
//!
//! # The row `flags` word: what the fold discards
//!
//! The fold reads the country row's flag word into `_flags` and drops it
//! (`world_fold.rs::fold_diplomacy`), then emits `at_war_count` and `casualties` raw. The
//! writer, however, sets bit `0` exactly when `at_war` was captured and bit `1`
//! exactly when `casualties` was captured, and packs a **zero** in the field
//! otherwise (`attach_log.rs:69679`-`:69698`). So a fold reading `casualties: 0`
//! cannot tell a country that has taken no casualties from one whose war data
//! was never read — which is precisely the distinction record-state contract exists to keep, and
//! precisely the one the collector went out of its way to record
//! (`attach_log.rs:59771`-`:59779`, quoted below). This schema keeps both flags
//! as gates and reports the difference; that is a fidelity gain over the fold
//! and is recorded as a fold-vs-mapper divergence in the deliverable.
//!
//! # `at_war_count` / `casualties`: three states, decided by the frame's status
//!
//! Both fields are populated by `apply_war_relations_to_country_rows`
//! (`attach_log.rs:80257`-`:80289`), which fills a country's entry **only when
//! that country appears in at least one war relation**. So a clear gate means
//! one of two different things, and the frame's status word says which:
//!
//! * `COMPACT_STATUS_WAR_SEGMENT_ABSENT` (bit 13) **clear** — the war-relations
//!   segment was read and applied (`attach_log.rs:59764`-`:59770`). A country
//!   with a clear gate was compared against the whole relation table and appears
//!   in none: it is at war with nobody and has no war casualties. That is a real
//!   measured **zero** (record-state contract), not an empty state.
//! * bit 13 **set** — the segment did not capture at all (no bridge, an invalid
//!   bridge, a bridge read that returned nothing, or a panicked country worker:
//!   `attach_log.rs:59776`-`:59779` and `:44980`-`:44988`). The collector's own
//!   words for why the bit exists: *"a diplomacy frame whose war-relations
//!   segment did not capture … says so FIRST-CLASS — per-row `at_war: None` is
//!   otherwise ambiguous with 'not at war'."* That is record-state contract's **unknown**,
//!   "applicable, not captured", word for word.
//!
//! Neither field is ever `null` (nothing here is a relation that points at
//! nothing) and neither is ever `absent` (both apply to every country row the
//! payload carries), so both declare `unknown_capable` and nothing else. The
//! status bit is a *frame* property, so it enters the mapper as [`WarSegment`]
//! rather than being read from a constant this crate must not depend on.
//!
//! A gated-off field must pack a zero, and this mapper enforces that: without
//! it, a writer that clears the gate while packing a real value would have that
//! value silently replaced by `Unknown` here while the fold still emitted it.
//! With it, the row is either lossless or refused.
//!
//! # `changed_rows` is parsed and not enforced
//!
//! The fold never reads `+12` (`world_fold.rs::fold_diplomacy` read `+8` and
//! `+20` and nothing else), so d17 states no `changed_rows` invariant for
//! [`split_payload`] to carry. The writer happens to store `country_count`
//! twice (`attach_log.rs:69673`-`:69674`), and all 903 records of the capture
//! agree — but asserting an equality no authority states is the defect the
//! recipe records against d10, so the field is parsed, reported, and left
//! unchecked.
//!
//! # The accumulator
//!
//! d17 is **not** full-replace despite its `(full each frame)` banner comment.
//! `fold_diplomacy` writes one country object per tag *present in the payload*
//! (`:13908`-`:13911`) and touches nothing else; no path in `world_fold.rs`
//! removes a country's `diplomacy`. So the state is a retained accumulator with
//! upsert-only country rows and no deletion — 901 of the capture's 903 frames
//! carry zero rows and are therefore no-ops, not mass deletions. [`Accumulator`]
//! is the shared implementation; the measurement bin, the tests and any future
//! caller use it rather than re-deriving the rule. The other writers of the
//! fold's `diplomacy` object (`world_fold.rs::project_market_trade_lend_lease` lend-lease projection, `world_fold.rs::project_domain53_country_overlays` d53
//! overlay) add their own keys and never touch these three fields.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock; the mapper takes `frame_tick` and ignores it so every domain's
//! boundary has one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 17;
/// One row per country the payload carries.
pub const COUNTRIES: u32 = 1;
/// Reserved for the war-graph collection, which is a reported stop condition
/// (see the module docs). No schema declares it; the id is held so a later port
/// does not reuse it.
pub const AT_WAR_RESERVED: u32 = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different
/// field.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const FACTION_GROUP: u32 = 2;
    pub const AT_WAR_COUNT: u32 = 3;
    pub const CASUALTIES: u32 = 4;
    /// Reserved for the stopped war-graph collection's enemy symbol. Not
    /// emitted; held so a later port does not reuse the id.
    pub const ENEMY_TAG_RESERVED: u32 = 5;
    /// Reserved likewise for that collection's token length prefix.
    pub const ENEMY_TAG_TOKEN_LEN_RESERVED: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs::fold_diplomacy`. -------------------

/// `world_fold.rs::fold_diplomacy` — `need(payload, 24, "diplomacy header")`.
pub const HEADER_LEN: usize = 24;
/// `world_fold.rs::fold_diplomacy` — `need(payload, cursor + 24, "diplomacy country row")`.
pub const COUNTRY_ROW_LEN: usize = 24;
/// `world_fold.rs::fold_diplomacy` — `need(payload, cursor + 16, "diplomacy at-war row")`.
pub const AT_WAR_ROW_LEN: usize = 16;
/// The at-war row's inline token field, `world_fold.rs::fold_diplomacy`.
pub const ENEMY_TAG_OFFSET: usize = 8;
pub const ENEMY_TAG_BYTES: usize = 8;

/// `attach_log.rs:69680`-`:69682` — set exactly when `at_war` was captured.
pub const FLAG_AT_WAR: u32 = 1 << 0;
/// `attach_log.rs:69683`-`:69685` — set exactly when `casualties` was captured.
pub const FLAG_CASUALTIES: u32 = 1 << 1;

/// The `token_len` every row the collector can write carries: `is_country_tag`
/// admits exactly three bytes (`attach_log.rs:86688`) and
/// `append_fixed_ascii_token8` stores `min(len, 8)` (`:71937`).
pub const COLLECTOR_TOKEN_LEN: u32 = 3;

/// Whether the frame's war-relations segment captured, which is the frame's
/// `COMPACT_STATUS_WAR_SEGMENT_ABSENT` bit (bit 13) read by the caller.
///
/// It is a parameter rather than a bit tested here because that constant belongs
/// to `compact_contract`; d17 is not admitted to the production-codec
/// dependency surface, and copying the literal into this module is exactly the
/// drift the recipe forbids.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WarSegment {
    /// Bit 13 clear: the segment was read and applied. A clear per-row gate is a
    /// measured "no wars".
    Captured,
    /// Bit 13 set: the segment did not capture. A clear per-row gate is record-state contract's
    /// unknown.
    Absent,
}

pub fn schema() -> Schema {
    use field as f;
    let countries = Collection {
        collection_id: COUNTRIES,
        // Upsert accumulator across hours (see the module docs): a country's
        // `tag_db_id` is a stable identity that persists, which is what a keyed
        // set needs — and only a keyed set has a delta grammar (collection-shape contract).
        shape: Shape::KeyedSet,
        fields: vec![
            // The country's `tag_db_id` (`world_fold.rs::fold_diplomacy`), the identity d18
            // keys its roster by. `ContentId` because a country is content-defined
            // (content-identity contract) — a manifest must resolve it for it to keep its meaning under
            // another mod set. This is the raw-source-id cast every ported domain
            // makes today pending the manifest-id assignment rule; d63's `tag_db_id` and
            // d20's `owner` are the same call, and it is listed in the
            // deliverable's open questions as a ContentId KEY, whose retrofit
            // relayouts baselines rather than one field's value.
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            // An opaque engine `i32` read from the country object at `+0x598`
            // (`attach_log.rs:80137`, `offsets.rs:154`) and folded raw
            // (`world_fold.rs::fold_diplomacy`). `Signed`, not `ContentId`: no authority
            // says a manifest resolves it, and the read is unfiltered `i32`, so
            // a negative value is representable and `ContentId`'s unsigned
            // domain could not carry it. Not gated — the whole country row is
            // omitted when this read fails (`attach_log.rs:80137`'s `?`), so
            // a row that exists always has it, and every value including 0 is
            // real.
            Field::new(f::FACTION_GROUP, Kind::Signed),
            // Count of live war relations naming this country. A cardinality,
            // not a counter: `Unsigned`. `unknown_capable` for the
            // segment-absent case; see the module docs. This is also the whole
            // of what survives the war-graph stop condition, and it survives
            // exactly, which is why the stop costs identities and not facts.
            Field::new(f::AT_WAR_COUNT, Kind::Unsigned).unknown_capable(),
            // Lifetime war casualties, summed raw over this country's side of
            // every live relation (`attach_log.rs:80272`-`:80278`). `Monotone`
            // is an encoding hint and never a validity constraint (schema-kind contract), so a
            // total that drops when a war ends still declares it — d20's
            // `cumulative_losses` is the same case.
            Field::new(f::CASUALTIES, Kind::Monotone).unknown_capable(),
        ],
    };
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![countries],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 24-byte payload header (`world_fold.rs::fold_diplomacy`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    /// Parsed and reported, never enforced — the fold does not read `+12`. See
    /// the module docs.
    pub changed_rows: u32,
    pub status_flags: u32,
    pub relation_count: u32,
}

/// Split a d17 DATA payload into its header, its country rows and its at-war
/// rows, applying the fold's own structural checks
/// (`world_fold.rs::fold_diplomacy`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
///
/// The fold's structural rule is the conjunction of its per-row `need` calls and
/// its closing "trailing bytes" check (`:13902`), which is exactly
/// `len == 24 + country_count * 24 + relation_count * 16`. The multiplications
/// are checked: both counts are attacker-shaped words from a file on disk.
///
/// The at-war slice is returned even though no collection stores it — a caller
/// must be able to see, count and report what the stop condition leaves behind.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "diplomacy payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        country_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        relation_count: u32_at(payload, 20),
    };
    let country_bytes = (header.country_count as usize)
        .checked_mul(COUNTRY_ROW_LEN)
        .ok_or_else(|| format!("diplomacy country_count {} overflows", header.country_count))?;
    let at_war_bytes = (header.relation_count as usize)
        .checked_mul(AT_WAR_ROW_LEN)
        .ok_or_else(|| format!("diplomacy relation_count {} overflows", header.relation_count))?;
    let expected = HEADER_LEN
        .checked_add(country_bytes)
        .and_then(|len| len.checked_add(at_war_bytes))
        .ok_or_else(|| "diplomacy payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("diplomacy payload len {} != {expected}", payload.len()));
    }
    let country_end = HEADER_LEN + country_bytes;
    Ok((
        header,
        &payload[HEADER_LEN..country_end],
        &payload[country_end..],
    ))
}

/// The key of one country row: its `tag_db_id` (`world_fold.rs::fold_diplomacy`).
///
/// The fold does not reject a zero `tag_db_id` here (unlike d18's roster fold at
/// `world_fold.rs::fold_country_identity`) — it folds the row under country key `"0"` — so neither does this.
pub fn key_from_country_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_row_len(row, COUNTRY_ROW_LEN, "country")?;
    Ok(key(&[u32_at(row, 0) as u64]))
}

// --- The at-war table: decoding only, for the stop condition's census --------

/// The at-war row's `tag_db_id` — the *owning* country, `+0`
/// (`world_fold.rs::fold_diplomacy`).
pub fn at_war_owner_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row, AT_WAR_ROW_LEN, "at-war")?;
    Ok(u32_at(row, 0))
}

/// The at-war row's token length prefix, `+4` (`world_fold.rs::fold_diplomacy`).
pub fn enemy_tag_token_len(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row, AT_WAR_ROW_LEN, "at-war")?;
    Ok(u32_at(row, 4))
}

/// The 8-byte enemy token packed little-endian, `+8..+16`. A total bijection
/// onto `u64`, used to compare and count tokens in the census; **not** a wire
/// value — see the module docs' stop condition.
pub fn enemy_tag_word(row: &[u8]) -> core::result::Result<u64, String> {
    check_row_len(row, AT_WAR_ROW_LEN, "at-war")?;
    Ok(u64::from_le_bytes(
        row[ENEMY_TAG_OFFSET..ENEMY_TAG_OFFSET + ENEMY_TAG_BYTES]
            .try_into()
            .expect("8 bytes"),
    ))
}

/// The inverse of [`enemy_tag_word`]: the token field's eight bytes.
pub fn enemy_tag_token(word: u64) -> [u8; ENEMY_TAG_BYTES] {
    word.to_le_bytes()
}

/// The fold's own `ascii8` (`world_fold.rs::ascii8`), reproduced from a
/// packed word and a token length, so a census can report exactly the string
/// `fold_diplomacy` put in `at_war_with`.
pub fn enemy_tag_string(word: u64, token_len: u32) -> String {
    let bytes = enemy_tag_token(word);
    let take = (token_len as usize).min(bytes.len());
    let slice = &bytes[..take];
    let end = slice.iter().position(|b| *b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).to_string()
}

/// Whether a token is one the collector could have written: `token_len == 3`,
/// three `[A-Z0-9]` bytes, and five zero pad bytes
/// (`attach_log.rs:86687`-`:86693`, `:71935`-`:71942`).
///
/// Reported by the measurement harness; nothing enforces it, because nothing
/// stores these rows.
///
/// The character class is an independent restatement of the collector's
/// `compact_contract::country_tag::is_engine_country_tag`, not an import: this
/// crate's compact-authority allow-list (see `Cargo.toml`) admits only d81's
/// rail payload and two status predicates, so a reference decoder may not reach
/// into a collector contract module for it. That module carries the evidence —
/// the engine fixes the length at 3, applies no character-class test of its
/// own, and decides dynamic-ness by database index rather than by spelling.
///
/// This previously also required at least one alphabetic byte. That mirrored a
/// guess the collector used to make, and it went stale the moment the collector
/// dropped it: an all-digit tag would have been counted here as a token the
/// collector could not have produced.
pub fn enemy_tag_is_collector_shaped(word: u64, token_len: u32) -> bool {
    let bytes = enemy_tag_token(word);
    token_len == COLLECTOR_TOKEN_LEN
        && bytes[3..] == [0u8; 5]
        && bytes[..3]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// One compact row of the `countries` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 24-byte row slice (the caller has
/// already consumed the payload header via [`split_payload`]); d17 has exactly
/// one country row form, so length selects nothing and a wrong length is
/// rejected. `war_segment` is the frame's war-segment status, which decides
/// whether a clear gate is a measured zero or an uncaptured value.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d17 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed on anything that would lose data: a gated-off field carrying a
/// non-zero word, or a negative `casualties`, is refused rather than coerced.
/// Nothing in this function can panic on any 24-byte input.
pub fn record_from_country_row(
    collection: &Collection,
    row: &[u8],
    war_segment: WarSegment,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d17 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row, COUNTRY_ROW_LEN, "country")?;
    // The key is not carried in the record body (record-state contract), but its validity is still
    // part of this row's validity.
    let _ = key_from_country_row(row)?;

    // `world_fold.rs::fold_diplomacy` reads the flag word and discards it, so an
    // unrecognised bit is not a rejected row here either — mirroring the fold
    // rather than tightening it. Bits 2..=31 have no writer
    // (`attach_log.rs:69679`-`:69685`) and gate nothing.
    let flags = u32_at(row, 4);
    let has_at_war = flags & FLAG_AT_WAR != 0;
    let has_casualties = flags & FLAG_CASUALTIES != 0;

    let at_war_count = u32_at(row, 12);
    let casualties = i64_at(row, 16);
    // The zero-packing invariant (`attach_log.rs:69689`-`:69698`: both fields
    // are written `unwrap_or(0)` under a clear gate). This is what makes the
    // Unknown mapping lossless — without it a non-zero word under a clear gate
    // would be replaced by Unknown here while the fold still emitted it.
    if !has_at_war && at_war_count != 0 {
        return Err(format!(
            "diplomacy country row {} carries at_war_count {at_war_count} without its flag",
            u32_at(row, 0)
        ));
    }
    if !has_casualties && casualties != 0 {
        return Err(format!(
            "diplomacy country row {} carries casualties {casualties} without its flag",
            u32_at(row, 0)
        ));
    }
    // `Kind::Monotone` is carried as `Value::Unsigned` (schema-kind contract's absolute form), so
    // a negative total has no representation. The collector validates every
    // summand into `0..=WR_MAX_CASUALTIES` before adding it
    // (`attach_log.rs:80200`-`:80205`), so a negative total is not this grammar.
    if casualties < 0 {
        return Err(format!(
            "diplomacy country row {} carries negative casualties {casualties}",
            u32_at(row, 0)
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::FACTION_GROUP => Value::Signed(i32_at(row, 8) as i64),
            field::AT_WAR_COUNT => {
                gated(has_at_war, war_segment, Value::Unsigned(at_war_count as u64))
            }
            field::CASUALTIES => gated(
                has_casualties,
                war_segment,
                Value::Unsigned(casualties as u64),
            ),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!("diplomacy countries has no mapping for field id {other}"));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// A gate that is clear means the war-relations segment either found no relation
/// for this country (a measured zero) or did not capture at all (record-state contract's unknown).
/// The frame's status word is the only thing that can tell them apart — see the
/// module docs.
fn gated(present: bool, war_segment: WarSegment, value: Value) -> Value {
    if present {
        value
    } else {
        match war_segment {
            WarSegment::Captured => Value::Unsigned(0),
            WarSegment::Absent => Value::Unknown,
        }
    }
}

// --- The shipping accumulator ------------------------------------------------

/// What one d17 DATA record did to the accumulator, and what it left unstored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FoldReport {
    pub country_rows: usize,
    /// At-war rows the frame carried. **None of them are stored** — the war
    /// graph is a reported stop condition (see the module docs) — so this is the
    /// count a caller must surface rather than let pass silently.
    pub at_war_rows_not_stored: usize,
    /// Of those, rows naming a country the payload does not carry. The fold
    /// drops them silently (`world_fold.rs::fold_diplomacy`), so they would not have been
    /// stored even without the stop condition; counted separately so the stop's
    /// real cost is not overstated.
    pub orphan_at_war_rows: usize,
    /// Of those, rows whose token is not one `is_country_tag` +
    /// `append_fixed_ascii_token8` could have produced.
    pub non_collector_shaped_tokens: usize,
    /// Country rows whose `at_war_count` disagrees with the number of at-war
    /// rows the same frame carries for them, under a set `FLAG_AT_WAR`. The fold
    /// checks nothing here, so neither does this; the count is a report, and it
    /// is the check that would catch `at_war_count` drifting away from the
    /// unstored table it counts.
    pub at_war_count_mismatches: usize,
    /// The frame's war segment did not capture, so nothing this frame says about
    /// wars is a measurement of this hour. The field-level half of that is
    /// encoded (`at_war_count`/`casualties` are `Unknown`); the collection-level
    /// half has no wire representation (gap contract has no code here) and is reported.
    pub at_war_hour_gap: bool,
}

/// The running d17 state, replaying `fold_diplomacy`'s accumulator grammar.
///
/// This is the piece that makes d17 different from a full-replace domain, so it
/// lives beside the schema rather than in a harness: the measurement bin, the
/// tests and any future caller share one implementation of upsert-on-present and
/// never-delete-a-country.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `countries` collection's current content, keyed by `tag_db_id`.
    pub countries: KeyedState,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one d17 DATA payload, exactly as `fold_diplomacy` does from
    /// `world_fold.rs::fold_diplomacy` onward, for the part of the payload this schema
    /// stores; the at-war table is censused into the report and not stored (see
    /// the module docs' stop condition).
    ///
    /// A failure-status record is folded here too, and the caller accounts for
    /// it separately. `world_fold.rs::fold_diplomacy` applies and accounts for
    /// degraded rows (`world_fold.rs::record_degraded_rows_applied`), because
    /// the rows a partly-failed read produced are real and throwing them away
    /// loses measured state. A caller that mirrors the fold must therefore fold
    /// the rows AND record the failure; a caller that only wants clean frames
    /// must filter them out itself, since this function no longer refuses them.
    ///
    /// `war_segment` is the caller's reading of
    /// `COMPACT_STATUS_WAR_SEGMENT_ABSENT`; that bit is informational and is
    /// **not** part of the failure mask (`world_fold.rs::fold_diplomacy`), so a frame carrying
    /// it is data, is folded, and is NOT a degradation.
    ///
    /// On `Err` the accumulator is left as it was, so a caller that reports the
    /// error and stops is not left holding a half-folded frame: every row is
    /// mapped before anything is mutated, exactly as the fold's own errors all
    /// precede its first mutation (`world_fold.rs::fold_diplomacy` is the last of them).
    pub fn fold_payload(
        &mut self,
        countries_collection: &Collection,
        payload: &[u8],
        war_segment: WarSegment,
    ) -> core::result::Result<FoldReport, String> {
        let (header, country_rows, at_war_rows) = split_payload(payload)?;
        let _ = header;

        let mut new_countries: Vec<(Key, Record)> =
            Vec::with_capacity(country_rows.len() / COUNTRY_ROW_LEN);
        for row in country_rows.chunks_exact(COUNTRY_ROW_LEN) {
            new_countries.push((
                key_from_country_row(row)?,
                record_from_country_row(countries_collection, row, war_segment, 0)?,
            ));
        }
        // `world_fold.rs::fold_diplomacy`: the country table is built into a map
        // first, so a tag repeated inside one frame is last-writer-wins rather
        // than a rejected frame, and the at-war rows attach to the survivor.
        let mut present: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for row in country_rows.chunks_exact(COUNTRY_ROW_LEN) {
            present.insert(u32_at(row, 0));
        }

        let mut report = FoldReport {
            country_rows: new_countries.len(),
            at_war_rows_not_stored: at_war_rows.len() / AT_WAR_ROW_LEN,
            at_war_hour_gap: war_segment == WarSegment::Absent,
            ..FoldReport::default()
        };

        // The census of the table this schema stops on. Every row is decoded, so
        // a malformed one is still an error and none is passed over in silence.
        let mut counted: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
        for row in at_war_rows.chunks_exact(AT_WAR_ROW_LEN) {
            let owner = at_war_owner_of_row(row)?;
            let word = enemy_tag_word(row)?;
            let token_len = enemy_tag_token_len(row)?;
            if !enemy_tag_is_collector_shaped(word, token_len) {
                report.non_collector_shaped_tokens += 1;
            }
            // `world_fold.rs::fold_diplomacy` — a row naming a tag the country table does
            // not carry falls on the floor even in the fold.
            if !present.contains(&owner) {
                report.orphan_at_war_rows += 1;
                continue;
            }
            *counted.entry(owner).or_default() += 1;
        }

        // --- Nothing above this line has mutated the accumulator. ------------

        for (k, record) in new_countries {
            // A whole-row upsert: `world_fold.rs::fold_diplomacy` replaces the country's
            // diplomacy object outright and removes no other country.
            self.countries.insert(k, record);
        }

        // Report-only: the writer sets `at_war_count` to the length of the same
        // list it then serializes (`attach_log.rs:69690`-`:69706`), but the fold
        // asserts nothing, so neither does this.
        for row in country_rows.chunks_exact(COUNTRY_ROW_LEN) {
            let flags = u32_at(row, 4);
            if flags & FLAG_AT_WAR != 0 {
                let owner = u32_at(row, 0);
                if u32_at(row, 12) as usize != counted.get(&owner).copied().unwrap_or(0) {
                    report.at_war_count_mismatches += 1;
                }
            }
        }
        Ok(report)
    }
}

fn check_row_len(row: &[u8], expected: usize, what: &str) -> core::result::Result<(), String> {
    if row.len() != expected {
        return Err(format!(
            "diplomacy {what} row length {} != {expected}",
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

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
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

    fn countries() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the collection")
    }

    /// A raw 24-byte country row, built the way the collector packs one
    /// (`attach_log.rs:69686`-`:69698`).
    fn country_row(tag: u32, flags: u32, faction_group: i32, at_war_count: u32, casualties: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(COUNTRY_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&faction_group.to_le_bytes());
        row.extend_from_slice(&at_war_count.to_le_bytes());
        row.extend_from_slice(&casualties.to_le_bytes());
        assert_eq!(row.len(), COUNTRY_ROW_LEN);
        row
    }

    /// A raw 16-byte at-war row, built the way `append_fixed_ascii_token8` packs
    /// one (`attach_log.rs:71935`-`:71942`).
    fn at_war_row(tag: u32, enemy: &str) -> Vec<u8> {
        let bytes = enemy.as_bytes();
        let len = bytes.len().min(ENEMY_TAG_BYTES);
        let mut fixed = [0u8; ENEMY_TAG_BYTES];
        fixed[..len].copy_from_slice(&bytes[..len]);
        raw_at_war_row(tag, len as u32, fixed)
    }

    /// An at-war row with an arbitrary length prefix and token, for the cases the
    /// collector cannot produce but a file on disk can carry.
    fn raw_at_war_row(tag: u32, token_len: u32, token: [u8; ENEMY_TAG_BYTES]) -> Vec<u8> {
        let mut row = Vec::with_capacity(AT_WAR_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&token_len.to_le_bytes());
        row.extend_from_slice(&token);
        assert_eq!(row.len(), AT_WAR_ROW_LEN);
        row
    }

    fn payload(status_flags: u32, countries: &[Vec<u8>], at_war: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        p.extend_from_slice(&(countries.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(at_war.len() as u32).to_le_bytes());
        for row in countries {
            p.extend_from_slice(row);
        }
        for row in at_war {
            p.extend_from_slice(row);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema().validate(grid().log2()).expect("d17 schema is well-formed");
    }

    #[test]
    fn the_war_graph_collection_is_stopped_not_encoded() {
        // identity is an 8-byte ASCII country TAG. schema-kind contract has no string kind and content-identity contract
        // requires a manifest symbol id, "never text", while this mapper has not
        // yet been retrofitted with the file-global content-identity contract resolver. Packing the token into a
        // numeric word — under `ContentId` (which would claim it IS a symbol id)
        // or under `Unsigned` (which would still ship embedded text) — is not
        // conformant either way, so no collection is declared and the rows are
        // censused instead. What survives is the fact, not the identities:
        // `at_war_count` carries the cardinality exactly.
        let s = schema();
        assert_eq!(s.collections.len(), 1, "only the country table is ported");
        assert_eq!(s.collections[0].collection_id, COUNTRIES);
        assert!(
            s.collections
                .iter()
                .all(|c| c.collection_id != AT_WAR_RESERVED),
            "the war-graph collection id is reserved, never declared"
        );
        let ids: Vec<u32> = s.collections[0].fields.iter().map(|f| f.field_id).collect();
        for reserved in [field::ENEMY_TAG_RESERVED, field::ENEMY_TAG_TOKEN_LEN_RESERVED] {
            assert!(!ids.contains(&reserved), "field id {reserved} stays reserved");
        }
        assert!(
            ids.contains(&field::AT_WAR_COUNT),
            "the cardinality the stopped table would have carried is still stored"
        );
    }

    #[test]
    fn the_country_collection_is_a_keyed_set_keyed_by_the_country() {
        let c = countries();
        assert_eq!(c.shape, Shape::KeyedSet, "an accumulator of stable country identities");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(keys[0].kind, Kind::ContentId);
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 4, "1 key + faction_group + at_war_count + casualties");
    }

    #[test]
    fn exactly_two_fields_are_unknown_capable_and_nothing_is_optional_or_nullable() {
        // The schema declares exactly one empty state, on exactly the two fields
        // the war segment can fail to capture. Absent would claim the concept
        // does not apply to a country; Null would claim we know there is nothing.
        let c = countries();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        let unknown: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(unknown, vec![field::AT_WAR_COUNT, field::CASUALTIES]);
    }

    #[test]
    fn no_field_is_quantized_because_no_field_is_continuous() {
        // Step 4's "quantization saves bytes" test does not apply: d17 has no
        // continuous field at all. Every field the fold emits is an integer word
        // folded raw (`world_fold.rs::fold_diplomacy`) — there is no fixed-point
        // scale anywhere in this domain, so there is nothing to quantize.
        for f in countries().value_fields() {
            assert!(!matches!(f.kind, Kind::Continuous), "field {}", f.field_id);
            assert_eq!(f.quant, Quant::None, "field {}", f.field_id);
        }
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d17 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(countries().fields.iter().all(|f| f.derived.is_none()));
    }

    // --- Field offsets, one field at a time ---------------------------------

    #[test]
    fn every_country_field_reads_its_own_offset() {
        // The transposition test: each field is given a distinct value, so a
        // swapped pair fails. There is no scale in this domain, so this is also
        // the "no field is divided" test.
        let c = countries();
        let row = country_row(7, FLAG_AT_WAR | FLAG_CASUALTIES, -12345, 9, 1_234_567_890_123);
        let r = record_from_country_row(&c, &row, WarSegment::Captured, 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(-12345));
        assert_eq!(r.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unsigned(9));
        assert_eq!(
            r.values[index_of(&c, field::CASUALTIES)],
            Value::Unsigned(1_234_567_890_123),
            "casualties is a raw i64 count, never divided by any scale"
        );
        // The key is the tag alone, from `+0`.
        assert_eq!(key_from_country_row(&row).expect("keys"), key(&[7]));
        // And `faction_group` is a full i32, both signs and both extremes.
        for fg in [i32::MIN, -1, 0, 1, i32::MAX] {
            let row = country_row(1, 0, fg, 0, 0);
            let r = record_from_country_row(&c, &row, WarSegment::Captured, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::FACTION_GROUP)],
                Value::Signed(fg as i64),
                "faction_group {fg}"
            );
        }
    }

    #[test]
    fn every_at_war_field_is_decodable_at_its_own_offset() {
        // Nothing stores these rows, but the census reads all three fields and a
        // transposition would corrupt what it reports.
        let row = at_war_row(0xAABB_CCDD, "GER");
        assert_eq!(at_war_owner_of_row(&row).expect("owner"), 0xAABB_CCDD);
        assert_eq!(enemy_tag_token_len(&row).expect("len"), 3);
        let expected_word = u64::from_le_bytes([b'G', b'E', b'R', 0, 0, 0, 0, 0]);
        assert_eq!(enemy_tag_word(&row).expect("word"), expected_word);
        assert_eq!(enemy_tag_string(expected_word, 3), "GER");
        for len in [0usize, 1, AT_WAR_ROW_LEN - 1, AT_WAR_ROW_LEN + 1] {
            let short = vec![0u8; len];
            assert!(at_war_owner_of_row(&short).is_err(), "len {len}");
            assert!(enemy_tag_token_len(&short).is_err(), "len {len}");
            assert!(enemy_tag_word(&short).is_err(), "len {len}");
        }
    }

    #[test]
    fn the_census_reproduces_the_folds_own_string() {
        // `ascii8` trims to `min(token_len, 8)` BEFORE trimming to the first NUL
        // (`world_fold.rs::ascii8`). The census must reproduce that
        // exactly or its report is about a different string than the fold's.
        let token = [b'G', b'E', b'R', b'M', b'A', b'N', b'Y', b'!'];
        assert_eq!(enemy_tag_string(u64::from_le_bytes(token), 3), "GER");
        assert_eq!(enemy_tag_string(u64::from_le_bytes(token), 8), "GERMANY!");
        assert_eq!(enemy_tag_string(u64::from_le_bytes(token), 0), "");
        assert_eq!(enemy_tag_string(u64::from_le_bytes(token), u32::MAX), "GERMANY!");
        let padded = [b'G', b'E', b'R', 0, 0, 0, 0, 0];
        assert_eq!(enemy_tag_string(u64::from_le_bytes(padded), 8), "GER");
        // And the collector-shape predicate is exactly `is_country_tag` +
        // `append_fixed_ascii_token8`.
        assert!(enemy_tag_is_collector_shaped(u64::from_le_bytes(padded), 3));
        assert!(!enemy_tag_is_collector_shaped(u64::from_le_bytes(token), 8), "8 chars");
        assert!(!enemy_tag_is_collector_shaped(u64::from_le_bytes(padded), 8), "wrong len");
        // A dynamic country tag is an ordinary tag the collector writes like any
        // other, so counting one as non-collector-shaped would misreport the
        // census. Goes red if the alphabetic-byte requirement comes back.
        assert!(
            enemy_tag_is_collector_shaped(u64::from_le_bytes([b'D', b'0', b'4', 0, 0, 0, 0, 0]), 3),
            "D04 is a dynamic country tag, not a malformed token"
        );
        assert!(
            enemy_tag_is_collector_shaped(u64::from_le_bytes([b'9', b'9', b'9', 0, 0, 0, 0, 0]), 3),
            "the engine applies no character-class rule, so an all-digit tag is admissible"
        );
        assert!(
            !enemy_tag_is_collector_shaped(u64::from_le_bytes([b'g', b'e', b'r', 0, 0, 0, 0, 0]), 3),
            "lowercase is not a country tag"
        );
        // The packing is a total bijection, so the census can compare tokens
        // without a decode step that could fail.
        for position in 0..ENEMY_TAG_BYTES {
            for byte in [0u8, 1, 0x41, 0x7F, 0x80, 0xFE, 0xFF] {
                let mut t = [0u8; ENEMY_TAG_BYTES];
                t[position] = byte;
                let row = raw_at_war_row(1, 8, t);
                assert_eq!(enemy_tag_token(enemy_tag_word(&row).expect("word")), t);
            }
        }
    }

    // --- The four states, through the mapper, from raw bytes ----------------

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Every flag set, every value zero: the row for a country in faction
        // group 0 that is at war with nobody and has taken no casualties. record-state contract
        // keeps zero out of the three empty states.
        let c = countries();
        let row = country_row(1, FLAG_AT_WAR | FLAG_CASUALTIES, 0, 0, 0);
        for segment in [WarSegment::Captured, WarSegment::Absent] {
            let r = record_from_country_row(&c, &row, segment, 0).expect("maps");
            assert_eq!(r.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(0));
            assert_eq!(r.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unsigned(0));
            assert_eq!(r.values[index_of(&c, field::CASUALTIES)], Value::Unsigned(0));
        }
    }

    #[test]
    fn a_clear_gate_is_a_measured_zero_when_the_war_segment_captured() {
        // `apply_war_relations_to_country_rows` fills a country's entry only if
        // it appears in a relation (`attach_log.rs:80277`-`:80288`). With the
        // segment applied, a clear gate means it appears in none — a measured
        // "no wars", not an empty state.
        let c = countries();
        for (flags, gated_ids) in [
            (FLAG_CASUALTIES, vec![field::AT_WAR_COUNT]),
            (FLAG_AT_WAR, vec![field::CASUALTIES]),
            (0u32, vec![field::AT_WAR_COUNT, field::CASUALTIES]),
        ] {
            let row = country_row(
                3,
                flags,
                5,
                if flags & FLAG_AT_WAR != 0 { 2 } else { 0 },
                if flags & FLAG_CASUALTIES != 0 { 77 } else { 0 },
            );
            let r = record_from_country_row(&c, &row, WarSegment::Captured, 0).expect("maps");
            for id in [field::AT_WAR_COUNT, field::CASUALTIES] {
                let expected = if gated_ids.contains(&id) {
                    Value::Unsigned(0)
                } else if id == field::AT_WAR_COUNT {
                    Value::Unsigned(2)
                } else {
                    Value::Unsigned(77)
                };
                assert_eq!(r.values[index_of(&c, id)], expected, "field {id} flags {flags:#x}");
            }
        }
    }

    #[test]
    fn a_clear_gate_is_unknown_when_the_war_segment_did_not_capture() {
        // The whole gate space, one gate at a time, under an absent segment.
        let c = countries();
        for (flags, unknown_ids) in [
            (FLAG_CASUALTIES, vec![field::AT_WAR_COUNT]),
            (FLAG_AT_WAR, vec![field::CASUALTIES]),
            (0u32, vec![field::AT_WAR_COUNT, field::CASUALTIES]),
            (FLAG_AT_WAR | FLAG_CASUALTIES, vec![]),
        ] {
            let row = country_row(
                3,
                flags,
                5,
                if flags & FLAG_AT_WAR != 0 { 2 } else { 0 },
                if flags & FLAG_CASUALTIES != 0 { 77 } else { 0 },
            );
            let r = record_from_country_row(&c, &row, WarSegment::Absent, 0).expect("maps");
            for id in [field::AT_WAR_COUNT, field::CASUALTIES] {
                assert_eq!(
                    r.values[index_of(&c, id)] == Value::Unknown,
                    unknown_ids.contains(&id),
                    "field {id} under flags {flags:#x}"
                );
            }
            // `faction_group` is never gated: it is read before the war segment
            // is even consulted (`attach_log.rs:80137`).
            assert_eq!(r.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(5));
        }
    }

    #[test]
    fn a_set_gate_carries_its_value_whatever_the_segment_status() {
        // Mirror, do not tighten: if the flag says the value was captured, the
        // value is data even in a frame whose segment status says otherwise.
        let c = countries();
        let row = country_row(3, FLAG_AT_WAR | FLAG_CASUALTIES, 1, 4, 500);
        let r = record_from_country_row(&c, &row, WarSegment::Absent, 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unsigned(4));
        assert_eq!(r.values[index_of(&c, field::CASUALTIES)], Value::Unsigned(500));
    }

    #[test]
    fn the_unknown_state_survives_a_roundtrip() {
        let c = countries();
        let record =
            record_from_country_row(&c, &country_row(3, 0, -7, 0, 0), WarSegment::Absent, 0)
                .expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(-7));
        assert_eq!(back.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unknown);
        assert_eq!(back.values[index_of(&c, field::CASUALTIES)], Value::Unknown);
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = countries();
        for record in [
            record_from_country_row(
                &c,
                &country_row(7, FLAG_AT_WAR | FLAG_CASUALTIES, -3, 5, 900_000),
                WarSegment::Captured,
                0,
            )
            .expect("maps"),
            // And the fully-unknown row, whose bitmaps are the interesting case.
            record_from_country_row(&c, &country_row(7, 0, 0, 0, 0), WarSegment::Absent, 0)
                .expect("maps"),
        ] {
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

    // --- What the mapper refuses --------------------------------------------

    #[test]
    fn the_mapper_refuses_rows_it_cannot_carry_losslessly() {
        let c = countries();
        for len in [0usize, 1, COUNTRY_ROW_LEN - 1, COUNTRY_ROW_LEN + 1, 48] {
            assert!(
                record_from_country_row(&c, &vec![0u8; len], WarSegment::Captured, 0).is_err(),
                "country length {len} must be rejected, not read"
            );
        }
        // A gated-off field carrying a value: refusing keeps the Unknown mapping
        // lossless. Both fields, both segment statuses.
        for segment in [WarSegment::Captured, WarSegment::Absent] {
            assert!(
                record_from_country_row(&c, &country_row(1, FLAG_CASUALTIES, 0, 1, 0), segment, 0)
                    .is_err(),
                "at_war_count without its flag"
            );
            assert!(
                record_from_country_row(&c, &country_row(1, FLAG_AT_WAR, 0, 0, 1), segment, 0)
                    .is_err(),
                "casualties without its flag"
            );
        }
        // A negative casualties total has no monotone representation.
        for casualties in [-1i64, i64::MIN] {
            assert!(
                record_from_country_row(
                    &c,
                    &country_row(1, FLAG_AT_WAR | FLAG_CASUALTIES, 0, 0, casualties),
                    WarSegment::Captured,
                    0
                )
                .is_err(),
                "casualties {casualties}"
            );
        }
        // The largest representable total is accepted, so the bound is not a
        // range check in disguise.
        assert!(record_from_country_row(
            &c,
            &country_row(1, FLAG_AT_WAR | FLAG_CASUALTIES, 0, u32::MAX, i64::MAX),
            WarSegment::Captured,
            0
        )
        .is_ok());
    }

    #[test]
    fn an_unrecognised_flag_bit_is_ignored_exactly_as_the_fold_ignores_it() {
        // `world_fold.rs::fold_diplomacy` reads the flag word into `_flags` and drops it.
        // Rejecting an unknown bit would refuse a row the fold accepts.
        let c = countries();
        for bit in 2..32u32 {
            let flags = FLAG_AT_WAR | FLAG_CASUALTIES | (1 << bit);
            let r = record_from_country_row(&c, &country_row(1, flags, 2, 3, 4), WarSegment::Captured, 0)
                .unwrap_or_else(|e| panic!("bit {bit} must be ignored, not refused: {e}"));
            assert_eq!(r.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unsigned(3));
            assert_eq!(r.values[index_of(&c, field::CASUALTIES)], Value::Unsigned(4));
        }
    }

    #[test]
    fn a_zero_tag_db_id_is_folded_because_the_fold_folds_it() {
        // Unlike d18's roster (`world_fold.rs:13940`), `fold_diplomacy` makes no
        // zero check — it folds the row under country key "0". Mirrored.
        let c = countries();
        assert!(record_from_country_row(&c, &country_row(0, 0, 0, 0, 0), WarSegment::Captured, 0).is_ok());
        assert_eq!(key_from_country_row(&country_row(0, 0, 0, 0, 0)).expect("keys"), key(&[0]));
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = countries();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_country_row(&c, &country_row(1, 0, 0, 0, 0), WarSegment::Captured, 0).is_err());
    }

    // --- split_payload -------------------------------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let c_rows = vec![country_row(1, FLAG_AT_WAR, 0, 1, 0), country_row(2, 0, 0, 0, 0)];
        let w_rows = vec![at_war_row(1, "GER")];
        let p = payload(0, &c_rows, &w_rows);
        let (header, countries_bytes, at_war_bytes) = split_payload(&p).expect("well-formed");
        assert_eq!(header.country_count, 2);
        assert_eq!(header.relation_count, 1);
        assert_eq!(countries_bytes.len(), 2 * COUNTRY_ROW_LEN);
        assert_eq!(at_war_bytes.len(), AT_WAR_ROW_LEN);

        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // Any length that is not exactly the two tables (`:13902`'s trailing
        // check plus the per-row `need`s).
        for len in HEADER_LEN..p.len() {
            assert!(split_payload(&p[..len]).is_err(), "truncated to {len}");
        }
        let mut trailing = p.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err(), "trailing bytes");

        // Counts whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        for offset in [8usize, 20] {
            let mut huge = p.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&huge).is_err(), "count at +{offset}");
        }
        // An empty payload of exactly the header is legal and is what 901 of the
        // capture's 903 hours carry.
        let empty = payload(0, &[], &[]);
        let (h, c, w) = split_payload(&empty).expect("a rowless frame is well-formed");
        assert_eq!((h.country_count, h.relation_count), (0, 0));
        assert!(c.is_empty() && w.is_empty());
    }

    #[test]
    fn changed_rows_is_parsed_and_never_enforced() {
        // The fold reads `+8` and `+20` and nothing else (`world_fold.rs:13871`-
        // `:13872`), so d17 states no `changed_rows` invariant. Asserting one an
        // authority does not state is the recorded d10 defect.
        let rows = vec![country_row(1, 0, 0, 0, 0)];
        let mut p = payload(0, &rows, &[]);
        for changed in [0u32, 1, 7, u32::MAX] {
            p[12..16].copy_from_slice(&changed.to_le_bytes());
            let (header, _, _) = split_payload(&p).expect("changed_rows never gates the split");
            assert_eq!(header.changed_rows, changed);
        }
    }

    // --- The accumulator -----------------------------------------------------

    #[test]
    fn a_country_row_upserts_and_no_frame_ever_deletes_one() {
        // `world_fold.rs::fold_diplomacy` writes one country object per tag in
        // the payload and touches nothing else, so a later rowless frame is a
        // no-op rather than a mass deletion — which is what 901 of the capture's
        // 903 hours are.
        let c = countries();
        let mut acc = Accumulator::new();
        let report = acc
            .fold_payload(&c, &payload(0, &[country_row(1, 0, 4, 0, 0)], &[]), WarSegment::Captured)
            .expect("folds");
        assert_eq!(report.country_rows, 1);
        assert_eq!(acc.countries.len(), 1);

        acc.fold_payload(&c, &payload(0, &[], &[]), WarSegment::Captured)
            .expect("a rowless frame folds");
        assert_eq!(acc.countries.len(), 1, "a rowless frame deletes nothing");

        // And a second row for the same tag replaces the whole record.
        acc.fold_payload(&c, &payload(0, &[country_row(1, 0, 9, 0, 0)], &[]), WarSegment::Captured)
            .expect("folds");
        assert_eq!(acc.countries.len(), 1);
        let record = acc.countries.values().next().expect("one row");
        assert_eq!(record.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(9));
    }

    #[test]
    fn a_tag_repeated_in_one_frame_is_last_writer_wins() {
        // `world_fold.rs::fold_diplomacy`'s `per_tag.insert` overwrites, so a repeat is not
        // a rejected frame.
        let c = countries();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(0, &[country_row(1, 0, 4, 0, 0), country_row(1, 0, 6, 0, 0)], &[]),
            WarSegment::Captured,
        )
        .expect("folds");
        assert_eq!(acc.countries.len(), 1);
        assert_eq!(
            acc.countries.values().next().expect("one row").values[index_of(&c, field::FACTION_GROUP)],
            Value::Signed(6)
        );
    }

    #[test]
    fn every_unstored_at_war_row_is_counted_never_passed_over() {
        // The stop condition must be visible in the numbers: rows carried, rows
        // the fold itself would drop (`world_fold.rs::fold_diplomacy`), and tokens outside
        // the collector's own shape.
        let c = countries();
        let mut acc = Accumulator::new();
        let report = acc
            .fold_payload(
                &c,
                &payload(
                    0,
                    &[country_row(1, FLAG_AT_WAR, 0, 2, 0)],
                    &[
                        at_war_row(1, "GER"),
                        at_war_row(1, "ENG"),
                        at_war_row(99, "SOV"),
                        raw_at_war_row(1, 8, *b"GERMANY!"),
                    ],
                ),
                WarSegment::Captured,
            )
            .expect("folds");
        assert_eq!(report.at_war_rows_not_stored, 4);
        assert_eq!(report.orphan_at_war_rows, 1, "tag 99 is not in the country table");
        assert_eq!(report.non_collector_shaped_tokens, 1);
        // The fact survives the stop: the country row's own count is stored.
        assert_eq!(
            acc.countries.values().next().expect("one row").values[index_of(&c, field::AT_WAR_COUNT)],
            Value::Unsigned(2)
        );
    }

    #[test]
    fn the_at_war_count_mismatch_is_reported_and_never_enforced() {
        // The fold checks nothing here, so a disagreeing count is data, not a
        // rejected frame — and with the war graph stopped, this report is the
        // only cross-check left between the count and the table it counts.
        let c = countries();
        let mut acc = Accumulator::new();
        let report = acc
            .fold_payload(
                &c,
                &payload(0, &[country_row(1, FLAG_AT_WAR, 0, 5, 0)], &[at_war_row(1, "GER")]),
                WarSegment::Captured,
            )
            .expect("folds");
        assert_eq!(report.at_war_count_mismatches, 1);
        assert_eq!(
            acc.countries.values().next().expect("one row").values[index_of(&c, field::AT_WAR_COUNT)],
            Value::Unsigned(5),
            "the row's own word is what the fold emits"
        );
        // Orphan rows do not count toward the owner's tally, exactly as the fold
        // does not fold them.
        let mut acc = Accumulator::new();
        let report = acc
            .fold_payload(
                &c,
                &payload(0, &[country_row(1, FLAG_AT_WAR, 0, 1, 0)], &[at_war_row(1, "GER")]),
                WarSegment::Captured,
            )
            .expect("folds");
        assert_eq!(report.at_war_count_mismatches, 0);
    }

    #[test]
    fn a_segment_absent_frame_is_folded_and_its_hour_reported() {
        // Bit 13 is informational and outside the failure mask
        // (`world_fold.rs::fold_diplomacy`), so its frames are data and are
        // folded. What cannot be expressed is that this hour measured nothing
        // about wars; the field-level half of that IS expressed.
        let c = countries();
        let mut acc = Accumulator::new();
        let report = acc
            .fold_payload(&c, &payload(0, &[country_row(1, 0, 4, 0, 0)], &[]), WarSegment::Absent)
            .expect("folds");
        assert!(report.at_war_hour_gap);
        assert_eq!(acc.countries.len(), 1, "the country row is still data");
        let record = acc.countries.values().next().expect("one row");
        assert_eq!(record.values[index_of(&c, field::FACTION_GROUP)], Value::Signed(4));
        assert_eq!(record.values[index_of(&c, field::AT_WAR_COUNT)], Value::Unknown);
        assert_eq!(record.values[index_of(&c, field::CASUALTIES)], Value::Unknown);
        // And a captured frame does not report a gap.
        let report = acc
            .fold_payload(&c, &payload(0, &[country_row(1, 0, 4, 0, 0)], &[]), WarSegment::Captured)
            .expect("folds");
        assert!(!report.at_war_hour_gap);
    }

    #[test]
    fn a_refused_frame_leaves_the_accumulator_exactly_as_it_was() {
        let c = countries();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[country_row(1, 0, 4, 0, 0)], &[]), WarSegment::Captured)
            .expect("folds");
        let before = acc.countries.clone();
        // A structurally sized frame whose second row is not carryable.
        let bad = payload(
            0,
            &[country_row(2, 0, 1, 0, 0), country_row(3, FLAG_AT_WAR, 0, 0, 9)],
            &[],
        );
        assert!(acc.fold_payload(&c, &bad, WarSegment::Captured).is_err());
        assert_eq!(acc.countries, before, "no half-folded frame");
        // And an at-war row of the wrong shape cannot be reached: the split
        // rejects the payload before any row is read.
        let mut short = payload(0, &[country_row(2, 0, 1, 0, 0)], &[at_war_row(2, "GER")]);
        short.truncate(short.len() - 1);
        assert!(acc.fold_payload(&c, &short, WarSegment::Captured).is_err());
        assert_eq!(acc.countries, before);
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the accumulator is fed from a file on disk.
        let c = countries();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..3_000 {
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
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &bytes, WarSegment::Captured);
            let _ = acc.fold_payload(&c, &bytes, WarSegment::Absent);
        }
    }

    // --- The grammar end to end ---------------------------------------------

    #[test]
    fn the_collection_survives_a_baseline_and_a_delta_replay() {
        // The reader replay the measurement makes per hour, in miniature: an
        // insert, an update, and an unknown that stays unknown.
        let schema = schema();
        let c = countries();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    country_row(1, FLAG_AT_WAR | FLAG_CASUALTIES, 3, 1, 100),
                    country_row(2, 0, 4, 0, 0),
                ],
                &[at_war_row(1, "GER")],
            ),
            WarSegment::Absent,
        )
        .expect("folds");
        let first = vec![acc.countries.clone()];

        let mut bytes = Vec::new();
        encode_baseline(&schema, &first, &mut bytes).expect("baseline encodes");
        let (decoded, consumed) = decode_baseline(&schema, &bytes, 0).expect("baseline decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, first);

        // Hour two: country 1 gains casualties; country 2 is untouched and must
        // not be re-emitted.
        acc.fold_payload(
            &c,
            &payload(0, &[country_row(1, FLAG_AT_WAR | FLAG_CASUALTIES, 3, 0, 250)], &[]),
            WarSegment::Captured,
        )
        .expect("folds");
        let second = vec![acc.countries.clone()];

        let deltas: Vec<_> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                let d = diff(collection, &first[i], &second[i]);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        let delta = deltas[0].as_ref().expect("country 1 changed");
        assert_eq!(delta.changed.len(), 1, "only country 1");
        assert!(delta.tombstones.is_empty(), "nothing is ever deleted");

        let mut delta_bytes = Vec::new();
        encode_delta(&schema, &first, &deltas, &mut delta_bytes).expect("delta encodes");
        let mut reader = decoded;
        let (read_deltas, consumed) =
            decode_delta(&schema, &reader, &delta_bytes, 0).expect("delta decodes");
        assert_eq!(consumed, delta_bytes.len());
        for (i, d) in read_deltas.iter().enumerate() {
            if let Some(d) = d {
                apply(&mut reader[i], d);
            }
        }
        assert_eq!(reader, second, "the reader reconstructs the fold's state");
    }
}
