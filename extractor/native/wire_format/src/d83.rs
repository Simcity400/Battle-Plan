//! d83 `nuke_stockpile_live` — per-country nuclear stockpile containers.
//!
//! Derived from `world_fold.rs:14068`-`:14137`
//! (`fold_nuke_stockpile`) and the payload codec the fold calls, which for this
// @world-fold-authority domain=83 functions=fold_nuke_stockpile lines=14068-14137 sha256=f938de9a0f2da83b112ea3fe8129fe1f28232fcf829cfccba0c6a8d6d5ec317e closure=36c7768027ab42579e60db9ca730a55d1cb8106d855f9f9a3bb24157d1986ec3
//! domain lives **outside** the fold in its own leaf crate:
//! `extractor/native/lanes/nuke_stockpile_live/src/lib.rs` (`encode_payload` at
//! `:293`, `decode_payload` at `:328`, `validate_frame` at `:270`,
//! `validate_snapshot` at `:243`) over the exact-a729 leaf
//! `lanes/nuke_stockpile_live/nuke_stockpile_exact.rs` (`NUKE_TYPE_COUNT` at
//! `:19`, `built_count_from_raw` at `:99`-`:106`).
//! `extractor/documents/domains/d83_nuke_stockpile_live.md` states the row shape
//! this module mirrors — `tag_db_id -> {container_capacity, [{nuke_type_raw,
//! amount_raw, nukes_ready}]}` — and the oracle used in the tests below
//! (`amount_raw = 8000, nukes_ready = 0` projects **zero** built nukes).
//!
//! **Payload grammar** (`lib.rs:293`-`:326`, mirrored by `decode_payload`):
//! a 164-byte header, then — only when the header's snapshot byte is set — a
//! 12-byte snapshot prologue and `country_count` fixed 60-byte country blocks.
//!
//! | offset | width | header field | notes |
//! |---|---|---|---|
//! | `+0`   | u32   | `magic`      | `NUK1`, `lib.rs:24`; any other value rejected (`:330`) |
//! | `+4`   | u32   | `schema_version` | `2`, `lib.rs:25`; pins the grammar below (`:330`) |
//! | `+8`   | u32   | `authority`  | `0` unavailable / `1` exact-a729 receipt (`:26`-`:27`, `:79`-`:85`) |
//! | `+12`  | u32   | `status_flags` | the compact status word |
//! | `+16`  | u64   | `elapsed_wall_ms` | frame-level, not state |
//! | `+24`  | u64   | `frame_hour` | frame-level, not state |
//! | `+32`  | 32 B  | `executable_sha256` | constant; mismatch rejected (`:337`) |
//! | `+64`  | 32 B  | `receipt_sha256` | constant per authority state (`:282`, `:275`) |
//! | `+96`  | 32 B  | `container_constructor_manifest_sha256` | constant; mismatch rejected (`:342`) |
//! | `+128` | 32 B  | `cnuke_vtable_artifact_sha256` | constant; mismatch rejected (`:345`) |
//! | `+160` | u8    | `snapshot_present` | `0`/`1` only (`:439`-`:443`) |
//! | `+161` | 3 B   | reserved | must be zero (`:349`) |
//!
//! Snapshot prologue: `source_sequence` u64 at `+164`, `country_count` u32 at
//! `+172` (`lib.rs:311`-`:312`). Country block, 60 bytes (`lib.rs:313`-`:323`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`          | raw | none (key 0; zero/duplicate rejected, `lib.rs:249`) |
//! | `+4`  | i32 | `container_capacity` | raw | none (`>= 2`, `lib.rs:252`) |
//! | `+8`  | u32 | `entry_count`        | —   | — (exactly `2`, `lib.rs:255`, `:363`) |
//! | `+12` | 24 B × 2 | entry rows      |     | |
//!
//! Entry row, 24 bytes (`lib.rs:317`-`:323`, `:367`-`:382`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `nuke_type_raw` | raw | none (key 1; must equal its own index, `lib.rs:258`-`:265`) |
//! | `+4`  | u32 | reserved        | —   | must be zero (`lib.rs:369`) |
//! | `+8`  | i64 | `amount_raw`    | **raw, and see below** | none |
//! | `+16` | u32 | `nukes_ready`   | raw | none |
//! | `+20` | u32 | reserved        | —   | must be zero (`lib.rs:374`) |
//!
//! **`built_count_from_raw` is not a scale, and the derived count is not a
//! field.** The fold emits both `amount_raw` and `amount`
//! (`world_fold.rs::fold_nuke_stockpile`), where `amount` is
//! `built_count_from_raw(amount_raw)`: a pure function of a field this schema
//! already carries, so it is a derived value and not state (recipe step 2). Its
//! arithmetic is `raw.wrapping_mul(100_000).wrapping_div(10_000_000_000_000)` —
//! net ÷10⁸, **not** `FIXED_SCALE`'s ÷10⁵, so dividing `amount_raw` by
//! `FIXED_SCALE` would be wrong by 1000× — with a non-finite branch returning
//! the engine's `0xa7c5` sentinel (`nuke_stockpile_exact.rs:99`-`:106`) and a
//! wrapping multiply that garbles any `|raw| > i64::MAX / 100_000`. Carrying the
//! raw i64 keeps that whole hazard off the wire: the sentinel is never computed,
//! no divisor is chosen here, and the one interpretation of the law stays in the
//! leaf module that exists to be the only one. This module deliberately does
//! **not** mirror `built_count_from_raw` — the leaf's own header says it is
//! dependency-free precisely so no second interpretation exists.
//!
//! `nukes_total` and `nukes_ready_total` (`world_fold.rs::fold_nuke_stockpile`) are
//! likewise per-country folds over the entries this schema carries, so they are
//! derived and unmapped.
//!
//! **The four 32-byte header hashes are domain-level authority evidence and are
//! not on the wire.** Four reasons, each sufficient:
//! 1. None of them varies with a country or an entry, so there is no collection
//!    whose rows they could be fields of. §3's record grammar is per row; a
//!    frame-level scalar has no slot, exactly as d63's `elapsed_wall_ms` and
//!    d62's `total_settings_nodes` have none.
//! 2. All four are compile-time constants on **every** payload this schema can
//!    describe. `decode_payload` rejects any payload whose executable,
//!    container-constructor or CNuke-vtable blob differs from the lane's
//!    constant (`lib.rs:337`-`:347`), and `validate_frame` forces
//!    `receipt_sha256` to `WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256` on every
//!    receipted frame and to zero on every unavailable one (`lib.rs:275`,
//!    `:282`). That is d62's constant-flags-word case: state with no state.
//!    Corroborated empirically — across all 903 d83 DATA records of
//!    `runs/batch1/deepwar_t1/collector_stream.bin` the receipt blob is the
//!    producer/consumer constant in every one.
//! 3. §4 has no kind for a 32-byte blob (`schema.rs:12`-`:20`), so carrying one
//!    is a `WIRE_FORMAT.md` §13 change, not a schema-side choice.
//! 4. The invariant the omission rests on is **enforced**, not assumed:
//!    [`split_payload`] checks all four exactly as the codec does, so a payload
//!    from another executable or another authority artifact is refused here too.
//!
//! `source_sequence` is frame-level for the same reason as (1). It is also
//! derivable: the collector sets it to `frame_hour + 1`
//! (`collector/src/attach_log.rs:32083`-`:32086`), and the measurement harness
//! reports that it holds on every record of the real capture.
//!
//! **No field can be absent, null or unknown, and the schema declares no
//! capability.** There is no gate anywhere in this payload: every word is read
//! unconditionally and range-rejected (`lib.rs:352`-`:384`, `:243`-`:268`). A
//! failed read never produces a row with a hole — `read_country_nukes` returns
//! `Err` for the whole country (`nuke_stockpile_exact.rs:111`-`:202`),
//! `read_snapshot_from_country_roots` propagates it (`lib.rs:178`-`:185`), and
//! the collector converts any such error into a rowless
//! `COMPACT_STATUS_SOURCE_UNAVAILABLE` frame with **no snapshot at all**
//! (`attach_log.rs:32117`-`:32133`), which the fold treats as a failed
//! domain-hour that folds nothing (`world_fold.rs::fold_nuke_stockpile`). That is an
//! hour with no d83 state, not a row with an empty field — §3's four states are
//! per field. Declaring a capability no mapper can emit would cost real bits (§3
//! sizes the null and unknown bitmaps by the fields that declare them) and
//! assert a state that cannot occur.
//!
//! **`entry_count` is enforced state and is not a field.** The codec rejects any
//! country whose entry count is not `NUKE_TYPE_COUNT` on encode (`lib.rs:255`)
//! and on decode (`lib.rs:363`), and the exact leaf rejects it at read time
//! (`nuke_stockpile_exact.rs:123`). It is therefore constant over every row this
//! schema can describe — d62's flags-word case again — and [`split_payload`] and
//! [`record_from_country_row`] enforce the invariant the omission rests on.
//! `container_capacity` is **not** in that class and stays a field: the codec
//! constrains it only to `>= NUKE_TYPE_COUNT` (`lib.rs:252`), so its value is
//! measured rather than implied. It is `2` in all 396 417 country rows of the
//! real capture, but empirically constant is not invariantly constant, and
//! dropping a measured value because one campaign never moved it is the fidelity
//! loss §3 exists to prevent.
//!
//! **`nuke_type_raw` is a key component, not a droppable derived word.** The
//! codec enforces `nuke_type_raw == index` (`lib.rs:258`-`:265`), which makes it
//! a function of the entry's *position in the payload array* — and a keyed set
//! has no positions. It is exactly the discriminator that keeps a country's two
//! entries apart, so dropping it would collide the two rows onto one key. §5 puts
//! it at `Unsigned`, not `ContentId`: `NUKE_TYPE_COUNT` is an engine constant of
//! pinned a729 (`nuke_stockpile_exact.rs:19`), no content manifest resolves an
//! ordinal the reader hard-rejects when it is not its own index, and §7.4 admits
//! only `Unsigned`/`Signed`/`ContentId` as key kinds anyway (`schema.rs:29`).
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: a stockpile moves when the country builds or spends a warhead, not on
//! a clock-linear rule. The mapper still takes `frame_tick` so every domain's
//! boundary has one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 83;

