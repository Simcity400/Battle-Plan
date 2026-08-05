//! d61 `air_loss_stats_live` — per-country CUMULATIVE strategic-air loss statistics.
//!
//! Derived from `world_fold.rs:16926`-`:16979`
//! (`fold_air_loss_stats`, dispatched at `:4708`), its JSON emission at
// @world-fold-authority domain=61 functions=fold_air_loss_stats lines=16926-16979 sha256=66ea65854ffcdc1958091016b6f1d524fd91b017b314f6a2452f5abf8aa21906
//! `:9150`-`:9183`, and the payload codec the fold calls, which for this domain
//! lives **outside** the fold in its own leaf crate:
//! `extractor/native/lanes/air_loss_stats_live/src/lib.rs` (`decode_payload_rows`
//! at `:278`, the live derivation `derive_country_row_result` at `:531`).
//!
//! The domain is the compact-packet DATA layer for the C1 crack: statistic id 23,
//! the per-country 89-field aggregate air-loss history record, of which the 24
//! fields the strategic-air details panel displays are emitted in exact engine
//! display order (`lib.rs:71`-`:96`, pinned by the static `u32[24]` channel table
//! at RVA `0x29f8940` and the `AIRVIEW_DETAILS_CHANNEL_1..24` localisation keys).
//!
//! **Payload grammar** (`lib.rs:135`-`:137`, `:181`-`:201`, `:278`-`:357`): a
//! 32-byte header — `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`,
//! `status_flags` u32 at `+12`, `schema_version` u32 at `+16`, `field_count` u32
//! at `+20`, `row_len` u32 at `+24`, and a reserved pad u32 at `+28` — followed
//! by `row_count` fixed 208-byte rows. The header carries its own `row_len`,
//! `schema_version` and `field_count`, and the codec rejects any payload whose
//! `schema_version != 2`, `field_count != 89` or `row_len != 208`
//! (`lib.rs:297`-`:311`), so the length never selects a row form: there is
//! exactly one, and no legacy short row exists.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id` | raw | none (key 0) |
//! | `+4`  | u32 | `country_idx`     | raw | none — **not on the wire**, see below |
//! | `+8`  | u32 | `flags`           | —   | — **not on the wire**, see below |
//! | `+12` | u32 | pad               | —   | never read by the codec or the fold |
//! | `+16 + 8k` | i64 | channel `k`, `0 <= k < 24` | raw; see the divisor note | none |
//!
//! # The two divisors, and why the wire carries the raw integer
//!
//! `CHANNEL_DISPLAY_DIVISORS` (`lib.rs:102`-`:127`) is one fixed array holding
//! **two** different constants: channels 0 (`our_buildings_bombed`) and 12
//! (`enemy_buildings_bombed`) divide by `BUILDINGS_BOMBED_DISPLAY_SCALE = 1_000`
//! (`lib.rs:58`), and the other 22 divide by `FIELD_SCALE = 100_000`
//! (`lib.rs:57`). Reading either constant for the wrong channel is a 100×
//! error that every round-trip test still passes, because the error would be in
//! the mapping and not in the codec — recipe step 1's named failure class.
//!
//! This port does not have to choose a divisor at all, and that is the point.
//! **The fold's own emitter carries all three quantities per channel**
//! (`world_fold.rs::insert_air_loss_stats`): `channels[label]` is `row.channel_value(k)`,
//! `raw[label]` is `row.channels[k]`, and `display_divisors[label]` is
//! `CHANNEL_DISPLAY_DIVISORS[k]`. The display value is therefore a **pure
//! function of the raw word and a compile-time constant of the grammar** —
//! recipe step 2's rule for a value the fold hard-enforces as a function of
//! carried state: enforce it, test the identity, and leave it off the wire,
//! because re-encoding it would be a second copy of state the decoder must then
//! keep consistent. So each channel ships as the engine's exact raw `i64` and
//! [`channel_display_value`] reproduces `AirLossStatRow::channel_value`
//! (`lib.rs:161`-`:163`) from it. The divisor stays load-bearing and is pinned
//! per channel by [`tests::every_channel_carries_its_own_divisor`].
//!
//! Storing the raw word also makes the port lossless for free: a raw `i64` in an
//! integer kind is exact over the whole range the codec admits, so there is no
//! reproduction band to state and no `Fixed(k)` rounding to bound. Shipping the
//! display value instead would need `Fixed(3)` on two fields and `Fixed(5)` on
//! twenty-two — representable, but it would convert an exact integer counter
//! into a binary64 and put the hazard *inside* the encoded bytes rather than in
//! one tested helper. Recorded in the deliverable's open questions.
//!
//! **`MAX_CHANNEL_RAW = 1e12` is the fold's own fail-closed guard, and mirroring
//! it is not gating against the fold.** `decode_payload_rows` (`lib.rs:340`-`:342`)
//! rejects any channel outside `0 ..= 1_000_000_000_000`, and the live derivation
//! fails the whole row closed on the same bound (`lib.rs:516`, `:841`) so a
//! torn or aliased read cannot pass the structural checks. Step 2's d36 rule
//! forbids refusing rows the fold **accepts**; the fold accepts none of these, so
//! the mapper mirrors the rejection. The band is documented rather than assumed:
//! within it the stored value reproduces the raw integer exactly and without
//! qualification, because `Kind::Monotone` encodes a `u64` varint
//! (`record.rs:383`) rather than a quantized float.
//!
//! # Four states: this domain declares none of them
//!
//! No field can be absent, null or unknown, and the schema declares no
//! capability. There is no gate anywhere in this row: every one of the 24
//! channels is read unconditionally at a fixed offset and range-checked
//! (`lib.rs:336`-`:344`), and the two identity words are read unconditionally
//! too. A country whose read failed never becomes a row — the live derivation
//! returns `Absent` for a clean null optional link and `Failed` for any
//! unreadable address or failed guard (`lib.rs:531`-`:856`), and **neither
//! emits a row**; "fail-closed per field: an unreadable/out-of-range value folds
//! ABSENT, never a fabricated 0" is the lane's stated contract (`lib.rs:26`-`:28`).
//! In a keyed set that is the absence of the row, which is collection
//! membership, not a §3 field state.
//!
//! **This is also what settles the witness bits.** `BINDING_VERIFIED` and
//! `STRUCT_VERIFIED` look like they gate Unknown-vs-measured, but they cannot:
//! `decode_payload_rows` rejects a row carrying any bit outside the two
//! (`lib.rs:328`-`:330`) **and** a row missing either of them (`lib.rs:331`-`:335`),
//! and either rejection makes the whole payload an `Err` that the fold never
//! folds (`world_fold.rs::fold_air_loss_stats`). So on the whole set of rows this
//! schema can ever describe, `flags` is the constant `0b11`: it has no state to
//! encode, and every channel on a row that exists is a measured value. The
//! witnesses are witnesses *about a row that exists*, not gates on its fields —
//! the d62 precedent exactly (`d62.rs:56`-`:70`). The mapper enforces the
//! invariant the omission rests on (any other flags word is an `Err`, never a
//! coerced value), and the grammar is version-pinned, so a collector that gave
//! the bits per-row meaning would have to bump `SCHEMA_VERSION`, which
//! [`split_payload`] rejects.
//!
//! **`country_idx` is off the wire for the same reason, one rule up.** The fold
//! rejects the entire payload when `owner_tag_db_id != country_idx`
//! (`world_fold.rs:15908`-`:15912`), so on every row the fold folds the two words
//! are the same number and the second is a second copy of the key. [`record_from_row`]
//! enforces the identity and does not carry it, per step 2's derived-value rule.
//!
//! The row's pad word at `+12` is read by nothing — not the codec, not the fold —
//! so the mapper reads nothing there either. Refusing a non-zero pad would refuse
//! payloads the recorder folds.
//!
//! # Shape, and what the accumulator does
//!
//! **Full replace**: `self.air_loss_stats = fresh` (`world_fold.rs::fold_air_loss_stats`) discards
//! the prior table wholesale, and a duplicate `owner_tag_db_id` is a rejected
//! payload (`world_fold.rs::fold_air_loss_stats`), never a last-writer-wins overwrite — which is what
//! makes the key a real identity. The country is a stable identity that persists
//! across hours, so §7.3's keyed set applies and the domain has a delta grammar.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock. The channels are cumulative counters, which is a §4 `Monotone` *hint*
//! and never a constraint — and this domain has a concrete reason not to enforce
//! it: the crack reads a **yearly** cumulative window (`lib.rs:36`,
//! `LOOPHIST_WINDOW_OFF` with `EXPECTED_YEARLY_CAPACITY = 5`), so a channel may
//! legitimately fall at a year boundary. The mapper asserts no monotonicity and
//! the measurement bin reports observed decreases rather than refusing them. The
//! mapper still takes `frame_tick` so every domain's boundary has one shape.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 61;
pub const COUNTRY_AIR_LOSS_STATS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
/// The 24 channel ids are `2 ..= 25`, assigned in `PINNED_CHANNELS` order
/// (`lanes/air_loss_stats_live/src/lib.rs:71`-`:96`) — the exact engine display
/// order, which is NOT the aggregate-history field order.
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;

    pub const OUR_BUILDINGS_BOMBED: u32 = 2;
    pub const OUR_TROOPS_BOMBED: u32 = 3;
    pub const OUR_SHIPS_BOMBED: u32 = 4;
    pub const OUR_FIGHTERS_LOST: u32 = 5;
    pub const OUR_BOMBERS_LOST: u32 = 6;
    pub const OUR_CAS_LOST: u32 = 7;
    pub const OUR_TRANSPORTS_LOST: u32 = 8;
    pub const OUR_BOMBERS_DISRUPTED: u32 = 9;
    pub const OUR_TRAINS_LOST: u32 = 10;
    pub const OUR_TRUCKS_LOST: u32 = 11;
    pub const OUR_RAILWAY_GUNS_LOST: u32 = 12;
    pub const OUR_MISSILES_LOST: u32 = 13;
    pub const ENEMY_BUILDINGS_BOMBED: u32 = 14;
    pub const ENEMY_TROOPS_BOMBED: u32 = 15;
    pub const ENEMY_SHIPS_SUNK: u32 = 16;
    pub const ENEMY_FIGHTERS_LOST: u32 = 17;
    pub const ENEMY_BOMBERS_LOST: u32 = 18;
    pub const ENEMY_CAS_LOST: u32 = 19;
    pub const ENEMY_TRANSPORTS_LOST: u32 = 20;
    pub const ENEMY_BOMBERS_DISRUPTED: u32 = 21;
    pub const ENEMY_TRAINS_LOST: u32 = 22;
    pub const ENEMY_TRUCKS_LOST: u32 = 23;
    pub const ENEMY_RAILWAY_GUNS_LOST: u32 = 24;
    pub const ENEMY_MISSILES_LOST: u32 = 25;
}

