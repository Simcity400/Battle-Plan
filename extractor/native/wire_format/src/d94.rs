//! d94 `pov_province_visibility_live` — the observing country's fog-of-war level
//! for every province, as one complete engine byte vector per hour.
//!
//! Derived from `world_fold.rs:14462`-`:14523`
//! (`fold_pov_province_visibility`, dispatched at `:4745`, its error arm at
// @world-fold-authority domain=94 functions=fold_pov_province_visibility lines=14462-14523 sha256=f4f2a1bf6b68acc138462a5344beaf993617b18b124b4fc745192514c3c29f90 closure=75584555e480cbc103bc38dc1dbb6282fd8b64988e1cb082a8f294f2a5fb3ef0
//! `:4917`-`:4925`, its hour reset at `world_fold.rs::reset_hour_state`) and from the payload codec
//! the domain already owns, `compact_contract/src/pov_visibility.rs` (recipe
//! authority 2). There is no `extractor/documents/domains/d94_*.md`, which is
//! expected for a batch-3 domain.
//!
//! **Payload grammar** (`pov_visibility.rs:32`-`:69`): a 24-byte header, then
//! `count` raw bytes.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | `[u8;4]` | magic `FOW1`      | — | rejected unless exact (`:48`) |
//! | `+4`  | u16 | `version`              | — | must be `1` (`:53`) |
//! | `+6`  | u16 | `flags`                | — | only `FLAG_PRODUCER_HOUR` defined; any other bit rejected (`:53`) |
//! | `+8`  | u64 | `capture_sequence`     | raw | must be non-zero (`:20`) |
//! | `+16` | u32 | `pov_country_id`       | raw | `1..=MAX_POV_COUNTRY_ID` (`:23`) |
//! | `+20` | u32 | `count`                | raw | `1..=65535`, and `len == 24 + count + 8·hour` (`:26`, `:59`) |
//! | `+24` | u8 × `count` | `province_levels` | **raw, no scale** | none |
//! | `+24+count` | i64 | `producer_hour_raw` | raw engine date | present iff `FLAG_PRODUCER_HOUR`; must fit `i32` |
//!
//! **THE HEADLINE: the index is the key.** `province_levels` is the complete
//! engine byte vector and *"Index is the province database ID"*
//! (`pov_visibility.rs:14`). The position is therefore a `ContentId` — a province
//! id a content manifest must resolve — not an ordinal the boundary generates.
//! Recipe step 5's ordinal device does not apply and is not needed: the keyed set
//! is `(pov_country_id, province_id) -> level` directly, and the position
//! persists across hours as the same thing in the strongest possible sense
//! (province 1234 is province 1234 in every hour of every campaign under this mod
//! set). A `Vec` whose index carries content identity is a map, not a list.
//!
//! **POV is a FRAME property, carried in the KEY.** One record describes one
//! observing country; `pov_country_id` is a header word, not a row column, and it
//! must not drift into one. It is a key component rather than a bare frame input
//! because the fold's accumulator holds exactly one snapshot per hour
//! (`world_fold.rs:1838`, cleared at `world_fold.rs::reset_hour_state`): if the POV country ever changed,
//! the recorder's next hour would be that country's vector *alone*, and a keyed
//! set whose keys carry the POV reproduces that as 13,414 tombstones plus 13,414
//! inserts — losslessly. A frame input would instead let one POV's levels be
//! read as another's. See the coverage caveat: no capture exercises a POV change.
//!
//! **Full replace per hour.** Every record carries the whole vector and the fold
//! keeps only the latest (`world_fold.rs::fold_pov_province_visibility`); the accumulator is reset at each
//! hour boundary (`world_fold.rs::reset_hour_state`), so no state survives an hour and there is no
//! accumulator to patch. Recipe step 1 shape 1.
//!
//! **Four states: the collection declares NONE.** Every index in `0..count` has a
//! byte, there is no flag word anywhere in the grammar, and no read in the fold or
//! the codec can leave one slot unfilled — the vector is copied whole or the
//! payload is rejected whole. So `level` is always a plain value, a `0` is a real
//! measured Zero (record-state contract keeps zero out of the three empty states: it is the level of
//! a province this country cannot see, which is a fact about the world and not a
//! failure to capture it), and the schema declares no `.optional()`, no
//! `.nullable()` and no `.unknown_capable()`. A province id at or above `count`
//! is not an Absent field — it is a key that is not in the set at all.
//!
//! **Three inputs the payload does not carry** (recipe step 7), all read by the
//! fold, all typed parameters here:
//!
//! - the **d18 identity roster** — `country_identity_known_this_hour`
//!   (`world_fold.rs::fold_pov_province_visibility`) and `tag_by_db_id.contains_key(pov_country_id)`
//!   (`world_fold.rs::fold_pov_province_visibility`). Both are hard gates: the hour folds nothing without them. See
//!   [`Roster`].
//! - the **prior record of the same hour**, for the strictly-increasing
//!   `capture_sequence` rule (`world_fold.rs::fold_pov_province_visibility`). See [`State`].
//! - derived-field contract's `frame_tick`, which d94 has no derived field for and therefore ignores,
//!   so every domain's mapper keeps one shape.
//!
//! **Every non-folding path retracts the hour.** `fold_pov_province_visibility`
//! sets `pov_visibility_this_hour = None` on each of its three refusals
//! (`world_fold.rs::fold_pov_province_visibility`), and the dispatcher's error arm does the same
//! for a payload the codec refuses (`:4917`-`:4919`) — so a second record that
//! fails in an hour whose first record succeeded **erases** the good one rather
//! than leaving it standing. [`State::fold_payload`] mirrors that exactly; it is
//! the reason the error contract is stated rather than assumed.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 94;
pub const PROVINCES: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const POV_COUNTRY_ID: u32 = 1;
    pub const PROVINCE_ID: u32 = 2;
    pub const LEVEL: u32 = 3;
}

// --- Payload grammar, from `compact_contract/src/pov_visibility.rs`. ---------
//
// Re-stated here rather than imported because d94 is not admitted to the
// production-codec dependency surface. The measurement bin asserts that
// `PovVisibilitySnapshot::decode` and [`split_payload`] agree on every real
// payload AND on a swept space of synthetic ones, so the restatement cannot drift
// silently (`bin/bench_real_d94.rs`).