pub const COUNTRIES: u32 = 1;
pub const STOCKPILE_ENTRIES: u32 = 2;

/// Index of each collection in [`schema`]`().collections` and in the slices the
/// body encoders take. The order is the schema's and never changes.
pub const COUNTRIES_INDEX: usize = 0;
pub const STOCKPILE_ENTRIES_INDEX: usize = 1;
pub const COLLECTION_COUNT: usize = 2;

/// Field ids are stable and append-only; §11 forbids reusing an id for a
/// *different* field. Ids are unique per collection, and `TAG_DB_ID` names the
/// same concept in both collections, so it is deliberately the same id.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const CONTAINER_CAPACITY: u32 = 2;
    pub const NUKE_TYPE: u32 = 3;
    pub const AMOUNT_RAW: u32 = 4;
    pub const NUKES_READY: u32 = 5;
}

// --- Payload grammar, from `lanes/nuke_stockpile_live/`. ----------------------
//
// The constants are mirrored here rather than imported: `wire_format` is the
// reference implementation of the spec and takes no dependency on a capture
// lane (d62 set this precedent and it is followed, not re-decided). Every one
// carries the line it was taken from, and the version pin below is what makes
// the mirror safe — a collector that changed the grammar must bump
// `SCHEMA_VERSION`, and `split_payload` refuses any other value. See the
// deliverable's open questions.

/// `MAGIC`, `lib.rs:24` — `NUK1` little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"NUK1");
/// `SCHEMA_VERSION`, `lib.rs:25`. `decode_payload` (`:330`) rejects any other
/// value, so this pins the grammar every offset in this module belongs to.
pub const SCHEMA_VERSION: u32 = 2;

/// The fixed header, `lib.rs:296`-`:307`: 4 + 4 + 4 + 4 + 8 + 8 + 4×32 + 1 + 3.
pub const HEADER_LEN: usize = 164;
/// `source_sequence` u64 + `country_count` u32, `lib.rs:311`-`:312`.
pub const SNAPSHOT_PROLOGUE_LEN: usize = 12;
/// `tag_db_id` + `container_capacity` + `entry_count`, `lib.rs:314`-`:316`.
pub const COUNTRY_ROW_LEN: usize = 12;
/// `nuke_type_raw` + reserved + `amount_raw` + `nukes_ready` + reserved,
/// `lib.rs:318`-`:322`.
pub const ENTRY_ROW_LEN: usize = 24;
/// `NUKE_TYPE_COUNT`, `nuke_stockpile_exact.rs:19`. Exactly this many entries
/// per country, enforced on encode (`lib.rs:255`), on decode (`lib.rs:363`) and
/// at read time (`nuke_stockpile_exact.rs:123`).
pub const NUKE_TYPE_COUNT: usize = 2;
/// A country row and its entries are one fixed-size block, because
/// `NUKE_TYPE_COUNT` is invariant.
pub const COUNTRY_BLOCK_LEN: usize = COUNTRY_ROW_LEN + NUKE_TYPE_COUNT * ENTRY_ROW_LEN;
/// `MAX_COUNTRIES`, `lib.rs:28` (`:355`, `:244`).
pub const MAX_COUNTRIES: u32 = 2_048;

/// `AUTHORITY_SOURCE_UNAVAILABLE` / `AUTHORITY_EXACT_A729_RECEIPT`,
/// `lib.rs:26`-`:27`. Any other word is rejected (`lib.rs:83`).
pub const AUTHORITY_SOURCE_UNAVAILABLE: u32 = 0;
pub const AUTHORITY_EXACT_A729_RECEIPT: u32 = 1;

/// `EXECUTABLE_SHA256`, `lib.rs:42`-`:43` — pinned hoi4.exe 1.19.2.0.a729.
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
/// `WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256`, `lib.rs:36`-`:37`. This is the
/// `receipt_sha256` every receipted frame must carry (`lib.rs:282`).
pub const RECEIPT_SHA256: [u8; 32] =
    hex32(b"0968ed85cd6103149ebd51415d6b64d1959bb05d1aff4cbfce9d31e437a6995c");
/// `WAVE5_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256`, `lib.rs:38`-`:39`.
pub const CONTAINER_CONSTRUCTOR_SHA256: [u8; 32] =
    hex32(b"233962bab3a8ca79150977f1f7e77312d1bbd262e89e582d1c01caaf01a89032");
/// `WAVE5_VTABLE_ARTIFACT_SHA256`, `lib.rs:40`-`:41`.
pub const VTABLE_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"cfd9d9e4088b4998398e579a5ebee222534868bad8a4529240b370faf467adbb");

/// `hex32`, `lib.rs:47`-`:63`. A malformed nibble folds to `0xff` there and here
/// alike; the constants above are copied from the lane and the equality tests in
/// this module's test suite are what keep the mirror honest.
const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