// --- Payload grammar, from `lanes/air_loss_stats_live/src/lib.rs`. ------------
//
// The constants are mirrored here rather than imported: `wire_format` is the
// reference implementation of the spec and takes no dependency on a capture
// lane. Every one carries the line it was taken from, and the version pin below
// is what makes the mirror safe — a collector that changed the grammar must bump
// `SCHEMA_VERSION`, and `split_payload` refuses any other value. See the
// deliverable's open questions.

/// `PAYLOAD_HEADER_LEN`, `lib.rs:135`.
pub const HEADER_LEN: usize = 32;
/// `ROW_LEN`, `lib.rs:137`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 208;
/// `SCHEMA_VERSION`, `lib.rs:134`. `decode_payload_rows` (`:297`) rejects any
/// other value, so this pins the grammar the offsets above belong to.
pub const SCHEMA_VERSION: u32 = 2;
/// `EXPECTED_FIELD_COUNT`, `lib.rs:55` — the CLoopHistory field count the crack
/// requires. `decode_payload_rows` (`:303`) rejects any other value.
pub const EXPECTED_FIELD_COUNT: u32 = 89;

/// The number of strategic-air details channels (`CHANNEL_COUNT`, `lib.rs:98`).
pub const CHANNEL_COUNT: usize = 24;

/// `FIELD_SCALE`, `lib.rs:57` — the display divisor of 22 of the 24 channels.
pub const FIELD_SCALE: i64 = 100_000;
/// `BUILDINGS_BOMBED_DISPLAY_SCALE`, `lib.rs:58` — the display divisor of
/// channels 0 and 12, and of no other channel.
pub const BUILDINGS_BOMBED_DISPLAY_SCALE: i64 = 1_000;

/// `MAX_CHANNEL_RAW`, `lib.rs:131`: the fail-closed upper bound on a raw channel
/// word (10,000,000 planes × `FIELD_SCALE`). `decode_payload_rows` (`:340`) and
/// the live derivation (`:516`, `:841`) both reject outside `0 ..= this`, so the
/// mapper mirrors the rejection rather than widening it. See the module docs.
pub const MAX_CHANNEL_RAW: i64 = 1_000_000_000_000;

pub const ROW_FLAG_BINDING_VERIFIED: u32 = 1 << 0;
pub const ROW_FLAG_STRUCT_VERIFIED: u32 = 1 << 1;
/// `lib.rs:328`-`:335`: a row must carry **exactly** these two bits. Any extra
/// bit and any missing bit are both hard rejections, which is why this is one
/// constant and not a "known" mask plus a separate "required" mask.
pub const REQUIRED_FLAGS: u32 = ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED;