/// `CARRIER_MAGIC`, `pov_visibility.rs:5`.
pub const CARRIER_MAGIC: [u8; 4] = *b"FOW1";
/// `CARRIER_VERSION`, `pov_visibility.rs:6`.
pub const CARRIER_VERSION: u16 = 1;
/// `pov_visibility.rs:36`, `:48` — magic, version, flags, sequence, POV, count.
pub const HEADER_LEN: usize = 24;
/// `FLAG_PRODUCER_HOUR`, `pov_visibility.rs`. The hour the producer observed
/// at, appended AFTER the province vector so no pre-existing offset moves.
pub const FLAG_PRODUCER_HOUR: u16 = 1 << 0;
/// Every flag bit this carrier version defines; any other is refused.
pub const KNOWN_FLAGS: u16 = FLAG_PRODUCER_HOUR;
/// The width of the appended producer-hour word (`i64`, narrowed to `i32` by
/// the codec).
pub const HOUR_LEN: usize = 8;
/// One province slot is one byte. The "row" of this domain is that byte; its
/// position in the vector is its identity (see the module docs).
pub const ROW_LEN: usize = 1;
/// `MAX_PROVINCE_SLOTS`, `pov_visibility.rs`. A COUNT and a transport budget:
/// this domain is a DENSE one-byte-per-province vector, so this number sizes a
/// buffer taken from a stream word and must never be widened to the province
/// id space's structural width. The reasoning is recorded once, at the
/// contract's definition.
pub const MAX_PROVINCE_SLOTS: usize = 65_535;
/// `MAX_PROVINCE_SLOT_INDEX`, `pov_visibility.rs`. The ID side, DERIVED from
/// the count above: a d94 province id is a POSITION in the dense vector, so
/// the widest addressable index is one less than the widest vector. It does
/// NOT track `build_profile::MAX_PROVINCE_ID` and must not be pinned to it.
pub const MAX_PROVINCE_SLOT_INDEX: u32 = (MAX_PROVINCE_SLOTS - 1) as u32;
/// `MAX_POV_COUNTRY_ID`, `pov_visibility.rs`, which is the family bound
/// `build_profile::MAX_COMPACT_OWNER_TAG_DB_ID`.
///
/// Was `2_048`, a guess at the vanilla-plus-DLC country count.
/// ENGINE-STRUCTURAL now: `id < *(int32*)(db + 0x0C)` with a signed-`int` size
/// gives `i32::MAX - 1`. Pinned by
/// `tests/producer_consumer_admission_agreement.rs`.
pub const MAX_POV_COUNTRY_ID: u32 = (i32::MAX as u32) - 1;

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: PROVINCES,
            // Full-replace each hour (`world_fold.rs::fold_pov_province_visibility`), but the key is a
            // stable identity that persists across hours — which is what a keyed
            // set needs, and only a keyed set has a delta grammar (collection-shape contract). This is
            // the whole point of the port: the shipping arm re-sends 13,414 bytes
            // an hour for a table where a handful of slots move.
            shape: Shape::KeyedSet,
            fields: vec![
                // Key component 0: the observing country. `ContentId` because a
                // country tag is content-defined (content-identity contract) — a manifest must resolve
                // it for it to keep its meaning under another mod set, the
                // deciding question for ContentId-vs-Unsigned. d20's `owner` and
                // d63's `tag_db_id` are the same identity classed the same way.
                Field::new(f::POV_COUNTRY_ID, Kind::ContentId).key(0),
                // Key component 1: the province database id, which IS the byte's
                // position in `province_levels` (`pov_visibility.rs:14`).
                // `ContentId` for the same reason d20 classes `location_province`
                // one: province 1234 is whatever province the mod set defines.
                Field::new(f::PROVINCE_ID, Kind::ContentId).key(1),
                // The engine's fog-of-war level byte, emitted raw — the fold
                // applies no scale and no transform, it copies the vector
                // (`pov_visibility.rs:65`). A small `Unsigned`, not an `Enum`:
                // `Enum` rejects any code at or above its declared cardinality
                // (`record.rs:118`-`:121`), and nothing in the fold or the codec
                // bounds this byte, so an enum would refuse payloads the fold
                // folds — recipe step 2's "never gate against the fold". Not
                // `Monotone` either: visibility falls as well as rises.
                Field::new(f::LEVEL, Kind::Unsigned),
            ],
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 24-byte FOW1 header (`compact_contract/src/pov_visibility.rs:36`-`:44`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    pub count: u32,
}