pub fn schema() -> Schema {
    use field as f;

    // `countries`: the stockpile-container roster. The key is the country's
    // `tag_db_id` (`world_fold.rs::fold_nuke_stockpile`), the same identity d18 keys its roster
    // by and the one d63 keys its own rows by. `ContentId` because a country tag
    // is content-defined (§5) — a manifest must resolve it for it to keep its
    // meaning under another mod set, which is the deciding question for
    // ContentId-vs-Unsigned. d20 classes its `owner` country the same way, and
    // d62/d63 class `tag_db_id` the same way.
    //
    // `container_capacity` is the engine container's allocated capacity
    // (`nuke_stockpile_exact.rs:91`, read from `COUNTRY_NUKE_CAPACITY_OFF`),
    // emitted raw by the fold (`world_fold.rs::fold_nuke_stockpile`). `Unsigned`, not `Signed`:
    // the source word is `i32` but the codec rejects anything below
    // `NUKE_TYPE_COUNT` (`lib.rs:252`), so the value can never be negative and
    // the mapper checks that bound before it casts. Not `Monotone` either: a
    // container's capacity is an allocation, not a counter, and a fresh
    // container starts small — the monotone hint would be a claim about the data
    // rather than merely an unhelpful one.
    let countries = Collection {
        collection_id: COUNTRIES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::CONTAINER_CAPACITY, Kind::Unsigned),
        ],
    };

    // `stockpile_entries`: the per-country typed warhead rows, flattened onto
    // `(tag_db_id, nuke_type)` exactly as d23 flattens its `(parent, type_id)`
    // entry tables. The fold's own JSON nests them under the country
    // (`world_fold.rs::fold_nuke_stockpile`), and the entry's position in that array
    // is not state — the codec forces it to equal `nuke_type_raw`
    // (`lib.rs:258`-`:265`), so the type IS the identity.
    //
    // `amount_raw` is `Signed`, carried raw, and deliberately not `Continuous`:
    // see the module docs. `i64` is exactly representable in `Signed` and is not
    // in binary64 beyond 2^53, so the integer form is the only lossless one —
    // neither `Quant::Exact` nor a `Fixed(8)` over the ÷10⁸ denominator would
    // survive the whole `i64` domain the codec admits (`lib.rs:372`). The source
    // word is signed and nothing in the codec forbids a negative, so `Signed` is
    // also the faithful kind. Not `Monotone`: a stockpile falls when a warhead
    // is spent.
    //
    // `nukes_ready` is the engine's serialized ready leaf
    // (`nuke_stockpile_exact.rs:176`-`:183`), a `u32` the fold emits raw
    // (`world_fold.rs::fold_nuke_stockpile`). `Unsigned`, not `Monotone`, for the same reason.
    let stockpile_entries = Collection {
        collection_id: STOCKPILE_ENTRIES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::NUKE_TYPE, Kind::Unsigned).key(1),
            Field::new(f::AMOUNT_RAW, Kind::Signed),
            Field::new(f::NUKES_READY, Kind::Unsigned),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        // Full replace each frame — the fold assigns one whole per-hour value and
        // clears it on failure (`world_fold.rs::fold_nuke_stockpile`) — but
        // `tag_db_id` and `(tag_db_id, nuke_type)` are stable identities that
        // persist across hours, which is what a keyed set needs, and only a keyed
        // set has a delta grammar (§7.3).
        collections: vec![countries, stockpile_entries],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// `AuthorityState`, `lib.rs:65`-`:69`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authority {
    SourceUnavailable,
    ExactA729Receipt,
}

/// The 164-byte header plus the snapshot prologue when one is present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub authority: Authority,
    pub status_flags: u32,
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub receipt_sha256: [u8; 32],
    pub has_snapshot: bool,
    /// `0` when there is no snapshot; never `0` when there is (`lib.rs:244`).
    pub source_sequence: u64,
    /// `0` when there is no snapshot.
    pub country_count: u32,
}

/// Split a d83 DATA payload into its header and its country-block region,
/// applying the codec's **structural** checks (`lib.rs:328`-`:405`). Per-row
/// semantics stay in [`record_from_country_row`] and
/// [`record_from_stockpile_entry_row`]; the returned slice is empty when the
/// frame carries no snapshot.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
///
/// **Call order matters.** `fold_nuke_stockpile` decodes the payload
/// (`world_fold.rs::fold_nuke_stockpile`) *before* it consults the status word (`world_fold.rs::fold_nuke_stockpile`), so a
/// caller that excludes failure-status records must run this first and exclude
/// afterwards — a malformed payload is a refused stream whatever its status is.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "nuke_stockpile payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    // `lib.rs:330`: the payload declares which grammar it is.
    let magic = u32_at(payload, 0);
    if magic != MAGIC {
        return Err(format!("nuke_stockpile magic {magic:#x} != NUK1"));
    }
    let schema_version = u32_at(payload, 4);
    if schema_version != SCHEMA_VERSION {
        return Err(format!(
            "nuke_stockpile schema_version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    // `lib.rs:79`-`:85`: an unknown authority word is not this grammar.
    let authority = match u32_at(payload, 8) {
        AUTHORITY_SOURCE_UNAVAILABLE => Authority::SourceUnavailable,
        AUTHORITY_EXACT_A729_RECEIPT => Authority::ExactA729Receipt,
        other => return Err(format!("nuke_stockpile authority state {other} is unknown")),
    };
    let status_flags = u32_at(payload, 12);
    let elapsed_wall_ms = u64_at(payload, 16);
    let frame_hour = u64_at(payload, 24);
    // `lib.rs:337`-`:347`: three of the four blobs are compile-time authority
    // constants and a mismatch means the payload is not from the pinned
    // executable or not from these artifacts. See the module docs for why none of
    // them is a field.
    check_blob(payload, 32, &EXECUTABLE_SHA256, "executable identity")?;
    let mut receipt_sha256 = [0u8; 32];
    receipt_sha256.copy_from_slice(&payload[64..96]);
    check_blob(
        payload,
        96,
        &CONTAINER_CONSTRUCTOR_SHA256,
        "container-constructor authority",
    )?;
    check_blob(payload, 128, &VTABLE_ARTIFACT_SHA256, "CNuke vtable authority")?;
    // `lib.rs:439`-`:443`: the availability byte is a strict bool.
    let has_snapshot = match payload[160] {
        0 => false,
        1 => true,
        other => {
            return Err(format!(
                "nuke_stockpile snapshot-present byte {other} is not 0 or 1"
            ))
        }
    };
    // `lib.rs:349`.
    if payload[161..164] != [0, 0, 0] {
        return Err("nuke_stockpile availability reserved bytes are nonzero".into());
    }

    // `validate_frame`, `lib.rs:270`-`:291`: authority, status, receipt and
    // snapshot presence are one coupled statement, not four independent fields.
    // Mirrored exactly — loosening it here would accept payloads the recorder
    // refuses, and tightening it would refuse payloads the recorder accepts.
    match authority {
        Authority::SourceUnavailable => {
            if status_flags == 0 || receipt_sha256 != [0; 32] || has_snapshot {
                return Err(
                    "nuke_stockpile unavailable frame must be receipt-free and rowless".into(),
                );
            }
        }
        Authority::ExactA729Receipt => {
            if status_flags != 0 || receipt_sha256 != RECEIPT_SHA256 || !has_snapshot {
                return Err("nuke_stockpile receipted frame must be clean and measured".into());
            }
        }
    }

    if !has_snapshot {
        // `lib.rs:308`-`:310`: an unavailable frame is exactly the header.
        if payload.len() != HEADER_LEN {
            return Err(format!(
                "nuke_stockpile rowless payload len {} != {HEADER_LEN}",
                payload.len()
            ));
        }
        return Ok((
            PayloadHeader {
                authority,
                status_flags,
                elapsed_wall_ms,
                frame_hour,
                receipt_sha256,
                has_snapshot,
                source_sequence: 0,
                country_count: 0,
            },
            &payload[HEADER_LEN..],
        ));
    }

    if payload.len() < HEADER_LEN + SNAPSHOT_PROLOGUE_LEN {
        return Err(format!(
            "nuke_stockpile payload {} shorter than its snapshot prologue",
            payload.len()
        ));
    }
    let source_sequence = u64_at(payload, HEADER_LEN);
    let country_count = u32_at(payload, HEADER_LEN + 8);
    // `lib.rs:244`: a snapshot with sequence zero is not a measured snapshot.
    if source_sequence == 0 {
        return Err("nuke_stockpile snapshot has zero source_sequence".into());
    }
    // `lib.rs:355`.
    if country_count > MAX_COUNTRIES {
        return Err(format!(
            "nuke_stockpile country_count {country_count} exceeds {MAX_COUNTRIES}"
        ));
    }
    // Checked arithmetic: `country_count` is arbitrary input from a stream on
    // disk, and `country_count * COUNTRY_BLOCK_LEN` overflows `usize` for a large
    // enough word. `MAX_COUNTRIES` already bounds it, but the bound is a mirrored
    // constant and the multiplication must not depend on it staying small.
    let expected = (country_count as usize)
        .checked_mul(COUNTRY_BLOCK_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN + SNAPSHOT_PROLOGUE_LEN))
        .ok_or_else(|| format!("nuke_stockpile country_count {country_count} overflows"))?;
    // `lib.rs:392`: trailing bytes are a rejected payload, not slack.
    if payload.len() != expected {
        return Err(format!(
            "nuke_stockpile len {} != {expected}",
            payload.len()
        ));
    }
    Ok((
        PayloadHeader {
            authority,
            status_flags,
            elapsed_wall_ms,
            frame_hour,
            receipt_sha256,
            has_snapshot,
            source_sequence,
            country_count,
        },
        &payload[HEADER_LEN + SNAPSHOT_PROLOGUE_LEN..],
    ))
}

fn check_blob(
    payload: &[u8],
    offset: usize,
    expected: &[u8; 32],
    label: &str,
) -> core::result::Result<(), String> {
    if payload[offset..offset + 32] != expected[..] {
        return Err(format!("nuke_stockpile {label} mismatch"));
    }
    Ok(())
}

/// One country row's own words, with the codec's checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CountryRowFields {
    tag_db_id: u32,
    container_capacity: i32,
    entry_count: u32,
}