/// Byte offset of the first channel word within a row (`lib.rs:338`).
pub const CHANNELS_OFFSET: usize = 16;

/// One strategic-air details channel: its stable wire field id, its index into
/// the engine's 89-field aggregate-history array, its stable label, and the
/// engine's display divisor for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Channel {
    pub field_id: u32,
    /// The aggregate-history field index (`PINNED_CHANNELS.0`, `lib.rs:71`-`:96`).
    /// Carried for provenance only: the row packs channels densely in display
    /// order, so this index does not address anything in the payload.
    pub aggregate_index: u32,
    pub label: &'static str,
    /// `CHANNEL_DISPLAY_DIVISORS[k]`, `lib.rs:102`-`:127`.
    pub display_divisor: i64,
}

/// The 24 channels in exact engine display order — `PINNED_CHANNELS` zipped with
/// `CHANNEL_DISPLAY_DIVISORS` (`lib.rs:71`-`:127`), which are two parallel arrays
/// there and one table here so an index cannot drift between them.
pub const CHANNELS: [Channel; CHANNEL_COUNT] = [
    Channel { field_id: field::OUR_BUILDINGS_BOMBED, aggregate_index: 0, label: "our_buildings_bombed", display_divisor: BUILDINGS_BOMBED_DISPLAY_SCALE },
    Channel { field_id: field::OUR_TROOPS_BOMBED, aggregate_index: 1, label: "our_troops_bombed", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_SHIPS_BOMBED, aggregate_index: 2, label: "our_ships_bombed", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_FIGHTERS_LOST, aggregate_index: 3, label: "our_fighters_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_BOMBERS_LOST, aggregate_index: 10, label: "our_bombers_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_CAS_LOST, aggregate_index: 17, label: "our_cas_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_TRANSPORTS_LOST, aggregate_index: 24, label: "our_transports_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_BOMBERS_DISRUPTED, aggregate_index: 39, label: "our_bombers_disrupted", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_TRAINS_LOST, aggregate_index: 36, label: "our_trains_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_TRUCKS_LOST, aggregate_index: 37, label: "our_trucks_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_RAILWAY_GUNS_LOST, aggregate_index: 38, label: "our_railway_guns_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::OUR_MISSILES_LOST, aggregate_index: 40, label: "our_missiles_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_BUILDINGS_BOMBED, aggregate_index: 41, label: "enemy_buildings_bombed", display_divisor: BUILDINGS_BOMBED_DISPLAY_SCALE },
    Channel { field_id: field::ENEMY_TROOPS_BOMBED, aggregate_index: 42, label: "enemy_troops_bombed", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_SHIPS_SUNK, aggregate_index: 43, label: "enemy_ships_sunk", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_FIGHTERS_LOST, aggregate_index: 44, label: "enemy_fighters_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_BOMBERS_LOST, aggregate_index: 51, label: "enemy_bombers_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_CAS_LOST, aggregate_index: 58, label: "enemy_cas_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_TRANSPORTS_LOST, aggregate_index: 65, label: "enemy_transports_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_BOMBERS_DISRUPTED, aggregate_index: 80, label: "enemy_bombers_disrupted", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_TRAINS_LOST, aggregate_index: 77, label: "enemy_trains_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_TRUCKS_LOST, aggregate_index: 78, label: "enemy_trucks_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_RAILWAY_GUNS_LOST, aggregate_index: 79, label: "enemy_railway_guns_lost", display_divisor: FIELD_SCALE },
    Channel { field_id: field::ENEMY_MISSILES_LOST, aggregate_index: 81, label: "enemy_missiles_lost", display_divisor: FIELD_SCALE },
];

/// The byte offset of channel `k`'s raw word within a row (`lib.rs:338`), or
/// `None` for an index this domain has no channel for.
///
/// Total over every `usize`: a public entry point that takes a caller-supplied
/// index must not depend on the caller having bounded it. `CHANNELS_OFFSET +
/// channel * 8` overflows for a large enough index, which panics under this
/// workspace's overflow checks and silently wraps to a plausible in-row offset
/// without them — the second being the worse outcome, since it would read a
/// real word from the wrong place.
pub const fn channel_offset(channel: usize) -> Option<usize> {
    if channel < CHANNEL_COUNT {
        Some(channel_offset_in_range(channel))
    } else {
        None
    }
}

/// The same arithmetic for a caller that has already proved its index is a
/// channel index. Private, and every call site below iterates `0..CHANNEL_COUNT`
/// or indexes `CHANNELS`, so the multiplication is bounded by `23 * 8` and
/// cannot overflow.
const fn channel_offset_in_range(channel: usize) -> usize {
    CHANNELS_OFFSET + channel * 8
}

/// The engine's human display value for a channel's raw word — exactly
/// `AirLossStatRow::channel_value` (`lib.rs:161`-`:163`), which the fold's own
/// emitter calls to build its `channels` map (`world_fold.rs::insert_air_loss_stats`).
///
/// This is the one place the per-channel divisor is applied. The wire carries
/// the raw word, so nothing downstream of the codec depends on this function
/// being called — it exists so the divisor mapping is a tested property of the
/// port rather than a comment. `None` for a channel index outside the table.
pub fn channel_display_value(channel: usize, raw: i64) -> Option<f64> {
    let spec = CHANNELS.get(channel)?;
    Some(raw as f64 / spec.display_divisor as f64)
}