/// Split a d94 DATA payload into its header and its province-level bytes,
/// applying every structural check `PovVisibilitySnapshot::decode` and
/// `::validate` apply (`pov_visibility.rs:19`-`:29`, `:47`-`:69`) and no others.
///
/// **All of them belong here, and none of them is deferrable.** Unlike the folds
/// that run some header checks before a status early exit and the rest after
/// (recipe step 7), d94's fold has no status word to exit on: it calls `decode`
/// as its very first act (`world_fold.rs::fold_pov_province_visibility`), so the codec's whole predicate
/// runs before anything else the fold does. A payload that fails any part of it
/// is not this grammar, and — this is the part that decides the harness — it is a
/// **stream rejection**, not an excluded hour: the recorder's second, status-
/// independent authority runs `decode` on every DOMAIN_DATA record
/// (`recorder_host/src/domain_data_authority.rs:16737`-`:16748`, dispatched
/// `:17141`), and its `parse_errors` become the `Err` that
/// `validate_domain_data_payload_at` reports at seal time
/// (`recorder_host/src/stream_fold.rs:593`-`:611`, `domain_data_authority.rs:17170`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// Total: no payload of any length or content panics.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    // `pov_visibility.rs:48` — length and magic are one check there, and the
    // length half must come first or the magic slice would panic.
    if payload.len() < HEADER_LEN || payload[..4] != CARRIER_MAGIC {
        return Err(format!(
            "pov_province_visibility carrier magic/length mismatch (len {})",
            payload.len()
        ));
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    let flags = u16::from_le_bytes([payload[6], payload[7]]);
    // `:53` — one check over both words, so a v2 carrier and an undefined flag
    // bit are the same refusal. The word that was `reserved` now carries
    // `FLAG_PRODUCER_HOUR`; every OTHER bit is still refused, so this is the
    // same predicate the codec applies and not a relaxation of it.
    if version != CARRIER_VERSION || flags & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "pov_province_visibility carrier version/reserved mismatch \
             (version {version}, reserved {flags})"
        ));
    }
    let header = PayloadHeader {
        capture_sequence: u64::from_le_bytes(payload[8..16].try_into().expect("8 bytes")),
        pov_country_id: u32_at(payload, 16),
        count: u32_at(payload, 20),
    };
    // `:59` — checked addition, because `count` is a u32 read from a file and
    // `24 + count` must not wrap on a 32-bit `usize`. Exact equality: a trailing
    // byte is as much a refusal as a truncation. The producer-hour word, when
    // its flag is set, is part of the expected length and NOT part of the
    // province vector — the codec slices the body as `[HEADER_LEN..HEADER_LEN +
    // count]` for exactly that reason.
    let hour_words = usize::from(flags & FLAG_PRODUCER_HOUR != 0);
    let expected = HEADER_LEN
        .checked_add(header.count as usize)
        .and_then(|len| len.checked_add(HOUR_LEN.saturating_mul(hour_words)))
        .ok_or_else(|| format!("pov_province_visibility carrier length overflow ({})", header.count))?;
    if payload.len() != expected {
        return Err(format!(
            "pov_province_visibility carrier is truncated or has trailing bytes \
             (len {}, header claims {expected})",
            payload.len()
        ));
    }
    // The codec narrows the appended `i64` to `i32` and refuses a word that does
    // not fit. Checking the tail's PRESENCE but not its VALUE would leave a
    // payload the recorder's own authority refuses and this restatement accepts,
    // which is exactly the drift the restatement exists to make impossible.
    //
    // It sits HERE, between the length check and the `validate` mirror, because
    // that is where `decode` does it: the codec reads the hour word while
    // building the snapshot and only then calls `validate`. Placed after the
    // three checks below, a payload with BOTH a zero sequence and an
    // out-of-range hour would report a different first reason here than the
    // codec reports, and this module's contract is that only which rejection is
    if hour_words == 1 {
        let at = HEADER_LEN + header.count as usize;
        let word = i64::from_le_bytes(payload[at..at + HOUR_LEN].try_into().expect("8 bytes"));
        if i32::try_from(word).is_err() {
            return Err(format!(
                "pov_province_visibility producer hour {word} is outside the engine's date range"
            ));
        }
    }
    // `validate`, `:19`-`:29`. Ordered as the codec orders them so the reported
    // reason is the codec's reason.
    if header.capture_sequence == 0 {
        return Err("pov_province_visibility capture sequence is zero".into());
    }
    if header.pov_country_id == 0 || header.pov_country_id > MAX_POV_COUNTRY_ID {
        return Err(format!(
            "pov_province_visibility POV country ID {} is outside the admitted range",
            header.pov_country_id
        ));
    }
    if header.count == 0 || header.count as usize > MAX_PROVINCE_SLOTS {
        return Err(format!(
            "pov_province_visibility province vector length {} is outside the admitted range",
            header.count
        ));
    }
    Ok((header, &payload[HEADER_LEN..HEADER_LEN + header.count as usize]))
}

/// The key of one province slot: `(pov_country_id, province_id)`, where
/// `province_id` **is** the slot's position in `province_levels`
/// (`compact_contract/src/pov_visibility.rs:14`).
///
/// A child row whose identity includes its parent's takes the parent id first
/// (recipe step 7, d23/d83's convention): the POV country is the frame's, so a
/// caller that already parsed the header does not parse it again.
///
/// Both components are range-checked against the codec's own admitted ranges, so
/// a key can never be built for a slot the codec would have refused.
pub fn key_from_province_slot(
    pov_country_id: u32,
    province_id: u32,
) -> core::result::Result<Key, String> {
    if pov_country_id == 0 || pov_country_id > MAX_POV_COUNTRY_ID {
        return Err(format!(
            "pov_province_visibility POV country ID {pov_country_id} is outside the admitted range"
        ));
    }
    if province_id > MAX_PROVINCE_SLOT_INDEX {
        return Err(format!(
            "pov_province_visibility province id {province_id} is outside the admitted range"
        ));
    }
    Ok(key(&[pov_country_id as u64, province_id as u64]))
}

/// One province slot of the `provinces` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw slot slice — **one byte**, the
/// caller having already consumed the 24-byte header via [`split_payload`] and
/// indexed the vector. d94 has exactly one slot form, so length selects nothing
/// and a wrong length is rejected.
///
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d94 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// The key components are not carried in the record body (record-state contract); they come from
/// [`key_from_province_slot`]. Nothing here can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d94 has no derived field; derived-field contract's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // The record is per collection, and this domain's single collection carries
    // exactly one value field. A record built for the WRONG collection would
    // otherwise come out plausible instead of failing (recipe step 7's key-only
    // rule, `variant_upgrades.rs:633`).
    if collection.collection_id != PROVINCES {
        return Err(format!(
            "pov_province_visibility has no mapping for collection {}",
            collection.collection_id
        ));
    }
    if row.len() != ROW_LEN {
        return Err(format!(
            "pov_province_visibility slot length {} != {ROW_LEN}",
            row.len()
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // Raw. The fold applies no scale to this byte and no transform: it
            // copies the vector whole (`pov_visibility.rs:65`). A zero is the
            // level of a province this country cannot see — a measured value.
            field::LEVEL => Value::Unsigned(row[0] as u64),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "pov_province_visibility has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold (recipe step 9's fourth public boundary) -----------

/// The two facts `fold_pov_province_visibility` reads out of the **d18
/// `country_identity`** accumulator, and nothing else
/// (`world_fold.rs::fold_pov_province_visibility`).
///
/// They are separate on purpose, because the recorder keeps them separate:
/// `tag_by_db_id` **persists across hours** — d18 is full-replace and the map is
/// only overwritten by a successful d18 fold (`world_fold.rs::fold_country_identity`) —
/// while `country_identity_known_this_hour` is cleared at every hour boundary
/// (`world_fold.rs::reset_hour_state`) and set only by that same successful fold (`world_fold.rs::fold_country_identity`). An
/// hour whose d18 record carried a failure status, or whose d18 payload the fold
/// refused, therefore has a *populated* roster that is nonetheless **not known
/// this hour**, and d94 folds nothing in it. Collapsing the two into "is the
/// roster non-empty" would measure hours the recorder publishes nothing for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    known_this_hour: bool,
    tag_db_ids: BTreeSet<u32>,
}

impl Roster {
    /// A roster that no d18 record has measured **this hour**. `tag_db_ids` is
    /// still whatever the last successful d18 fold left, because the recorder's
    /// map persists; d94 refuses on the freshness gate before it is ever read.
    pub fn carried_over(tag_db_ids: BTreeSet<u32>) -> Self {
        Self { known_this_hour: false, tag_db_ids }
    }

    /// A roster a d18 record measured this hour (`world_fold.rs::fold_country_identity`).
    pub fn measured_this_hour(tag_db_ids: BTreeSet<u32>) -> Self {
        Self { known_this_hour: true, tag_db_ids }
    }

    pub fn is_known_this_hour(&self) -> bool {
        self.known_this_hour
    }

    pub fn contains(&self, tag_db_id: u32) -> bool {
        self.tag_db_ids.contains(&tag_db_id)
    }

    pub fn len(&self) -> usize {
        self.tag_db_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tag_db_ids.is_empty()
    }
}

/// One accepted POV visibility snapshot — what the fold stores in
/// `pov_visibility_this_hour` (`world_fold.rs:1838`, `:10533`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    /// The complete engine byte vector; index is the province database id.
    pub province_levels: Vec<u8>,
}

/// The d94 accumulator for ONE game hour.
///
/// Nothing survives an hour: `finish_hour` clears both fields
/// (`world_fold.rs:3093`-`:3094`), so [`begin_hour`](State::begin_hour) is the
/// whole of the between-hours carry. That is what makes d94 full-replace rather
/// than incremental — there is no prior state for a payload to patch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    seen: bool,
    accepted: Option<Snapshot>,
}