fn parse_country_row(row: &[u8]) -> core::result::Result<CountryRowFields, String> {
    if row.len() != COUNTRY_ROW_LEN {
        return Err(format!(
            "nuke_stockpile country row length {} != {COUNTRY_ROW_LEN}",
            row.len()
        ));
    }
    let fields = CountryRowFields {
        tag_db_id: u32_at(row, 0),
        container_capacity: i32_at(row, 4),
        entry_count: u32_at(row, 8),
    };
    // `lib.rs:249`.
    if fields.tag_db_id == 0 {
        return Err("nuke_stockpile country row has zero tag_db_id".into());
    }
    // `lib.rs:252` — and the check that makes the `u64` cast below safe.
    if (fields.container_capacity as i64) < NUKE_TYPE_COUNT as i64 {
        return Err(format!(
            "nuke_stockpile country {} capacity {} violates the exact a729 invariant",
            fields.tag_db_id, fields.container_capacity
        ));
    }
    // `lib.rs:255`, `:363`. See the module docs for why this is not a field.
    if fields.entry_count as usize != NUKE_TYPE_COUNT {
        return Err(format!(
            "nuke_stockpile country {} entry_count {} != {NUKE_TYPE_COUNT}",
            fields.tag_db_id, fields.entry_count
        ));
    }
    Ok(fields)
}

/// One entry row's own words, with the codec's checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EntryRowFields {
    nuke_type_raw: u32,
    amount_raw: i64,
    nukes_ready: u32,
}

fn parse_entry_row(entry: &[u8]) -> core::result::Result<EntryRowFields, String> {
    if entry.len() != ENTRY_ROW_LEN {
        return Err(format!(
            "nuke_stockpile entry row length {} != {ENTRY_ROW_LEN}",
            entry.len()
        ));
    }
    // `lib.rs:369` and `:374`: both reserved words must be zero. A payload that
    // put data there is not this grammar.
    if u32_at(entry, 4) != 0 {
        return Err("nuke_stockpile entry identity reserved field is nonzero".into());
    }
    if u32_at(entry, 20) != 0 {
        return Err("nuke_stockpile entry reserved field is nonzero".into());
    }
    Ok(EntryRowFields {
        nuke_type_raw: u32_at(entry, 0),
        amount_raw: i64_at(entry, 8),
        nukes_ready: u32_at(entry, 16),
    })
}

/// Split one 60-byte country block into its country row and its entries region,
/// enforcing the positional invariant `nuke_type_raw == index`
/// (`lib.rs:258`-`:265`) that makes the two entries distinguishable.
///
/// The invariant is structural — it is about where an entry sits, and a keyed set
/// has no positions — so it is checked here, once, during the walk, rather than
/// in the per-entry mapper which sees only the row.
pub fn split_country_block(block: &[u8]) -> core::result::Result<(&[u8], &[u8]), String> {
    if block.len() != COUNTRY_BLOCK_LEN {
        return Err(format!(
            "nuke_stockpile country block length {} != {COUNTRY_BLOCK_LEN}",
            block.len()
        ));
    }
    let row = &block[..COUNTRY_ROW_LEN];
    let fields = parse_country_row(row)?;
    let entries = &block[COUNTRY_ROW_LEN..];
    for (index, entry) in entries.chunks_exact(ENTRY_ROW_LEN).enumerate() {
        let parsed = parse_entry_row(entry)?;
        if parsed.nuke_type_raw as usize != index {
            return Err(format!(
                "nuke_stockpile country {} entry {index} has nuke_type_raw {}",
                fields.tag_db_id, parsed.nuke_type_raw
            ));
        }
    }
    Ok((row, entries))
}

/// The key of one country row: its `tag_db_id` (`world_fold.rs::fold_nuke_stockpile`).
pub fn key_from_country_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[tag_db_id_of_row(row)? as u64]))
}

/// `tag_db_id` with the codec's zero check (`lib.rs:249`).
pub fn tag_db_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    Ok(parse_country_row(row)?.tag_db_id)
}

/// The key of one flattened stockpile entry: `(tag_db_id, nuke_type_raw)`.
pub fn key_from_stockpile_entry_row(
    tag_db_id: u32,
    entry: &[u8],
) -> core::result::Result<Key, String> {
    Ok(key(&[
        tag_db_id as u64,
        parse_entry_row(entry)?.nuke_type_raw as u64,
    ]))
}

/// One compact row of the `countries` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 12-byte country row (the caller has
/// already consumed the payload header via [`split_payload`] and the block via
/// [`split_country_block`]); d83 has exactly one country row form, so length
/// selects nothing and a wrong length is rejected. `frame_tick` is §6's
/// derived-field input (stored = v + c * (frame_tick >> u)); d83 declares no
/// derived field, so it is taken and ignored — every domain's mapper has the
/// same shape.
///
/// Fail-closed throughout, mirroring the codec: a row the codec would reject is
/// an `Err` here, never a coerced value. Nothing in this function can panic on
/// any input.
pub fn record_from_country_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let fields = parse_country_row(row)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            // `parse_country_row` has already rejected anything below
            // `NUKE_TYPE_COUNT`, so this cast cannot turn a negative word into a
            // huge unsigned one.
            field::CONTAINER_CAPACITY => Value::Unsigned(fields.container_capacity as u64),
            other => return Err(no_mapping("countries", other)),
        });
    }
    Ok(Record::new(values))
}