pub fn schema() -> Schema {
    let mut fields = Vec::with_capacity(1 + CHANNEL_COUNT);

    // The fold's key, and its whole key: `fresh` is keyed by `owner_tag_db_id`
    // alone (`world_fold.rs::fold_air_loss_stats`), with a duplicate rejecting the payload
    // rather than overwriting.
    //
    // `ContentId`: a country tag-db id is an id whose meaning a content manifest
    // must resolve — country 16 is whatever country the mod set defines — which
    // is §5's deciding question, and d20 classes `owner` and d62 classes
    // `owner_tag_db_id` the same way. §13's manifest-id assignment rule is still
    // open, so the mapper casts the raw u32; that is recorded in the
    // deliverable, and a ContentId **key** is the heavier retrofit class because
    // key components sort and delta-encode by raw value.
    fields.push(Field::new(field::OWNER_TAG_DB_ID, Kind::ContentId).key(0));

    // The 24 channels, in engine display order.
    //
    // `Monotone`: each is a cumulative loss counter, which §4 makes a *hint*
    // about how the value usually moves and never a validity constraint — and
    // this domain needs that latitude, because the crack reads a yearly
    // cumulative window (`lib.rs:36`, `:56`) whose counters can fall at a year
    // boundary. `Monotone` carries a `u64` varint (`record.rs:383`), which is
    // exact over the whole `0 ..= MAX_CHANNEL_RAW` band the codec admits.
    //
    // The value stored is the engine's RAW word, not the displayed quantity:
    // the fold emits raw, divisor and display side by side
    // (`world_fold.rs::insert_air_loss_stats`) and the display is a pure function of the
    // other two, so carrying it would be a second copy of derived state. See
    // the module docs and [`channel_display_value`].
    for spec in CHANNELS {
        fields.push(Field::new(spec.field_id, Kind::Monotone));
    }

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: COUNTRY_AIR_LOSS_STATS,
            // Full replace each frame (`world_fold.rs::fold_air_loss_stats` assigns the whole
            // fresh map), but the country is a stable identity that persists
            // across hours — the measured capture holds a constant 440 rows an
            // hour with ~25 changing — and only a keyed set has a delta grammar
            // (§7.3).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 32-byte payload header (`lib.rs:181`-`:190`, `:289`-`:296`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub status_flags: u32,
    pub schema_version: u32,
    pub field_count: u32,
    pub row_len: u32,
    /// The header's trailing pad word (`lib.rs:189`). Parsed and reported, never
    /// enforced: neither the codec nor the fold reads it, so requiring it to be
    /// zero would refuse payloads the recorder folds.
    pub reserved: u32,
}