impl State {
    /// `world_fold.rs::reset_hour_state` — the hour boundary reset.
    pub fn begin_hour(&mut self) {
        self.seen = false;
        self.accepted = None;
    }

    /// Whether any d94 record was observed this hour, of any outcome
    /// (`world_fold.rs::fold_pov_province_visibility`, and the error arm at `world_fold.rs::apply_domain_data`). The recorder
    /// publishes `null` rather than nothing for an hour that was seen and
    /// refused (`build_pov_visibility_value`, `world_fold.rs::build_pov_visibility_value`).
    pub fn seen(&self) -> bool {
        self.seen
    }

    /// The hour's accepted snapshot, or `None` if the hour folds nothing.
    pub fn accepted(&self) -> Option<&Snapshot> {
        self.accepted.as_ref()
    }

    /// Fold one d94 DATA payload, exactly as `fold_pov_province_visibility` does
    /// (`world_fold.rs::fold_pov_province_visibility`), in the fold's own order: decode, then
    /// the d18 freshness gate, then the d18 membership gate, then the
    /// strictly-increasing `capture_sequence` gate.
    ///
    /// **Error contract — this fold MUTATES BEFORE ERRORING, and the mutation is
    /// the point.** Every refusal clears the hour's accepted snapshot
    /// (`world_fold.rs::fold_pov_province_visibility`, and the dispatcher's arm at `world_fold.rs::apply_domain_data` for a
    /// payload the codec refuses), so a later bad record in an hour erases an
    /// earlier good one instead of leaving it standing. A caller that treats an
    /// `Err` as "nothing happened" disagrees with the recorder on exactly the
    /// hours that fail.
    ///
    /// **Which of recipe step 9's three outcomes each `Err` is** — the harness
    /// must say, by name:
    ///
    /// - a **decode** failure (magic, version, reserved, length, or any of
    ///   `validate`'s three range rules) is a **stream rejection**: the
    ///   recorder's status-independent second authority runs the same decode on
    ///   every DOMAIN_DATA record and its complaint refuses the stream at seal
    ///   time (see [`split_payload`]).
    /// - the two **roster** gates and the **sequence** gate are **excluded
    ///   hours**: the dispatcher catches the `Err` and marks the domain-hour
    ///   unknown (`world_fold.rs::apply_domain_data`), the stream stays valid, and the
    ///   hour publishes nothing for d94.
    pub fn fold_payload(
        &mut self,
        payload: &[u8],
        roster: &Roster,
    ) -> core::result::Result<(), String> {
        // `:10509` — set before the decode, so a refused payload still counts as
        // "a d94 record happened this hour".
        self.seen = true;
        let decoded = match split_payload(payload) {
            Ok((header, body)) => Snapshot {
                capture_sequence: header.capture_sequence,
                pov_country_id: header.pov_country_id,
                province_levels: body.to_vec(),
            },
            Err(error) => {
                // The dispatcher's arm, `:4919`.
                self.accepted = None;
                return Err(format!("pov_province_visibility FOW1 invalid: {error}"));
            }
        };
        // `:10512`-`:10517`.
        if !roster.is_known_this_hour() {
            self.accepted = None;
            return Err("pov_province_visibility requires a current-hour D18 identity roster".into());
        }
        // `:10518`-`:10524`.
        if !roster.contains(decoded.pov_country_id) {
            self.accepted = None;
            return Err(format!(
                "pov_province_visibility POV country {} is absent from current D18 roster",
                decoded.pov_country_id
            ));
        }
        // `:10525`-`:10532`. Strictly increasing WITHIN the hour: the comparison
        // is against this hour's accepted snapshot, and the accumulator was
        // cleared at the hour boundary, so a new hour never compares against the
        // previous one's sequence.
        if self
            .accepted
            .as_ref()
            .is_some_and(|current| decoded.capture_sequence <= current.capture_sequence)
        {
            self.accepted = None;
            return Err("pov_province_visibility sequence is not strictly increasing".into());
        }
        self.accepted = Some(decoded);
        Ok(())
    }

    /// The hour's state as the wire format's keyed set: one row per province
    /// slot, keyed `(pov_country_id, province_id)`, with the slot's byte as its
    /// `level`. `None` when the hour folded nothing.
    ///
    /// This is the only place the vector becomes rows, and it goes through
    /// [`key_from_province_slot`] and [`record_from_row`] rather than around
    /// them, so the measurement bin, the tests and any future caller share ONE
    /// derivation of the key and the record.
    pub fn keyed_state(
        &self,
        collection: &Collection,
    ) -> core::result::Result<Option<KeyedState>, String> {
        let Some(snapshot) = self.accepted.as_ref() else {
            return Ok(None);
        };
        let mut state: KeyedState = BTreeMap::new();
        for (province_id, level) in snapshot.province_levels.iter().enumerate() {
            // `count <= MAX_PROVINCE_SLOTS` is enforced by `split_payload`, so
            // this cast cannot truncate; it is written fallibly anyway because
            // `Snapshot` is publicly constructible.
            let province_id = u32::try_from(province_id)
                .map_err(|_| format!("pov_province_visibility province id {province_id} overflows"))?;
            let key = key_from_province_slot(snapshot.pov_country_id, province_id)?;
            let record = record_from_row(collection, std::slice::from_ref(level), 0)?;
            if state.insert(key, record).is_some() {
                // Unreachable by construction — `enumerate` yields each position
                // once — but a silent overwrite is the exact defect class this
                // port exists to prevent, so it is checked rather than argued.
                return Err(format!(
                    "pov_province_visibility duplicate province id {province_id}"
                ));
            }
        }
        Ok(Some(state))
    }
}