/// One 24-byte entry row onto its stored `stockpile_entries` Record.
///
/// **`amount_raw` is carried raw and unscaled.** The fold's `amount`
/// (`world_fold.rs::fold_nuke_stockpile`) is `built_count_from_raw(amount_raw)`, a ÷10⁸ integer
/// law with a `0xa7c5` sentinel, not a scale factor — see the module docs.
pub fn record_from_stockpile_entry_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let fields = parse_entry_row(entry)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::AMOUNT_RAW => Value::Signed(fields.amount_raw),
            field::NUKES_READY => Value::Unsigned(fields.nukes_ready as u64),
            other => return Err(no_mapping("stockpile_entries", other)),
        });
    }
    Ok(Record::new(values))
}

/// A collection this mapper does not know how to fill. Filling it with a
/// placeholder would publish invented data under a field id that means something
/// else.
fn no_mapping(table: &str, field_id: u32) -> String {
    format!("nuke_stockpile {table} has no mapping for field id {field_id}")
}

/// One hour of d83 state: the two collections, in schema order.
///
/// d83 is full-replace — the fold assigns one whole per-hour value
/// (`world_fold.rs::fold_nuke_stockpile`) — so one usable record *is* one state and there is no
/// accumulator to carry across hours. The walk still lives here rather than in
/// the measurement harness so the tests and the harness share one derivation of
/// the payload's nesting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NukeStockpileState {
    pub countries: KeyedState,
    pub entries: KeyedState,
}

impl NukeStockpileState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The two collections in schema order, for `encode_baseline`/`encode_delta`.
    pub fn as_slice(&self) -> [KeyedState; COLLECTION_COUNT] {
        [self.countries.clone(), self.entries.clone()]
    }

    pub fn get(&self, index: usize) -> &KeyedState {
        match index {
            COUNTRIES_INDEX => &self.countries,
            _ => &self.entries,
        }
    }

    pub fn rows(&self) -> usize {
        self.countries.len() + self.entries.len()
    }
}