/// Split a d61 DATA payload into its header and its rows slice, applying the
/// codec's **structural** checks in the codec's own order (`lib.rs:282`-`:322`).
/// Per-row checks stay in [`check_codec_row`] and per-row semantics in
/// [`record_from_row`].
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
///
/// Note there is **no `changed_rows` word** in this header, so there is no such
/// invariant to enforce — the fold reads `row_count` and nothing else about
/// cardinality (`world_fold.rs::fold_air_loss_stats`).
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_loss_stats payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        status_flags: u32_at(payload, 12),
        schema_version: u32_at(payload, 16),
        field_count: u32_at(payload, 20),
        row_len: u32_at(payload, 24),
        reserved: u32_at(payload, 28),
    };
    // `lib.rs:297`: the payload declares which grammar it is. Every offset in
    // this module belongs to version 2, so another version is not a payload this
    // mapper may read.
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "air_loss_stats schema_version {} != {SCHEMA_VERSION}",
            header.schema_version
        ));
    }
    // `lib.rs:303`: the CLoopHistory field count the crack is pinned to.
    if header.field_count != EXPECTED_FIELD_COUNT {
        return Err(format!(
            "air_loss_stats field_count {} != {EXPECTED_FIELD_COUNT}",
            header.field_count
        ));
    }
    // `lib.rs:309`: the header also declares its own row stride.
    if header.row_len as usize != ROW_LEN {
        return Err(format!(
            "air_loss_stats row_len {} != {ROW_LEN}",
            header.row_len
        ));
    }
    // Checked arithmetic: `row_count` is arbitrary input from a stream on disk,
    // and `row_count * ROW_LEN` overflows `usize` for a large enough word. The
    // codec saturates instead (`lib.rs:312`), which rejects the same payloads by
    // way of the length comparison; checked multiplication says so directly.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("air_loss_stats row_count {} overflows", header.row_count))?;
    if payload.len() != expected {
        return Err(format!(
            "air_loss_stats len {} != expected {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The per-row checks `decode_payload_rows` applies (`lib.rs:327`-`:344`) — the
/// flags word and the 24 channel bands, and nothing else.
///
/// **These run before the fold ever looks at the status word.**
/// `fold_air_loss_stats` calls `decode_payload_rows` first
/// (`world_fold.rs::fold_air_loss_stats`) and only reaches `handle_compact_failure`
/// (`:15899`) if that decode returned `Ok`, so a row rejected here makes the
/// whole payload an error even in a record whose status marks the domain-hour
/// failed. A caller that classifies records by status must therefore run this
/// over **every** d61 record, including the ones it will exclude — which is why
/// it is public and separate from [`record_from_row`], whose extra rule is
/// fold-level and post-status.
pub fn check_codec_row(row: &[u8]) -> core::result::Result<(), String> {
    check_row_len(row)?;
    // `lib.rs:328`-`:335`. Both halves: an unknown bit means the row is not this
    // grammar, and a missing witness means the collector did not verify what the
    // row asserts. Either one makes the whole payload unfoldable upstream, which
    // is what the schema's omission of the flags word rests on.
    let flags = u32_at(row, 8);
    if flags != REQUIRED_FLAGS {
        return Err(format!(
            "air_loss_stats row flags {flags:#x} != the required {REQUIRED_FLAGS:#x}"
        ));
    }
    // `lib.rs:340`-`:342`: the fail-closed band, mirrored exactly. See the module
    // docs for why mirroring a rejection the fold makes is not gating against it.
    for channel in 0..CHANNEL_COUNT {
        let raw = i64_at(row, channel_offset_in_range(channel));
        if !(0..=MAX_CHANNEL_RAW).contains(&raw) {
            return Err(format!(
                "air_loss_stats channel {} raw {raw} out of range 0..={MAX_CHANNEL_RAW}",
                CHANNELS[channel].label
            ));
        }
    }
    Ok(())
}

/// The key of one compact row: the fold's `owner_tag_db_id`
/// (`world_fold.rs::fold_air_loss_stats`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[owner_tag_db_id_of_row(row)? as u64]))
}

/// The key word, with the fold's identity invariant applied
/// (`world_fold.rs::fold_air_loss_stats`).
pub fn owner_tag_db_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let owner = u32_at(row, 0);
    let country_idx = u32_at(row, 4);
    if owner != country_idx {
        return Err(format!(
            "air_loss_stats owner_tag_db_id {owner} != country_idx {country_idx}"
        ));
    }
    Ok(owner)
}

/// One compact row of the `country_air_loss_stats` table, as the collector emits
/// it today, onto its stored Record. `row` is the raw 208-byte row slice (the
/// caller has already consumed the table header via [`split_payload`]); d61 has
/// exactly one row form, so length selects nothing and a wrong length is
/// rejected. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); d61 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `decode_payload_rows` (`lib.rs:278`) and
/// `fold_air_loss_stats`: a row either of them would reject is an `Err` here,
/// never a coerced value. The codec accumulates every error and reports them
/// together; this returns the first, which is the same accept/reject decision.
/// Nothing in this function can panic on any 208-byte input.
///
/// **Order matters.** [`check_codec_row`]'s rules are the codec's and run before
/// the fold reads the status word; the `owner_tag_db_id == country_idx` identity
/// is the fold's own and runs **after** `handle_compact_failure`
/// (`world_fold.rs:15899`-`:15912`), so it applies only to rows the fold folds.
/// A caller that excludes failure-status records must run [`check_codec_row`] on
/// those and reach this function only for the records it measures — otherwise it
/// refuses a payload the recorder accepts and folds nothing from.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d61 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;

    // Key-only collections are the reason this check exists at all, but a
    // collection with the wrong id would also silently fill this domain's field
    // ids from another domain's row. `record_from_row` refuses instead.
    if collection.collection_id != COUNTRY_AIR_LOSS_STATS {
        return Err(format!(
            "air_loss_stats has no mapping for collection id {}",
            collection.collection_id
        ));
    }

    // The codec's own per-row rules (`check_row_len` runs inside), then the
    // fold's identity invariant. Both, because a row being well-formed is part
    // of the record being well-formed even for a caller that already ran the
    // codec pass.
    check_codec_row(row)?;
    let _ = owner_tag_db_id_of_row(row)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let channel = CHANNELS
            .iter()
            .position(|spec| spec.field_id == f.field_id)
            .ok_or_else(|| {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field
                // id that means something else.
                format!("air_loss_stats has no mapping for field id {}", f.field_id)
            })?;
        // `channel` came from `CHANNELS.iter().position(..)`, so it is a channel
        // index by construction.
        let raw = i64_at(row, channel_offset_in_range(channel));
        // `check_codec_row` has already proved `0 <= raw <= MAX_CHANNEL_RAW`, so
        // the cast is exact and cannot wrap.
        values.push(Value::Unsigned(raw as u64));
    }
    Ok(Record::new(values))
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_loss_stats row length {} != {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach this only after a length check, so the slice is in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Callers reach this only after a length check, so the slice is in bounds.
fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
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

    /// A raw 208-byte row, built the way the lane packs one (`lib.rs:191`-`:199`).
    fn raw_row(owner: u32, country_idx: u32, flags: u32, channels: &[i64; CHANNEL_COUNT]) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&owner.to_le_bytes());
        row.extend_from_slice(&country_idx.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // pad
        for raw in channels {
            row.extend_from_slice(&raw.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// Channel values that are all distinct and none of which is a multiple of
    /// another, so a transposed pair of offsets cannot pass any assertion.
    fn distinct_channels() -> [i64; CHANNEL_COUNT] {
        let mut channels = [0i64; CHANNEL_COUNT];
        for (k, slot) in channels.iter_mut().enumerate() {
            *slot = 1_000 + k as i64 * 7;
        }
        channels
    }

    fn full_row() -> Vec<u8> {
        raw_row(16, 16, REQUIRED_FLAGS, &distinct_channels())
    }

    fn payload_of(rows: &[Vec<u8>], status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        payload.extend_from_slice(&EXPECTED_FIELD_COUNT.to_le_bytes());
        payload.extend_from_slice(&(ROW_LEN as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
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
            .expect("d61 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_country() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys by owner_tag_db_id alone");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag-db id is a mod-set-defined id"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(
            c.fields.len(),
            1 + CHANNEL_COUNT,
            "1 key + 24 channel fields; country_idx and flags are off the wire"
        );
        // Every channel is a Monotone counter, and the field ids are the stable
        // 2..=25 block in display order.
        for (k, spec) in CHANNELS.iter().enumerate() {
            let f = c
                .fields
                .iter()
                .find(|f| f.field_id == spec.field_id)
                .unwrap_or_else(|| panic!("channel {k} is in the schema"));
            assert_eq!(f.kind, Kind::Monotone, "channel {} is a counter", spec.label);
            assert_eq!(spec.field_id, k as u32 + 2, "field id block is 2..=25");
        }
    }

    #[test]
    fn every_channel_carries_its_own_divisor() {
        // THE hazard test. `CHANNEL_DISPLAY_DIVISORS` (`lib.rs:102`-`:127`) is one
        // fixed array holding two different constants, and reading either for the
        // wrong channel is a 100x error that no round-trip assertion can see.
        // Pinned three ways: the exact per-channel expectation, the two-way
        // partition, and the display arithmetic itself.
        let expected: [i64; CHANNEL_COUNT] = [
            BUILDINGS_BOMBED_DISPLAY_SCALE, // 0  our_buildings_bombed
            FIELD_SCALE,                    // 1  our_troops_bombed
            FIELD_SCALE,                    // 2  our_ships_bombed
            FIELD_SCALE,                    // 3  our_fighters_lost
            FIELD_SCALE,                    // 4  our_bombers_lost
            FIELD_SCALE,                    // 5  our_cas_lost
            FIELD_SCALE,                    // 6  our_transports_lost
            FIELD_SCALE,                    // 7  our_bombers_disrupted
            FIELD_SCALE,                    // 8  our_trains_lost
            FIELD_SCALE,                    // 9  our_trucks_lost
            FIELD_SCALE,                    // 10 our_railway_guns_lost
            FIELD_SCALE,                    // 11 our_missiles_lost
            BUILDINGS_BOMBED_DISPLAY_SCALE, // 12 enemy_buildings_bombed
            FIELD_SCALE,                    // 13 enemy_troops_bombed
            FIELD_SCALE,                    // 14 enemy_ships_sunk
            FIELD_SCALE,                    // 15 enemy_fighters_lost
            FIELD_SCALE,                    // 16 enemy_bombers_lost
            FIELD_SCALE,                    // 17 enemy_cas_lost
            FIELD_SCALE,                    // 18 enemy_transports_lost
            FIELD_SCALE,                    // 19 enemy_bombers_disrupted
            FIELD_SCALE,                    // 20 enemy_trains_lost
            FIELD_SCALE,                    // 21 enemy_trucks_lost
            FIELD_SCALE,                    // 22 enemy_railway_guns_lost
            FIELD_SCALE,                    // 23 enemy_missiles_lost
        ];
        for (k, spec) in CHANNELS.iter().enumerate() {
            assert_eq!(
                spec.display_divisor, expected[k],
                "channel {k} ({}) has the wrong display divisor",
                spec.label
            );
            // The display arithmetic, per channel: one whole unit of the thing
            // the channel counts is exactly its own divisor in raw words.
            assert_eq!(
                channel_display_value(k, expected[k]),
                Some(1.0),
                "channel {k} ({})",
                spec.label
            );
            assert_eq!(channel_display_value(k, 0), Some(0.0));
        }
        // The partition, stated the way the lane states it (`lib.rs:1275`-`:1280`):
        // exactly channels 0 and 12 are the thousand-scaled ones.
        for (k, spec) in CHANNELS.iter().enumerate() {
            let expected_divisor = if k == 0 || k == 12 {
                BUILDINGS_BOMBED_DISPLAY_SCALE
            } else {
                FIELD_SCALE
            };
            assert_eq!(spec.display_divisor, expected_divisor, "channel {k}");
        }
        assert_eq!(CHANNELS[0].label, "our_buildings_bombed");
        assert_eq!(CHANNELS[12].label, "enemy_buildings_bombed");
        assert_ne!(BUILDINGS_BOMBED_DISPLAY_SCALE, FIELD_SCALE);
        // And the two scales are not interchangeable: reading the wrong one is a
        // 100x error, which is what this test exists to catch.
        assert_eq!(
            channel_display_value(0, 100_000),
            Some(100.0),
            "a buildings channel read at FIELD_SCALE would say 1.0"
        );
        assert_eq!(
            channel_display_value(1, 100_000),
            Some(1.0),
            "a plane channel read at BUILDINGS scale would say 100.0"
        );
        assert_eq!(channel_display_value(CHANNEL_COUNT, 0), None);
    }

    #[test]
    fn the_channel_table_matches_the_lanes_pinned_order() {
        // The aggregate-history indices and labels, verbatim from
        // `PINNED_CHANNELS` (`lib.rs:71`-`:96`). A reordering here would silently
        // relabel every channel while every other test still passed.
        let indices: Vec<u32> = CHANNELS.iter().map(|c| c.aggregate_index).collect();
        assert_eq!(
            indices,
            vec![0, 1, 2, 3, 10, 17, 24, 39, 36, 37, 38, 40, 41, 42, 43, 44, 51, 58, 65, 80, 77, 78, 79, 81]
        );
        let labels: Vec<&str> = CHANNELS.iter().map(|c| c.label).collect();
        assert_eq!(
            labels,
            vec![
                "our_buildings_bombed",
                "our_troops_bombed",
                "our_ships_bombed",
                "our_fighters_lost",
                "our_bombers_lost",
                "our_cas_lost",
                "our_transports_lost",
                "our_bombers_disrupted",
                "our_trains_lost",
                "our_trucks_lost",
                "our_railway_guns_lost",
                "our_missiles_lost",
                "enemy_buildings_bombed",
                "enemy_troops_bombed",
                "enemy_ships_sunk",
                "enemy_fighters_lost",
                "enemy_bombers_lost",
                "enemy_cas_lost",
                "enemy_transports_lost",
                "enemy_bombers_disrupted",
                "enemy_trains_lost",
                "enemy_trucks_lost",
                "enemy_railway_guns_lost",
                "enemy_missiles_lost",
            ]
        );
        // Field ids are unique and so are the aggregate indices and labels.
        let mut ids: Vec<u32> = CHANNELS.iter().map(|c| c.field_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CHANNEL_COUNT);
        let mut sorted_indices = indices.clone();
        sorted_indices.sort_unstable();
        sorted_indices.dedup();
        assert_eq!(sorted_indices.len(), CHANNEL_COUNT);
        let mut sorted_labels = labels.clone();
        sorted_labels.sort_unstable();
        sorted_labels.dedup();
        assert_eq!(sorted_labels.len(), CHANNEL_COUNT);
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // Step 8 case 3's capability list is empty for this collection, so there
        // are no per-capability raw-byte cases to write, and this test records
        // why instead of inventing them. Every channel is read unconditionally at
        // a fixed offset and range-checked (`lib.rs:336`-`:344`); nothing is
        // gated. A country whose read failed never becomes a row at all — the
        // live derivation returns `Absent` or `Failed` and emits nothing
        // (`lib.rs:531`-`:856`) — which is the absence of a keyed-set member,
        // not an empty field. The two witness flags cannot express an empty
        // state either: a row missing either bit is a rejected payload
        // (`lib.rs:331`-`:335`), so every channel on a row that exists is
        // measured.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field is capturable-but-uncaptured; the row is what goes absent"
        );
    }

    #[test]
    fn no_field_is_continuous_so_none_is_quantized() {
        // Step 8 case 4 does not apply: quantization is declarable on continuous
        // fields only (`schema.rs`), and d61 has none. Every channel is the
        // engine's raw integer word carried in an integer kind — see the module
        // docs for why the displayed (divided) quantity is not what ships. There
        // is therefore no all-`Exact` variant to compare bytes against.
        let c = collection();
        assert!(
            c.fields.iter().all(|f| !matches!(f.kind, Kind::Continuous)),
            "d61 carries no continuous field"
        );
        assert!(c.fields.iter().all(|f| !f.quant.is_quantized()));
        assert!(c
            .fields
            .iter()
            .all(|f| matches!(f.quant, Quant::None | Quant::Exact)));
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d61 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        let c = collection();
        assert!(c.fields.iter().all(|f| f.derived.is_none()));
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
    fn every_channel_word_lands_in_its_own_field() {
        // The offset test (step 8 case 6). Each channel gets a value no other
        // channel has, so any transposition of two of the 24 adjacent 8-byte
        // words fails here — the same failure class as a wrong scale, and the
        // one a round-trip cannot see because both arms agree on the wrong slice.
        let c = collection();
        let mut channels = [0i64; CHANNEL_COUNT];
        for (k, slot) in channels.iter_mut().enumerate() {
            *slot = 100_003 * (k as i64 + 1);
        }
        let row = raw_row(16, 16, REQUIRED_FLAGS, &channels);
        let r = record_from_row(&c, &row, 0).expect("maps");
        for (k, spec) in CHANNELS.iter().enumerate() {
            assert_eq!(
                r.values[index_of(&c, spec.field_id)],
                Value::Unsigned(channels[k] as u64),
                "channel {k} ({}) at row offset {}",
                spec.label,
                channel_offset(k).expect("k is a channel index")
            );
        }
        // The key word, and the offsets it is read from.
        assert_eq!(owner_tag_db_id_of_row(&row).expect("key"), 16);
        assert_eq!(key_from_row(&row).expect("key"), key(&[16]));
        // Channel offsets are exactly the lane's `o + 16 + k * 8` (`lib.rs:338`).
        for k in 0..CHANNEL_COUNT {
            assert_eq!(channel_offset(k), Some(16 + k * 8));
        }
        assert_eq!(
            channel_offset(CHANNEL_COUNT - 1).expect("last channel") + 8,
            ROW_LEN
        );
    }

    #[test]
    fn a_zero_channel_is_a_measured_value_not_an_empty_state() {
        // §3 keeps zero out of the three empty states. A country at peace has
        // genuinely lost nothing, and the lane is explicit that an unreadable
        // value fails the row closed rather than folding to 0 (`lib.rs:26`-`:28`),
        // so a zero on the wire is always a real measurement.
        let c = collection();
        let row = raw_row(1, 1, REQUIRED_FLAGS, &[0i64; CHANNEL_COUNT]);
        let r = record_from_row(&c, &row, 0).expect("maps");
        for spec in CHANNELS.iter() {
            assert_eq!(
                r.values[index_of(&c, spec.field_id)],
                Value::Unsigned(0),
                "{} lost nothing, which is a value",
                spec.label
            );
        }
        // ... and it survives the codec as itself.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, r);
    }

    #[test]
    fn the_whole_admitted_band_reproduces_the_raw_integer_exactly() {
        // The band claim from the module docs, tested rather than asserted. An
        // integer kind has no rounding, so the reproduction is exact at both
        // edges and everywhere the fold admits — there is no d36-style first
        // failing value to find inside the band.
        // `MAX_CHANNEL_RAW` is 1e12, which is below 2^40 — so unlike d36's
        // `Fixed(5)` band there is no float mantissa anywhere near the limit and
        // no first-failing value to find. The probes are the edges, the two
        // scale constants, and a dense-ish sweep of the powers of two the band
        // contains.
        let c = collection();
        let mut probes = vec![
            0i64,
            1,
            999,
            1_000,
            100_000,
            MAX_CHANNEL_RAW - 1,
            MAX_CHANNEL_RAW,
        ];
        for shift in 0..40u32 {
            let value = 1i64 << shift;
            if value <= MAX_CHANNEL_RAW {
                probes.push(value);
                probes.push(value - 1);
            }
        }
        for raw in probes {
            let row = raw_row(3, 3, REQUIRED_FLAGS, &[raw; CHANNEL_COUNT]);
            let r = record_from_row(&c, &row, 0)
                .unwrap_or_else(|e| panic!("raw {raw} is inside the band: {e}"));
            let mut bytes = Vec::new();
            r.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            for spec in CHANNELS.iter() {
                assert_eq!(
                    back.values[index_of(&c, spec.field_id)],
                    Value::Unsigned(raw as u64),
                    "raw {raw} in {}",
                    spec.label
                );
            }
        }
    }

    #[test]
    fn the_mapper_rejects_every_row_the_codec_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 416] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
            assert!(check_codec_row(&row).is_err(), "length {len}");
        }
        // The flags word: the whole 32-bit space in the two directions that
        // matter. Any missing witness and any unknown bit are both rejections
        // (`lib.rs:328`-`:335`), which is what the schema's omission of the
        // field rests on.
        let channels = distinct_channels();
        for bit in 0..32u32 {
            let missing = REQUIRED_FLAGS & !(1 << bit);
            if missing != REQUIRED_FLAGS {
                let row = raw_row(16, 16, missing, &channels);
                assert!(
                    record_from_row(&c, &row, 0).is_err(),
                    "a row missing witness bit {bit} must be rejected"
                );
                assert!(check_codec_row(&row).is_err(), "bit {bit}");
            }
            let extra = REQUIRED_FLAGS | (1 << bit);
            if extra != REQUIRED_FLAGS {
                let row = raw_row(16, 16, extra, &channels);
                assert!(
                    record_from_row(&c, &row, 0).is_err(),
                    "a row carrying unknown bit {bit} must be rejected"
                );
                assert!(check_codec_row(&row).is_err(), "bit {bit}");
            }
        }
        assert_eq!(REQUIRED_FLAGS, 0b11);
        // The channel band, at both edges, on EVERY channel independently — a
        // check applied to only the first word would pass a row whose 24th
        // channel is a torn read.
        for k in 0..CHANNEL_COUNT {
            for (raw, ok) in [
                (i64::MIN, false),
                (-1i64, false),
                (0, true),
                (MAX_CHANNEL_RAW, true),
                (MAX_CHANNEL_RAW + 1, false),
                (i64::MAX, false),
            ] {
                let mut words = channels;
                words[k] = raw;
                let row = raw_row(16, 16, REQUIRED_FLAGS, &words);
                assert_eq!(
                    record_from_row(&c, &row, 0).is_ok(),
                    ok,
                    "channel {k} ({}) raw {raw}",
                    CHANNELS[k].label
                );
                assert_eq!(check_codec_row(&row).is_ok(), ok, "channel {k} raw {raw}");
            }
        }
    }

    #[test]
    fn the_folds_identity_invariant_is_enforced_but_is_not_a_codec_rule() {
        // `world_fold.rs::fold_air_loss_stats` rejects the payload when the two words
        // disagree, which is what lets `country_idx` stay off the wire. It sits
        // AFTER `handle_compact_failure` (`:15899`), so it is a fold rule and not
        // a codec rule — `check_codec_row` must NOT apply it, or a caller running
        // the codec pass over an excluded record would refuse a payload the
        // recorder accepts and folds nothing from.
        let c = collection();
        let channels = distinct_channels();
        for (owner, idx) in [(16u32, 17u32), (0, 1), (1, 0), (440, 439), (u32::MAX, 0)] {
            let row = raw_row(owner, idx, REQUIRED_FLAGS, &channels);
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "owner {owner} != country_idx {idx} must be rejected by the mapper"
            );
            assert!(key_from_row(&row).is_err(), "owner {owner} idx {idx}");
            assert!(
                check_codec_row(&row).is_ok(),
                "owner {owner} != country_idx {idx} is NOT a codec-level rejection"
            );
        }
        // And when they agree, every value of the pair is accepted — including
        // zero, which the fold does not exclude.
        for owner in [0u32, 1, 16, 440, u32::MAX] {
            let row = raw_row(owner, owner, REQUIRED_FLAGS, &channels);
            assert!(record_from_row(&c, &row, 0).is_ok(), "owner {owner}");
            assert_eq!(key_from_row(&row).expect("key"), key(&[owner as u64]));
        }
    }

    #[test]
    fn the_key_extractor_and_the_mapper_agree_on_what_a_valid_row_is() {
        // A harness calls both; if one accepts a row the other rejects, a frame
        // can be half-folded.
        let c = collection();
        let channels = distinct_channels();
        for owner in [0u32, 1, 16, 440] {
            for idx in [0u32, 1, 16, 440] {
                for flags in [0u32, REQUIRED_FLAGS, REQUIRED_FLAGS | 4] {
                    let row = raw_row(owner, idx, flags, &channels);
                    // The key extractor applies the identity invariant but not
                    // the codec's flag rule, so they agree exactly when the flags
                    // are right.
                    if flags == REQUIRED_FLAGS {
                        assert_eq!(
                            key_from_row(&row).is_ok(),
                            record_from_row(&c, &row, 0).is_ok(),
                            "owner {owner} idx {idx} flags {flags:#x}"
                        );
                    } else {
                        assert!(record_from_row(&c, &row, 0).is_err());
                    }
                }
            }
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = collection();
        c.collection_id = 7;
        assert!(record_from_row(&c, &full_row(), 0).is_err());

        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_codecs_structural_rules() {
        let rows = vec![full_row()];
        let payload = payload_of(&rows, 0);
        let (header, body) = split_payload(&payload).expect("well-formed payload");
        // Every header word `split_payload` reads gets its own known-bytes case:
        // a transposed header word mis-slices every row behind it and no
        // round-trip or replay assertion can see that.
        assert_eq!(header.elapsed_wall_ms, 7);
        assert_eq!(header.row_count, 1);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.schema_version, SCHEMA_VERSION);
        assert_eq!(header.field_count, EXPECTED_FIELD_COUNT);
        assert_eq!(header.row_len, ROW_LEN as u32);
        assert_eq!(header.reserved, 0);
        assert_eq!(body, &rows[0][..]);

        // Distinct values in every header word, so a transposition fails.
        let mut distinct = payload.clone();
        distinct[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        distinct[12..16].copy_from_slice(&0x0000_0400u32.to_le_bytes());
        distinct[28..32].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        let (header, _) = split_payload(&distinct).expect("still well-formed");
        assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(header.status_flags, 0x0000_0400);
        assert_eq!(
            header.reserved, 0xDEAD_BEEF,
            "the reserved word is reported, never enforced — the fold never reads it"
        );

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // Length that does not match the row count.
        let mut truncated = payload.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        // Another schema version (`lib.rs:297`) — every one but 2.
        for version in [0u32, 1, 3, u32::MAX] {
            let mut wrong = payload.clone();
            wrong[16..20].copy_from_slice(&version.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "schema_version {version}");
        }
        // Another field count (`lib.rs:303`).
        for count in [0u32, 64, 88, 90, u32::MAX] {
            let mut wrong = payload.clone();
            wrong[20..24].copy_from_slice(&count.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "field_count {count}");
        }
        // Another row stride (`lib.rs:309`).
        for row_len in [0u32, 16, 32, 207, 209, 416] {
            let mut wrong = payload.clone();
            wrong[24..28].copy_from_slice(&row_len.to_le_bytes());
            assert!(split_payload(&wrong).is_err(), "row_len {row_len}");
        }
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = payload.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
        // An empty table is structurally legal.
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
        let payload = payload_of(
            &[
                full_row(),
                raw_row(2, 2, REQUIRED_FLAGS, &[7i64; CHANNEL_COUNT]),
            ],
            0,
        );
        for len in 0..=payload.len() {
            let _ = split_payload(&payload[..len]);
        }
        let c = collection();
        for i in 0..payload.len() {
            let mut corrupt = payload.clone();
            corrupt[i] ^= 0xFF;
            if let Ok((_, body)) = split_payload(&corrupt) {
                for row in body.chunks_exact(ROW_LEN) {
                    let _ = record_from_row(&c, row, 0);
                    let _ = key_from_row(row);
                    let _ = check_codec_row(row);
                }
            }
        }
    }

    #[test]
    fn no_field_is_a_content_id_except_the_country_key() {
        // §13's manifest-id retrofit sweep needs "checked, and here they are"
        // rather than silence. d61 has exactly one ContentId and it is a KEY,
        // which is the heavier retrofit class: key components sort and
        // delta-encode by raw value, so a manifest remap reorders keys and
        // changes the byte layout of every baseline and key list.
        let c = collection();
        let content_ids: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| f.kind == Kind::ContentId)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(content_ids, vec![field::OWNER_TAG_DB_ID]);
        assert!(
            c.fields
                .iter()
                .find(|f| f.field_id == field::OWNER_TAG_DB_ID)
                .expect("present")
                .is_key(),
            "the one ContentId is a key component"
        );
        assert!(
            c.value_fields().all(|f| f.kind == Kind::Monotone),
            "every value field is a Monotone counter"
        );
    }

    #[test]
    fn every_channel_indexed_entry_point_is_total_over_usize() {
        // Round-2 review finding: `channel_offset` computed
        // `CHANNELS_OFFSET + channel * 8` unchecked, so `channel_offset(usize::MAX)`
        // overflowed — a panic under this workspace's overflow checks, and a
        // silent wrap to a plausible in-row offset without them, which would read
        // a real word from the wrong place.
        //
        // Fixed by property, not by the reported input: EVERY public entry point
        // that takes a caller-supplied channel index is swept over the whole
        // `usize` space here, so a future indexed entry point that forgets the
        // bound fails this test rather than the next review.
        let probes = [
            0usize,
            1,
            CHANNEL_COUNT - 1,
            CHANNEL_COUNT,
            CHANNEL_COUNT + 1,
            100,
            usize::MAX / 8,
            usize::MAX / 8 + 1,
            usize::MAX / 2,
            usize::MAX - 1,
            // The reviewer's own failing input.
            usize::MAX,
        ];
        for channel in probes {
            let offset = channel_offset(channel);
            let display = channel_display_value(channel, 1_000);
            if channel < CHANNEL_COUNT {
                assert_eq!(offset, Some(CHANNELS_OFFSET + channel * 8));
                // In range, the offset addresses a whole i64 inside the row.
                let offset = offset.expect("in range");
                assert!(offset + 8 <= ROW_LEN);
                assert!(display.is_some());
            } else {
                assert_eq!(offset, None, "channel {channel} is not a channel");
                assert_eq!(display, None, "channel {channel} is not a channel");
            }
        }
        // And the in-range set is exactly `0..CHANNEL_COUNT`, so the two entry
        // points agree with each other and with the table on where the boundary
        // is.
        for channel in 0..CHANNEL_COUNT + 4 {
            assert_eq!(
                channel_offset(channel).is_some(),
                channel < CHANNEL_COUNT,
                "channel {channel}"
            );
            assert_eq!(
                channel_display_value(channel, 0).is_some(),
                CHANNELS.get(channel).is_some(),
                "channel {channel}"
            );
        }
    }

    #[test]
    fn a_counter_that_falls_is_mapped_not_refused() {
        // §4 makes Monotone a hint and never a validity constraint, and this
        // domain needs the latitude: the crack reads a YEARLY cumulative window
        // (`lib.rs:36`, `EXPECTED_YEARLY_CAPACITY = 5` at `:56`), so a channel
        // can legitimately fall at a year boundary. Nothing in the mapper or the
        // codec may refuse that.
        let c = collection();
        let high = raw_row(9, 9, REQUIRED_FLAGS, &[5_000_000i64; CHANNEL_COUNT]);
        let low = raw_row(9, 9, REQUIRED_FLAGS, &[12i64; CHANNEL_COUNT]);
        let a = record_from_row(&c, &high, 0).expect("maps");
        let b = record_from_row(&c, &low, 0).expect("a fall is a legal transition");
        assert_ne!(a, b);
        for record in [&a, &b] {
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(&back, record);
        }
    }
}