/// Callers reach this only after a length check, so the slice is in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{decode_baseline, encode_baseline};
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

    /// A raw FOW1 payload, built the way `PovVisibilitySnapshot::encode` packs one
    /// (`compact_contract/src/pov_visibility.rs:32`-`:45`).
    fn raw_payload(capture_sequence: u64, pov_country_id: u32, levels: &[u8]) -> Vec<u8> {
        raw_payload_with_hour(capture_sequence, pov_country_id, levels, None)
    }

    /// The same, optionally carrying the appended producer-hour word.
    fn raw_payload_with_hour(
        capture_sequence: u64,
        pov_country_id: u32,
        levels: &[u8],
        producer_hour_raw: Option<i32>,
    ) -> Vec<u8> {
        let flags = if producer_hour_raw.is_some() {
            FLAG_PRODUCER_HOUR
        } else {
            0
        };
        let mut out = Vec::with_capacity(HEADER_LEN + levels.len() + HOUR_LEN);
        out.extend_from_slice(&CARRIER_MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&capture_sequence.to_le_bytes());
        out.extend_from_slice(&pov_country_id.to_le_bytes());
        out.extend_from_slice(&(levels.len() as u32).to_le_bytes());
        out.extend_from_slice(levels);
        if let Some(hour) = producer_hour_raw {
            out.extend_from_slice(&i64::from(hour).to_le_bytes());
        }
        out
    }

    fn roster_with(ids: &[u32]) -> Roster {
        Roster::measured_this_hour(ids.iter().copied().collect())
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d94 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_pov_country_and_the_province() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "(pov_country_id, province_id)");
        assert_eq!(keys[0].field_id, field::POV_COUNTRY_ID);
        assert_eq!(keys[1].field_id, field::PROVINCE_ID);
        for k in &keys {
            assert_eq!(
                k.kind,
                Kind::ContentId,
                "field {}: a country tag and a province id are both content-defined (content-identity contract)",
                k.field_id
            );
            assert!(k.kind.legal_as_key());
            // key-encoding contract: a key member is concrete — never optional, nullable,
            // unknown-capable or derived.
            assert!(!k.optional && !k.nullable && !k.unknown_capable);
            assert!(k.derived.is_none());
        }
        assert_eq!(c.fields.len(), 3, "2 key components + 1 value field");
        assert_eq!(c.value_fields().count(), 1);
        assert_eq!(
            c.value_fields().next().unwrap().field_id,
            field::LEVEL
        );
        assert_eq!(c.value_fields().next().unwrap().kind, Kind::Unsigned);
    }

    #[test]
    fn the_collection_declares_no_capability_at_all() {
        // Recipe step 8's states test, in the form this collection calls for: it
        // declares NONE of the three, so there is no per-capability raw-bytes
        // case to write and this test records why. Every index in `0..count` has
        // a byte; there is no flag word anywhere in the grammar
        // (`compact_contract/src/pov_visibility.rs:32`-`:69`) and no per-slot
        // read that can fail — the vector is copied whole or the payload is
        // refused whole. So `level` is never Absent (the concept applies to every
        // slot the vector carries), never Null (it is a measurement, not a
        // relation that can point at nothing) and never Unknown (a slot the
        // codec accepted was captured). A zero is a real Zero: the level of a
        // province this country cannot see. The states the mapper CAN reach —
        // zero, the maximum, and every byte between — are swept in
        // `every_level_byte_maps_to_its_own_unsigned_value`.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));
        // And the mapper agrees with the declaration: over the whole input space
        // of a slot — all 256 byte values — it emits a value and never an empty
        // state.
        for byte in 0..=u8::MAX {
            let r = record_from_row(&c, &[byte], 0).expect("maps");
            let v = r.values[index_of(&c, field::LEVEL)];
            assert!(!v.is_null() && !v.is_unknown() && v.is_present(), "byte {byte}");
        }
    }

    #[test]
    fn no_field_is_quantized_because_none_is_continuous() {
        // Recipe step 8's quantization test does not apply: quantization is
        // declared on continuous fields only (derived-field contract, `schema.rs:228`), and d94 has
        // no continuous field at all — the payload is one unsigned byte per
        // province, emitted raw by the fold with no scale
        // (`compact_contract/src/pov_visibility.rs:65`). There is nothing to
        // quantize and nothing an all-`Exact` variant could be compared against.
        let c = collection();
        assert_eq!(
            c.fields
                .iter()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
        assert!(c.fields.iter().all(|f| f.quant == Quant::None));
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; a fog-of-war level does
        // not change deterministically with the clock, so `frame_tick` never
        // enters a value — the mapper takes it and ignores it.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        for tick in [0u64, 1, 4096, u64::MAX] {
            assert_eq!(
                record_from_row(&c, &[7], tick).expect("maps"),
                record_from_row(&c, &[7], 0).expect("maps"),
                "frame_tick {tick} must not change a value"
            );
        }
    }

    #[test]
    fn every_field_is_a_content_id_key_or_the_level() {
        // Recipe's codec-measurement contract sweep marker: "this domain has no ContentId" is a result,
        // not a silence — and so is the opposite. d94 has TWO, and both are KEY
        // components, which is the heavier retrofit class: key components sort
        // and delta-encode by raw value, so a manifest remap reorders keys and
        // changes the byte layout of every baseline and key list.
        let c = collection();
        let content_ids: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| f.kind == Kind::ContentId)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(content_ids, vec![field::POV_COUNTRY_ID, field::PROVINCE_ID]);
        assert!(
            c.fields
                .iter()
                .filter(|f| f.kind == Kind::ContentId)
                .all(|f| f.is_key()),
            "both ContentIds are key components"
        );
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &[250], 0).expect("maps");
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
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // The level of a province this country cannot see. record-state contract keeps zero out of
        // the three empty states, and it is by far the commonest level in the
        // capture (11,615,074 of 13,856,662 slots).
        let c = collection();
        let record = record_from_row(&c, &[0], 0).expect("maps");
        assert_eq!(
            record.values[index_of(&c, field::LEVEL)],
            Value::Unsigned(0)
        );
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, record);
    }

    #[test]
    fn every_level_byte_maps_to_its_own_unsigned_value() {
        // Recipe step 8's per-field offset/scale test, in the form this row has:
        // one byte, no scale, no gate. The whole input space, not a sample —
        // this is what catches a byte read as anything other than itself (a
        // sign-extension, a scale that should not be there, an off-by-one index).
        let c = collection();
        for byte in 0..=u8::MAX {
            let record = record_from_row(&c, &[byte], 0).expect("maps");
            assert_eq!(
                record.values[index_of(&c, field::LEVEL)],
                Value::Unsigned(byte as u64),
                "level byte {byte} must read as itself, raw and unscaled"
            );
        }
    }

    #[test]
    fn the_mapper_rejects_every_slot_the_grammar_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice: a slot is
        // exactly one byte.
        for len in [0usize, 2, 24, 100] {
            assert!(
                record_from_row(&c, &vec![0u8; len], 0).is_err(),
                "slot length {len} must be rejected, not read"
            );
        }
        // A collection this mapper cannot fill — a foreign collection id, and a
        // foreign field inside this one. Without the first check a key-only or
        // one-field record built for the wrong collection comes out plausible.
        let mut foreign = c.clone();
        foreign.collection_id = 99;
        assert!(record_from_row(&foreign, &[1], 0).is_err());
        let mut extra = c.clone();
        extra.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&extra, &[1], 0).is_err());
    }

    #[test]
    fn split_payload_reads_every_header_word_at_its_own_offset() {
        // Recipe step 8's header case: every word `split_payload` reads gets a
        // known-bytes case, because a transposed header word mis-slices every row
        // behind it and no round-trip assertion can see that — both arms agree on
        // the wrong slice. The values are deliberately all different and none is
        // a byte-swap of another.
        let payload = raw_payload(0x0102_0304_0506_0708, 1_234, &[9, 8, 7]);
        let (header, body) = split_payload(&payload).expect("well-formed");
        assert_eq!(header.capture_sequence, 0x0102_0304_0506_0708);
        assert_eq!(header.pov_country_id, 1_234);
        assert_eq!(header.count, 3);
        assert_eq!(body, &[9u8, 8, 7]);
        // The body starts exactly at +24: a header one byte short or long would
        // shift every level by a province.
        assert_eq!(&payload[HEADER_LEN..], body);
    }

    /// This restatement must accept and reject exactly what the codec does,
    /// including the producer-hour tail — a restatement that drifts from the
    /// recorder is a harness that can be wrong with nothing failing.
    /// The cases below pin trailer sizing, the body boundary, hour bounds, and
    /// error precedence against the codec.
    #[test]
    fn split_payload_enforces_the_codecs_structural_rules() {
        // `PovVisibilitySnapshot::decode` + `::validate`, rule by rule
        // (`compact_contract/src/pov_visibility.rs:19`-`:29`, `:47`-`:69`).
        let good = raw_payload(1, 7, &[0, 1, 2]);
        assert!(split_payload(&good).is_ok());

        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // Bad magic, every byte of it.
        for i in 0..4 {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            assert!(split_payload(&p).is_err(), "magic byte {i}");
        }
        // Any version but 1.
        for version in [0u16, 2, 0x0100, u16::MAX] {
            let mut p = good.clone();
            p[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(split_payload(&p).is_err(), "version {version}");
        }
        // Any flag word with a bit this version does not define.
        for reserved in [0x0002u16, 0x0100, u16::MAX] {
            let mut p = good.clone();
            p[6..8].copy_from_slice(&reserved.to_le_bytes());
            assert!(split_payload(&p).is_err(), "reserved {reserved}");
        }
        // `FLAG_PRODUCER_HOUR` is defined, so it is refused for its LENGTH and
        // not for its bit: the same word over a payload that actually carries
        // the trailing hour word is accepted, and the body it hands back stops
        // at the province vector.
        let mut flag_without_tail = good.clone();
        flag_without_tail[6..8].copy_from_slice(&FLAG_PRODUCER_HOUR.to_le_bytes());
        assert!(split_payload(&flag_without_tail).is_err());
        let with_hour = raw_payload_with_hour(1, 7, &[0, 1, 2], Some(60_832_312));
        let (header, body) = split_payload(&with_hour).expect("an hour-carrying payload splits");
        assert_eq!(header.count, 3);
        assert_eq!(body, &[0u8, 1, 2], "the hour word is not a province byte");
        // And the tail is exact in both directions, exactly as the vector is.
        assert!(split_payload(&with_hour[..with_hour.len() - 1]).is_err());
        let mut padded = with_hour.clone();
        padded.push(0);
        assert!(split_payload(&padded).is_err());
        // The tail's VALUE is checked too, not only its presence: the codec
        // narrows the appended `i64` to `i32` and refuses a word that does not
        // fit, so a restatement that accepted one would drift from the recorder.
        for word in [1i64 << 32, i64::MIN, i64::MAX, i64::from(i32::MAX) + 1] {
            let mut out_of_range = with_hour.clone();
            let at = out_of_range.len() - HOUR_LEN;
            out_of_range[at..].copy_from_slice(&word.to_le_bytes());
            assert!(split_payload(&out_of_range).is_err(), "hour word {word}");
        }
        // ...and the two edges that DO fit are still accepted.
        for word in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            let mut edge = with_hour.clone();
            let at = edge.len() - HOUR_LEN;
            edge[at..].copy_from_slice(&word.to_le_bytes());
            assert!(split_payload(&edge).is_ok(), "hour word {word}");
        }
        // And the hour is checked BEFORE the `validate` mirror, because that is
        // where `decode` checks it. A payload that offends both must report the
        // hour, not the sequence, or this restatement and the codec would
        // disagree about the reason for a refusal they both make.
        let mut both = raw_payload_with_hour(1, 7, &[0, 1, 2], Some(1));
        both[8..16].copy_from_slice(&0u64.to_le_bytes());
        let at = both.len() - HOUR_LEN;
        both[at..].copy_from_slice(&i64::MAX.to_le_bytes());
        let reason = split_payload(&both).expect_err("both rules are offended");
        assert!(
            reason.contains("producer hour"),
            "the codec reports the hour first; this reported {reason:?}"
        );
        // A zero capture sequence (`:20`).
        let mut p = good.clone();
        p[8..16].copy_from_slice(&0u64.to_le_bytes());
        assert!(split_payload(&p).is_err());
        // A POV country outside `1..=MAX_POV_COUNTRY_ID` (`:23`), and the two
        // admitted edges accepted, so the bound is not off by one.
        for pov in [0u32, MAX_POV_COUNTRY_ID + 1, u32::MAX] {
            let mut p = good.clone();
            p[16..20].copy_from_slice(&pov.to_le_bytes());
            assert!(split_payload(&p).is_err(), "pov {pov}");
        }
        for pov in [1u32, MAX_POV_COUNTRY_ID] {
            let mut p = good.clone();
            p[16..20].copy_from_slice(&pov.to_le_bytes());
            assert!(split_payload(&p).is_ok(), "pov {pov}");
        }
        // Truncation and trailing bytes are both refused (`:59`).
        assert!(split_payload(&good[..good.len() - 1]).is_err());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err());
        // A count that disagrees with the length, in both directions, and one
        // whose byte length would overflow `usize` on a 32-bit target.
        for count in [0u32, 2, 4, u32::MAX] {
            let mut p = good.clone();
            p[20..24].copy_from_slice(&count.to_le_bytes());
            assert!(split_payload(&p).is_err(), "count {count}");
        }
        // An empty vector is refused even when the length agrees (`:26`).
        let empty = raw_payload(1, 7, &[]);
        assert!(split_payload(&empty).is_err());
        // A vector of exactly `MAX_PROVINCE_SLOTS` is accepted; the codec's bound
        // is inclusive (`:26` rejects `> MAX_PROVINCE_SLOTS`).
        let biggest = raw_payload(1, 7, &vec![0u8; MAX_PROVINCE_SLOTS]);
        assert!(split_payload(&biggest).is_ok());
        // One past it is refused, and the length still agrees — so this rejection
        // is the range rule and not the length rule.
        let too_big = raw_payload(1, 7, &vec![0u8; MAX_PROVINCE_SLOTS + 1]);
        assert!(split_payload(&too_big).is_err());
    }

    #[test]
    fn split_payload_never_panics_on_arbitrary_bytes() {
        // It is fed a payload read off disk, so it must diagnose, never unwind.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..3_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 80;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(&CARRIER_MAGIC);
            }
            let _ = split_payload(&bytes);
        }
        // And byte-by-byte corruption of a well-formed payload.
        let good = raw_payload(1, 7, &[0, 1, 2, 3]);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let _ = split_payload(&p);
        }
    }

    #[test]
    fn the_key_is_the_pov_country_then_the_province_index() {
        assert_eq!(
            key_from_province_slot(368, 1_234).expect("keys"),
            key(&[368, 1_234])
        );
        // The two components are NOT interchangeable — the guard against the
        // transposition d62's review found in an adjacent pair.
        assert_ne!(
            key_from_province_slot(368, 1_234).expect("keys"),
            key(&[1_234, 368])
        );
        // Province 0 is a legal slot: the engine vector has a byte at index 0 and
        // the fold carries it, so refusing it would drop a row the recorder
        // publishes. (It reads 0 in all 1,033 hours of the reference capture; see
        // the measurement's coverage caveats.)
        assert_eq!(key_from_province_slot(1, 0).expect("keys"), key(&[1, 0]));
        // Out-of-range components are refused rather than silently truncated.
        for pov in [0u32, MAX_POV_COUNTRY_ID + 1, u32::MAX] {
            assert!(key_from_province_slot(pov, 1).is_err(), "pov {pov}");
        }
        for province in [MAX_PROVINCE_SLOTS as u32, u32::MAX] {
            assert!(key_from_province_slot(1, province).is_err(), "province {province}");
        }
        assert!(key_from_province_slot(1, MAX_PROVINCE_SLOTS as u32 - 1).is_ok());
    }

    #[test]
    fn the_keyed_state_is_the_vector_indexed_by_province() {
        // The headline claim, made checkable: the byte at index i IS province i's
        // level, for every i, with no shift and no reordering.
        let c = collection();
        let levels: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
        let mut state = State::default();
        state
            .fold_payload(&raw_payload(9, 368, &levels), &roster_with(&[368]))
            .expect("folds");
        let keyed = state.keyed_state(&c).expect("materializes").expect("some");
        assert_eq!(keyed.len(), levels.len());
        for (i, level) in levels.iter().enumerate() {
            let k = key_from_province_slot(368, i as u32).expect("keys");
            let record = keyed.get(&k).unwrap_or_else(|| panic!("province {i}"));
            assert_eq!(
                record.values[index_of(&c, field::LEVEL)],
                Value::Unsigned(*level as u64),
                "province {i}"
            );
        }
        // And it survives a baseline round-trip: the reader reconstructs the same
        // (key, level) pairs, in the same order, byte-identically.
        let schema = schema();
        let mut bytes = Vec::new();
        encode_baseline(&schema, std::slice::from_ref(&keyed), &mut bytes).expect("encodes");
        let (states, consumed) = decode_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(states[0], keyed);
        let mut reencoded = Vec::new();
        encode_baseline(&schema, &states, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "baseline round-trip is byte-identical");
    }

    #[test]
    fn the_fold_requires_a_current_hour_d18_roster() {
        // `world_fold.rs::fold_pov_province_visibility`. A populated but stale roster is still a
        // refusal: `tag_by_db_id` persists across hours while
        // `country_identity_known_this_hour` is cleared at every boundary, so
        // "the map has the country" is not the gate.
        let payload = raw_payload(5, 368, &[1, 2, 3]);
        let mut state = State::default();
        let stale = Roster::carried_over([368u32].into_iter().collect());
        assert!(!stale.is_known_this_hour());
        assert!(stale.contains(368));
        assert!(state.fold_payload(&payload, &stale).is_err());
        assert!(state.seen(), "a refused record was still seen");
        assert!(state.accepted().is_none());
        // The same bytes with a roster measured this hour fold.
        let mut state = State::default();
        state
            .fold_payload(&payload, &roster_with(&[368]))
            .expect("folds");
        assert_eq!(state.accepted().expect("accepted").pov_country_id, 368);
    }

    #[test]
    fn the_fold_requires_the_pov_country_to_be_in_the_roster() {
        // `world_fold.rs::fold_pov_province_visibility`.
        let payload = raw_payload(5, 368, &[1, 2, 3]);
        let mut state = State::default();
        assert!(state
            .fold_payload(&payload, &roster_with(&[1, 2, 369]))
            .is_err());
        assert!(state.accepted().is_none());
        // Present in the roster: folds. An empty roster measured this hour is
        // still a refusal, which is the membership gate and not the freshness one.
        let mut state = State::default();
        assert!(state.fold_payload(&payload, &roster_with(&[])).is_err());
        let mut state = State::default();
        assert!(state
            .fold_payload(&payload, &roster_with(&[367, 368, 369]))
            .is_ok());
    }

    #[test]
    fn the_capture_sequence_must_strictly_increase_within_the_hour() {
        // `world_fold.rs::fold_pov_province_visibility`, and the retraction that comes with it.
        let roster = roster_with(&[368]);
        let mut state = State::default();
        state
            .fold_payload(&raw_payload(10, 368, &[1, 1, 1]), &roster)
            .expect("first record folds");
        // Strictly greater: accepted, and it REPLACES the earlier one (full
        // replace, `:10533`).
        state
            .fold_payload(&raw_payload(11, 368, &[2, 2, 2]), &roster)
            .expect("a greater sequence folds");
        assert_eq!(
            state.accepted().expect("accepted").province_levels,
            vec![2, 2, 2]
        );
        // Equal and lower are both refused — and both ERASE the accepted
        // snapshot, so the hour publishes nothing. A caller that treats the Err
        // as "keep what you had" disagrees with the recorder.
        for sequence in [11u64, 10, 1] {
            let mut state = State::default();
            state
                .fold_payload(&raw_payload(11, 368, &[2, 2, 2]), &roster)
                .expect("first record folds");
            assert!(state.accepted().is_some());
            assert!(
                state
                    .fold_payload(&raw_payload(sequence, 368, &[3, 3, 3]), &roster)
                    .is_err(),
                "sequence {sequence} after 11"
            );
            assert!(
                state.accepted().is_none(),
                "sequence {sequence}: the refusal must retract the hour"
            );
            assert!(state.seen());
        }
    }

    #[test]
    fn every_refusal_retracts_the_hour() {
        // The error contract, swept over all four refusal paths rather than the
        // one the sequence test covers: a decode failure (`:4919`), the two
        // roster gates (`:10513`, `:10519`) and the sequence gate (`:10530`) each
        // clear the accepted snapshot.
        let roster = roster_with(&[368]);
        let good = raw_payload(10, 368, &[1, 1, 1]);
        let mut truncated = raw_payload(10, 368, &[1, 1, 1]);
        truncated.pop();
        let cases: Vec<(&str, Vec<u8>, Roster)> = vec![
            ("decode failure", truncated, roster.clone()),
            (
                "stale roster",
                good.clone(),
                Roster::carried_over([368u32].into_iter().collect()),
            ),
            ("pov not in roster", good.clone(), roster_with(&[1])),
            ("sequence not increasing", raw_payload(10, 368, &[4, 4, 4]), roster.clone()),
        ];
        for (label, payload, roster_for_case) in cases {
            let mut state = State::default();
            state.fold_payload(&good, &roster).expect("first record folds");
            assert!(state.accepted().is_some());
            assert!(
                state.fold_payload(&payload, &roster_for_case).is_err(),
                "{label} must be refused"
            );
            assert!(
                state.accepted().is_none(),
                "{label} must retract the hour's accepted snapshot"
            );
            assert!(
                state.keyed_state(&collection()).expect("materializes").is_none(),
                "{label} must publish no keyed state"
            );
        }
    }

    #[test]
    fn the_hour_boundary_clears_everything() {
        // `world_fold.rs::reset_hour_state`. Nothing survives an hour, which is what
        // makes d94 full-replace: a new hour's first record never compares its
        // sequence against the previous hour's.
        let roster = roster_with(&[368]);
        let mut state = State::default();
        state
            .fold_payload(&raw_payload(100, 368, &[1]), &roster)
            .expect("folds");
        state.begin_hour();
        assert!(!state.seen());
        assert!(state.accepted().is_none());
        // A LOWER sequence in the new hour folds, because the comparison is
        // within-hour only.
        state
            .fold_payload(&raw_payload(2, 368, &[5]), &roster)
            .expect("a new hour starts a new sequence chain");
        assert_eq!(state.accepted().expect("accepted").capture_sequence, 2);
    }

    #[test]
    fn a_pov_change_replaces_the_whole_key_space() {
        // The reason `pov_country_id` is a key component and not a frame input:
        // the accumulator holds ONE snapshot per hour, so an hour under a new POV
        // is that country's vector alone. With the POV in the key the diff is a
        // full replacement — every old key tombstoned, every new key inserted —
        // which is exactly what the recorder publishes. With the POV outside the
        // key the two hours would be indistinguishable and one country's levels
        // would be read as another's.
        use crate::body::diff;
        let c = collection();
        let mut state = State::default();
        state
            .fold_payload(&raw_payload(1, 368, &[1, 2, 3]), &roster_with(&[368, 42]))
            .expect("folds");
        let first = state.keyed_state(&c).expect("materializes").expect("some");
        state.begin_hour();
        state
            .fold_payload(&raw_payload(1, 42, &[1, 2, 3]), &roster_with(&[368, 42]))
            .expect("folds");
        let second = state.keyed_state(&c).expect("materializes").expect("some");
        let d = diff(&c, &first, &second);
        assert_eq!(d.changed.len(), 3, "every row of the new POV is new");
        assert_eq!(d.tombstones.len(), 3, "every row of the old POV is gone");
        // Identical level bytes, and yet nothing is shared: the keys differ.
        assert!(first.keys().all(|k| !second.contains_key(k)));
    }

    #[test]
    fn a_refused_payload_never_reaches_the_roster_gates() {
        // The fold decodes FIRST (`world_fold.rs::fold_pov_province_visibility`), so a malformed payload
        // is refused as a malformed payload whatever the roster says — which
        // matters because the two refusals are different outcomes for the
        // harness: a decode failure is a stream rejection, a roster gate is an
        // excluded hour.
        let mut bad = raw_payload(1, 368, &[1, 2, 3]);
        bad[4..6].copy_from_slice(&2u16.to_le_bytes()); // version 2
        for roster in [
            roster_with(&[368]),
            roster_with(&[]),
            Roster::carried_over([368u32].into_iter().collect()),
        ] {
            let mut state = State::default();
            let error = state
                .fold_payload(&bad, &roster)
                .expect_err("a v2 carrier is not this grammar");
            assert!(
                error.contains("FOW1 invalid"),
                "the decode failure must be reported as one, not as a roster gate: {error}"
            );
        }
    }

    #[test]
    fn folding_never_panics_on_arbitrary_bytes() {
        let roster = roster_with(&[368]);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..2_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed as usize) % 96;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            if len >= 4 && seed & 1 == 0 {
                bytes[..4].copy_from_slice(&CARRIER_MAGIC);
            }
            let mut state = State::default();
            let _ = state.fold_payload(&bytes, &roster);
            let _ = state.keyed_state(&collection());
        }
    }
}