/// Fold one clean d83 payload body into its hour's state, exactly as
/// `fold_nuke_stockpile` does (`world_fold.rs::fold_nuke_stockpile`).
///
/// `header` and `body` come from [`split_payload`]. **The caller applies the
/// failure-status exclusion first**: `handle_compact_failure`
/// (`world_fold.rs:11461`) clears the hour and folds nothing before the two
/// checks reproduced here, so calling this on a failure-status record would
/// report an error for a record the recorder simply drops.
pub fn fold_payload(
    schema: &Schema,
    header: &PayloadHeader,
    body: &[u8],
    frame_tick: u64,
) -> core::result::Result<NukeStockpileState, String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "nuke_stockpile schema has {} collections, expected {COLLECTION_COUNT}",
            schema.collections.len()
        ));
    }
    // `world_fold.rs::fold_nuke_stockpile`: a clean payload without the receipt is an
    // error, not an hour to drop.
    if header.authority != Authority::ExactA729Receipt {
        return Err("clean nuke_stockpile payload lacks receipted authority".into());
    }
    // `world_fold.rs::fold_nuke_stockpile`.
    if !header.has_snapshot {
        return Err("nuke_stockpile clean payload omitted authenticated snapshot".into());
    }
    let countries_collection = &schema.collections[COUNTRIES_INDEX];
    let entries_collection = &schema.collections[STOCKPILE_ENTRIES_INDEX];
    let mut state = NukeStockpileState::new();
    let mut blocks = 0usize;
    for block in body.chunks_exact(COUNTRY_BLOCK_LEN) {
        blocks += 1;
        let (row, entries) = split_country_block(block)?;
        let tag_db_id = tag_db_id_of_row(row)?;
        let country_key = key_from_country_row(row)?;
        let record = record_from_country_row(countries_collection, row, frame_tick)?;
        // `lib.rs:249`: a duplicated country identity is a rejected snapshot,
        // never a last-writer-wins overwrite.
        if state.countries.insert(country_key, record).is_some() {
            return Err(format!(
                "nuke_stockpile snapshot repeats country {tag_db_id}"
            ));
        }
        for entry in entries.chunks_exact(ENTRY_ROW_LEN) {
            let entry_key = key_from_stockpile_entry_row(tag_db_id, entry)?;
            let entry_record =
                record_from_stockpile_entry_row(entries_collection, entry, frame_tick)?;
            // Unreachable while `split_country_block` enforces
            // `nuke_type_raw == index`, which makes the two types distinct — but
            // the insert is where a silent overwrite would happen, so the check
            // belongs here rather than in a comment.
            if state.entries.insert(entry_key, entry_record).is_some() {
                return Err(format!(
                    "nuke_stockpile snapshot repeats a nuke type for country {tag_db_id}"
                ));
            }
        }
    }
    // `split_payload` has already made the length exact, so this can only differ
    // if the two disagree — which is precisely the drift worth failing on.
    if blocks != header.country_count as usize {
        return Err(format!(
            "nuke_stockpile body carries {blocks} country blocks, header says {}",
            header.country_count
        ));
    }
    Ok(state)
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

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

    fn countries() -> Collection {
        schema().collections[COUNTRIES_INDEX].clone()
    }

    fn entries() -> Collection {
        schema().collections[STOCKPILE_ENTRIES_INDEX].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// One 12-byte country row, packed the way `encode_payload` packs one
    /// (`lib.rs:314`-`:316`).
    fn country_row(tag_db_id: u32, capacity: i32, entry_count: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(COUNTRY_ROW_LEN);
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&capacity.to_le_bytes());
        row.extend_from_slice(&entry_count.to_le_bytes());
        assert_eq!(row.len(), COUNTRY_ROW_LEN);
        row
    }

    /// One 24-byte entry row (`lib.rs:318`-`:322`).
    fn entry_row(nuke_type: u32, amount_raw: i64, nukes_ready: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ENTRY_ROW_LEN);
        row.extend_from_slice(&nuke_type.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        row.extend_from_slice(&nukes_ready.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(row.len(), ENTRY_ROW_LEN);
        row
    }

    fn country_block(tag_db_id: u32, capacity: i32, entries: [(i64, u32); NUKE_TYPE_COUNT]) -> Vec<u8> {
        let mut block = country_row(tag_db_id, capacity, NUKE_TYPE_COUNT as u32);
        for (index, (amount_raw, ready)) in entries.into_iter().enumerate() {
            block.extend_from_slice(&entry_row(index as u32, amount_raw, ready));
        }
        assert_eq!(block.len(), COUNTRY_BLOCK_LEN);
        block
    }

    /// A whole receipted payload carrying the given country blocks.
    fn payload(source_sequence: u64, blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&MAGIC.to_le_bytes());
        p.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        p.extend_from_slice(&AUTHORITY_EXACT_A729_RECEIPT.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&900u64.to_le_bytes()); // frame_hour
        p.extend_from_slice(&EXECUTABLE_SHA256);
        p.extend_from_slice(&RECEIPT_SHA256);
        p.extend_from_slice(&CONTAINER_CONSTRUCTOR_SHA256);
        p.extend_from_slice(&VTABLE_ARTIFACT_SHA256);
        p.push(1);
        p.extend_from_slice(&[0; 3]);
        assert_eq!(p.len(), HEADER_LEN);
        p.extend_from_slice(&source_sequence.to_le_bytes());
        p.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
        for block in blocks {
            p.extend_from_slice(block);
        }
        p
    }

    /// The rowless frame the collector emits when the source fails
    /// (`lib.rs:228`-`:241`, `attach_log.rs:32117`-`:32126`).
    fn unavailable_payload(status_flags: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&MAGIC.to_le_bytes());
        p.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        p.extend_from_slice(&AUTHORITY_SOURCE_UNAVAILABLE.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&900u64.to_le_bytes());
        p.extend_from_slice(&EXECUTABLE_SHA256);
        p.extend_from_slice(&[0u8; 32]);
        p.extend_from_slice(&CONTAINER_CONSTRUCTOR_SHA256);
        p.extend_from_slice(&VTABLE_ARTIFACT_SHA256);
        p.push(0);
        p.extend_from_slice(&[0; 3]);
        assert_eq!(p.len(), HEADER_LEN);
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d83 schema is well-formed");
    }

    #[test]
    fn the_mirrored_constants_match_the_lane() {
        // The mirror is only safe if it is the same bytes. The lane is not a
        // dependency of this crate, so the hex strings above are the copy and
        // these are the digits they must decode to.
        assert_eq!(MAGIC, u32::from_le_bytes(*b"NUK1"));
        assert_eq!(HEADER_LEN, 4 + 4 + 4 + 4 + 8 + 8 + 4 * 32 + 1 + 3);
        assert_eq!(COUNTRY_BLOCK_LEN, 60);
        assert_eq!(EXECUTABLE_SHA256[0], 0x87);
        assert_eq!(EXECUTABLE_SHA256[31], 0xad);
        assert_eq!(RECEIPT_SHA256[0], 0x09);
        assert_eq!(RECEIPT_SHA256[31], 0x5c);
        assert_eq!(CONTAINER_CONSTRUCTOR_SHA256[0], 0x23);
        assert_eq!(CONTAINER_CONSTRUCTOR_SHA256[31], 0x32);
        assert_eq!(VTABLE_ARTIFACT_SHA256[0], 0xcf);
        assert_eq!(VTABLE_ARTIFACT_SHA256[31], 0xbb);
        // Four distinct artifacts: a copy/paste that collapsed two of them would
        // make `split_payload` accept a payload the codec refuses.
        let blobs = [
            EXECUTABLE_SHA256,
            RECEIPT_SHA256,
            CONTAINER_CONSTRUCTOR_SHA256,
            VTABLE_ARTIFACT_SHA256,
        ];
        for i in 0..blobs.len() {
            for j in (i + 1)..blobs.len() {
                assert_ne!(blobs[i], blobs[j], "blobs {i} and {j} must differ");
            }
            // A bad nibble folds to 0xff in `hex32`; a whole blob of them would
            // mean the hex string was mistyped.
            assert!(blobs[i].iter().any(|b| *b != 0xff));
        }
    }

    #[test]
    fn it_is_two_keyed_sets_keyed_by_country_and_type() {
        let c = countries();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "a country row is keyed by its tag alone");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 2, "1 key + container_capacity");

        let e = entries();
        assert_eq!(e.shape, Shape::KeyedSet);
        let keys = e.key_fields();
        assert_eq!(keys.len(), 2, "an entry is keyed by (country, nuke type)");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(keys[0].kind, Kind::ContentId);
        assert_eq!(keys[1].field_id, field::NUKE_TYPE);
        assert_eq!(
            keys[1].kind,
            Kind::Unsigned,
            "the type ordinal is an engine constant, not mod content"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(e.fields.len(), 4, "2 keys + amount_raw + nukes_ready");
        assert_eq!(
            e.value_fields()
                .find(|f| f.field_id == field::AMOUNT_RAW)
                .expect("amount_raw is in the schema")
                .kind,
            Kind::Signed,
            "the raw fixed-point word is carried as an exact integer"
        );
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // d83 has no gate anywhere: every word of every row is read
        // unconditionally and range-rejected, and a failed read produces a
        // rowless unavailable FRAME rather than a row with a hole
        // (`attach_log.rs:32117`-`:32133`). Absent, Null and Unknown are
        // therefore all unreachable, and §3 sizes the presence/null/unknown
        // bitmaps by the fields that declare them — so declaring one would cost
        // bytes to encode a state that cannot occur.
        for c in [countries(), entries()] {
            assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
            assert!(c.value_fields().all(|f| !f.nullable));
            assert!(c.value_fields().all(|f| !f.unknown_capable));
        }
    }

    #[test]
    fn no_field_is_quantized_because_none_is_continuous() {
        // Step 4's "quantization saves bytes" test does not apply: d83 has no
        // `Kind::Continuous` field at all. The one fixed-point quantity,
        // `amount_raw`, is carried as the raw `i64` the codec transports
        // (`lib.rs:372`) — see the module docs on why the derived built count is
        // not a field and why no divisor belongs on this side of the boundary.
        for c in [countries(), entries()] {
            assert!(c.value_fields().all(|f| !matches!(f.kind, Kind::Continuous)));
            assert!(c.value_fields().all(|f| f.quant == Quant::None));
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; a stockpile moves when a
        // warhead is built or spent, so `frame_tick` never enters a value.
        for c in [countries(), entries()] {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        for (collection, record) in [
            (
                countries(),
                record_from_country_row(&countries(), &country_row(7, 2, 2), 0).expect("maps"),
            ),
            (
                entries(),
                record_from_stockpile_entry_row(&entries(), &entry_row(1, 250_000_000, 3), 0)
                    .expect("maps"),
            ),
        ] {
            let mut bytes = Vec::new();
            record.encode(&collection, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&collection, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
            assert_eq!(back, record, "decode reproduces the record");
            let mut reencoded = Vec::new();
            back.encode(&collection, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
        }
    }

    #[test]
    fn the_mapper_emits_values_at_every_offset() {
        // Every field's offset, field by field, with words that are distinct
        // across the row so a transposition fails: `amount_raw` and `nukes_ready`
        // are adjacent in the entry row and separated in the record.
        let c = countries();
        let r = record_from_country_row(&c, &country_row(11, 9, 2), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CONTAINER_CAPACITY)],
            Value::Unsigned(9),
            "capacity is read at +4 and emitted raw"
        );

        let e = entries();
        let r = record_from_stockpile_entry_row(&e, &entry_row(1, 250_000_000, 7), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&e, field::AMOUNT_RAW)],
            Value::Signed(250_000_000),
            "amount_raw is read at +8 as a signed 64-bit word"
        );
        assert_eq!(
            r.values[index_of(&e, field::NUKES_READY)],
            Value::Unsigned(7),
            "nukes_ready is read at +16"
        );
        // And the key reads `nuke_type_raw` at +0, not one of the value words.
        assert_eq!(
            key_from_stockpile_entry_row(11, &entry_row(1, 250_000_000, 7)).expect("keys"),
            key(&[11, 1])
        );
    }

    #[test]
    fn amount_raw_is_carried_raw_and_never_divided() {
        // The scale test, in the one form this domain admits: `built_count_from_raw`
        // is ÷10⁸ with a sentinel (`nuke_stockpile_exact.rs:99`-`:106`) and
        // `FIXED_SCALE` is ÷10⁵ — an accidental division by either would show up
        // as a changed word here. The dossier's oracle is the first case: a
        // positive `amount_raw` of 8000 is production progress, not a built nuke,
        // and only the raw word preserves that distinction.
        let e = entries();
        for raw in [
            0i64,
            8_000,              // dossier oracle: projects ZERO built nukes
            99_999_999,         // one below the first whole warhead
            100_000_000,        // exactly one
            250_000_000,        // two, with progress toward a third
            -1,
            -100_000_000,
            i64::MIN,           // the codec transports it; the sentinel branch is host-side
            i64::MIN + 1,
            i64::MAX,
            i64::MAX / 100_000, // the edge of `built_count_from_raw`'s wrapping multiply
        ] {
            let r = record_from_stockpile_entry_row(&e, &entry_row(0, raw, 0), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&e, field::AMOUNT_RAW)],
                Value::Signed(raw),
                "amount_raw {raw} must reach the record unchanged"
            );
            // And it survives the codec, so the raw word is what a reader sees.
            let mut bytes = Vec::new();
            r.encode(&e, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&e, &bytes, 0).expect("decodes");
            assert_eq!(back.values[index_of(&e, field::AMOUNT_RAW)], Value::Signed(raw));
        }
        // The whole u32 range of `nukes_ready` likewise rides unscaled.
        for ready in [0u32, 1, 7, u32::MAX] {
            let r = record_from_stockpile_entry_row(&e, &entry_row(0, 0, ready), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&e, field::NUKES_READY)],
                Value::Unsigned(ready as u64)
            );
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // The whole measured capture is this row: capacity 2, both entries zero.
        // §3 keeps zero out of the three empty states, and there is no state here
        // for it to be collapsed into.
        let c = countries();
        let e = entries();
        let r = record_from_country_row(&c, &country_row(1, 2, 2), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CONTAINER_CAPACITY)],
            Value::Unsigned(2)
        );
        let r = record_from_stockpile_entry_row(&e, &entry_row(0, 0, 0), 0).expect("maps");
        assert_eq!(r.values[index_of(&e, field::AMOUNT_RAW)], Value::Signed(0));
        assert_eq!(r.values[index_of(&e, field::NUKES_READY)], Value::Unsigned(0));
        for value in r.values.iter().chain(
            record_from_country_row(&c, &country_row(1, 2, 2), 0)
                .expect("maps")
                .values
                .iter(),
        ) {
            assert!(
                value.carries_value(),
                "no d83 field may reach an empty state: {value:?}"
            );
        }
    }

    #[test]
    fn the_mapper_rejects_every_row_the_codec_rejects() {
        let c = countries();
        let e = entries();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, COUNTRY_ROW_LEN - 1, COUNTRY_ROW_LEN + 1, 60] {
            assert!(
                record_from_country_row(&c, &vec![0u8; len], 0).is_err(),
                "country row length {len} must be rejected"
            );
        }
        for len in [0usize, 1, ENTRY_ROW_LEN - 1, ENTRY_ROW_LEN + 1, 48] {
            assert!(
                record_from_stockpile_entry_row(&e, &vec![0u8; len], 0).is_err(),
                "entry row length {len} must be rejected"
            );
        }
        // Zero tag_db_id (`lib.rs:249`).
        assert!(record_from_country_row(&c, &country_row(0, 2, 2), 0).is_err());
        assert!(key_from_country_row(&country_row(0, 2, 2)).is_err());
        // Capacity below NUKE_TYPE_COUNT, including every negative edge — the
        // check that stops a negative i32 becoming a huge Unsigned.
        for capacity in [i32::MIN, -1, 0, 1] {
            assert!(
                record_from_country_row(&c, &country_row(3, capacity, 2), 0).is_err(),
                "capacity {capacity} violates the exact a729 invariant"
            );
        }
        // And the bound itself is accepted, so it is not off by one.
        assert!(record_from_country_row(&c, &country_row(3, 2, 2), 0).is_ok());
        // entry_count that is not NUKE_TYPE_COUNT (`lib.rs:255`, `:363`).
        for count in [0u32, 1, 3, u32::MAX] {
            assert!(
                record_from_country_row(&c, &country_row(3, 2, count), 0).is_err(),
                "entry_count {count} must be rejected"
            );
        }
        // Either reserved word carrying data (`lib.rs:369`, `:374`).
        for offset in [4usize, 20] {
            let mut row = entry_row(0, 5, 5);
            row[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            assert!(
                record_from_stockpile_entry_row(&e, &row, 0).is_err(),
                "reserved word at +{offset} must be rejected"
            );
            assert!(key_from_stockpile_entry_row(1, &row).is_err());
        }
    }

    #[test]
    fn split_country_block_enforces_the_positional_type_invariant() {
        let good = country_block(4, 2, [(0, 0), (100_000_000, 1)]);
        let (row, entries) = split_country_block(&good).expect("well-formed block");
        assert_eq!(row.len(), COUNTRY_ROW_LEN);
        assert_eq!(entries.len(), NUKE_TYPE_COUNT * ENTRY_ROW_LEN);

        // Every wrong ordinal at every position (`lib.rs:258`-`:265`).
        for index in 0..NUKE_TYPE_COUNT {
            for wrong in [0u32, 1, 2, 7, u32::MAX] {
                if wrong as usize == index {
                    continue;
                }
                let mut block = good.clone();
                let at = COUNTRY_ROW_LEN + index * ENTRY_ROW_LEN;
                block[at..at + 4].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    split_country_block(&block).is_err(),
                    "entry {index} with nuke_type_raw {wrong} must be rejected"
                );
            }
        }
        // Wrong block length in both directions.
        for len in [0usize, COUNTRY_BLOCK_LEN - 1, COUNTRY_BLOCK_LEN + 1] {
            assert!(split_country_block(&vec![0u8; len]).is_err(), "block length {len}");
        }
    }

    #[test]
    fn split_payload_enforces_the_codecs_structural_rules() {
        let blocks = vec![country_block(7, 2, [(0, 0), (100_000_000, 1)])];
        let good = payload(17, &blocks);
        let (header, body) = split_payload(&good).expect("well-formed payload");
        assert_eq!(header.authority, Authority::ExactA729Receipt);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.frame_hour, 900);
        assert_eq!(header.source_sequence, 17);
        assert_eq!(header.country_count, 1);
        assert!(header.has_snapshot);
        assert_eq!(header.receipt_sha256, RECEIPT_SHA256);
        assert_eq!(body, &blocks[0][..]);

        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // Short of the prologue.
        for len in HEADER_LEN..(HEADER_LEN + SNAPSHOT_PROLOGUE_LEN) {
            assert!(split_payload(&good[..len]).is_err(), "prologue len {len}");
        }
        // A length that does not match the country count, in both directions.
        let mut short = good.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err(), "trailing bytes are rejected");

        // Magic and schema version (`lib.rs:330`).
        for (offset, patch) in [(0usize, 0x4b554e31u32), (4, 1), (4, 3)] {
            let mut p = good.clone();
            p[offset..offset + 4].copy_from_slice(&patch.to_le_bytes());
            assert!(split_payload(&p).is_err(), "patch {patch:#x} at +{offset}");
        }
        // An unknown authority word (`lib.rs:83`).
        for authority in [2u32, 3, u32::MAX] {
            let mut p = good.clone();
            p[8..12].copy_from_slice(&authority.to_le_bytes());
            assert!(split_payload(&p).is_err(), "authority {authority}");
        }
        // Each of the four authority blobs, one flipped bit at a time.
        for offset in [32usize, 64, 96, 128] {
            let mut p = good.clone();
            p[offset] ^= 0x01;
            assert!(
                split_payload(&p).is_err(),
                "a changed authority blob at +{offset} must be refused"
            );
        }
        // The snapshot-present byte is a strict bool (`lib.rs:439`).
        for value in [2u8, 3, 0xff] {
            let mut p = good.clone();
            p[160] = value;
            assert!(split_payload(&p).is_err(), "snapshot byte {value}");
        }
        // The three reserved bytes (`lib.rs:349`).
        for offset in 161..164 {
            let mut p = good.clone();
            p[offset] = 1;
            assert!(split_payload(&p).is_err(), "reserved byte at +{offset}");
        }
        // A receipted frame with a non-zero status word (`lib.rs:281`).
        let mut p = good.clone();
        p[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&p).is_err(), "a receipted frame must be clean");
        // A receipted frame carrying a different receipt (`lib.rs:282`).
        let mut p = good.clone();
        p[64] ^= 0x01;
        assert!(split_payload(&p).is_err());
        // source_sequence zero (`lib.rs:244`).
        let mut p = good.clone();
        p[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(split_payload(&p).is_err());
        // A country count past the bound, and one whose byte length overflows
        // `usize` — both must be refused, not wrapped into a plausible length.
        for count in [MAX_COUNTRIES + 1, u32::MAX] {
            let mut p = good.clone();
            p[HEADER_LEN + 8..HEADER_LEN + 12].copy_from_slice(&count.to_le_bytes());
            assert!(split_payload(&p).is_err(), "country_count {count}");
        }
        // A zero-country snapshot is structurally legal (`lib.rs:244` bounds only
        // the sequence and the maximum), so it must parse rather than be refused.
        let empty = payload(17, &[]);
        let (header, body) = split_payload(&empty).expect("a rowless snapshot is well-formed");
        assert_eq!(header.country_count, 0);
        assert!(body.is_empty());
    }

    #[test]
    fn split_payload_accepts_the_rowless_unavailable_frame_and_folds_nothing() {
        // The frame the collector emits for any source failure
        // (`attach_log.rs:32117`-`:32126`): authority 0, non-zero status, zeroed
        // receipt, no snapshot. It must parse — the fold decodes before it looks
        // at the status word (`world_fold.rs::fold_nuke_stockpile`) — and it must not
        // fold, because a clean fold requires the receipt (`world_fold.rs::fold_nuke_stockpile`).
        let schema = schema();
        let p = unavailable_payload(1);
        let (header, body) = split_payload(&p).expect("an unavailable frame is well-formed");
        assert_eq!(header.authority, Authority::SourceUnavailable);
        assert_eq!(header.status_flags, 1);
        assert!(!header.has_snapshot);
        assert_eq!(header.source_sequence, 0);
        assert!(body.is_empty());
        assert!(
            fold_payload(&schema, &header, body, 0).is_err(),
            "an unavailable frame carries no state to fold"
        );

        // The three incoherent combinations `validate_frame` refuses
        // (`lib.rs:272`-`:288`).
        let mut zero_status = unavailable_payload(1);
        zero_status[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&zero_status).is_err(), "unavailable needs a status");
        let mut with_receipt = unavailable_payload(1);
        with_receipt[64..96].copy_from_slice(&RECEIPT_SHA256);
        assert!(split_payload(&with_receipt).is_err(), "unavailable must be receipt-free");
        let mut with_snapshot = unavailable_payload(1);
        with_snapshot[160] = 1;
        with_snapshot.extend_from_slice(&17u64.to_le_bytes());
        with_snapshot.extend_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&with_snapshot).is_err(), "unavailable must be rowless");
    }

    #[test]
    fn fold_payload_builds_both_collections() {
        let schema = schema();
        let blocks = vec![
            country_block(7, 2, [(0, 0), (100_000_000, 1)]),
            country_block(9, 4, [(8_000, 0), (0, 0)]),
        ];
        let p = payload(17, &blocks);
        let (header, body) = split_payload(&p).expect("well-formed");
        let state = fold_payload(&schema, &header, body, 0).expect("folds");
        assert_eq!(state.countries.len(), 2);
        assert_eq!(state.entries.len(), 4, "two typed rows per country");
        assert_eq!(state.rows(), 6);

        let c = countries();
        let e = entries();
        assert_eq!(
            state.countries[&key(&[9])].values[index_of(&c, field::CONTAINER_CAPACITY)],
            Value::Unsigned(4)
        );
        assert_eq!(
            state.entries[&key(&[7, 1])].values[index_of(&e, field::AMOUNT_RAW)],
            Value::Signed(100_000_000)
        );
        assert_eq!(
            state.entries[&key(&[9, 0])].values[index_of(&e, field::AMOUNT_RAW)],
            Value::Signed(8_000),
            "the dossier's progress oracle keeps its raw word"
        );
        assert_eq!(
            state.entries[&key(&[7, 0])].values[index_of(&e, field::NUKES_READY)],
            Value::Unsigned(0)
        );
        assert_eq!(
            state.entries[&key(&[7, 1])].values[index_of(&e, field::NUKES_READY)],
            Value::Unsigned(1)
        );
        assert_eq!(state.as_slice()[COUNTRIES_INDEX].len(), 2);
        assert_eq!(state.as_slice()[STOCKPILE_ENTRIES_INDEX].len(), 4);
        assert_eq!(state.get(COUNTRIES_INDEX).len(), 2);
        assert_eq!(state.get(STOCKPILE_ENTRIES_INDEX).len(), 4);
    }

    #[test]
    fn fold_payload_rejects_a_repeated_country() {
        // `lib.rs:249`: a duplicated identity is a rejected snapshot, never a
        // last-writer-wins overwrite — the two rows would otherwise collapse to
        // one and the hour would silently lose a country.
        let schema = schema();
        let blocks = vec![
            country_block(7, 2, [(0, 0), (0, 0)]),
            country_block(7, 2, [(1, 0), (0, 0)]),
        ];
        let p = payload(17, &blocks);
        let (header, body) = split_payload(&p).expect("structurally well-formed");
        assert!(fold_payload(&schema, &header, body, 0).is_err());
    }

    #[test]
    fn fold_payload_refuses_a_schema_it_cannot_fill() {
        // A caller handing over another domain's schema, or one whose collections
        // gained a field this mapper does not know, must not get records full of
        // placeholders under this domain's field ids.
        let mut schema = schema();
        schema.collections[STOCKPILE_ENTRIES_INDEX]
            .fields
            .push(Field::new(999, Kind::Unsigned));
        let blocks = vec![country_block(7, 2, [(0, 0), (0, 0)])];
        let p = payload(17, &blocks);
        let (header, body) = split_payload(&p).expect("well-formed");
        assert!(fold_payload(&schema, &header, body, 0).is_err());

        let mut one_collection = super::schema();
        one_collection.collections.truncate(1);
        assert!(fold_payload(&one_collection, &header, body, 0).is_err());
    }

    #[test]
    fn the_public_boundary_never_panics_on_arbitrary_bytes() {
        // Every public entry point is total over hostile input: these read bytes
        // that came off a disk, and must return a diagnosis rather than unwind.
        let schema = schema();
        let c = countries();
        let e = entries();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..3_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 260;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            // Half the cases keep a valid magic so the parse gets further in.
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
            }
            if let Ok((header, body)) = split_payload(&bytes) {
                let _ = fold_payload(&schema, &header, body, 0);
            }
            let _ = record_from_country_row(&c, &bytes, 0);
            let _ = record_from_stockpile_entry_row(&e, &bytes, 0);
            let _ = key_from_country_row(&bytes);
            let _ = key_from_stockpile_entry_row(1, &bytes);
            let _ = split_country_block(&bytes);
        }
        // And single-byte corruption of a genuinely well-formed payload.
        let good = payload(17, &[country_block(7, 2, [(0, 0), (100_000_000, 1)])]);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            if let Ok((header, body)) = split_payload(&p) {
                let _ = fold_payload(&schema, &header, body, 0);
            }
        }
    }
}
