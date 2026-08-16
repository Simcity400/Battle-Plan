//! Certified binary seek accelerator for canonical H4CR recordings.
//!
//! H4CR remains the recording authority.  This format stores:
//! * a complete hourly H4CR byte-offset index; and
//! * sparse full world checkpoints followed by bounded binary value deltas.
//!
//! The accelerator is deliberately not a second recording format.  Every
//! reconstructed hour is bound to the exact canonical JSON bytes emitted by
//! the production `replay_decoder` fold.  A reader rejects a cache/index/source
//! identity disagreement before returning a world state.
//!
//! # Byte-exactness
//!
//! The cache stores the world body as a [`CanonValue`] — a JSON tree that keeps
//! the SOURCE SPELLING of every number and string token — rather than as a
//! `serde_json::Value`, which keeps a decoded `f64` and can only spell it back
//! the way `serde_json` would have. Those are not the same document: on capture
//! t26 hour 17032314 the fold emits `"current":189352.52932739258` and a
//! `Value` round trip returns `189352.52932739255`, because `serde_json`'s
//! default number parser is not correctly-rounded. See [`crate::canon_value`]
//! for the full argument, the alternatives that were rejected, and the token
//! classes involved.
//!
//! Nothing here assumes the representation is lossless. The encoder takes each
//! hour's `world_json_sha256` from the SOURCE bytes while the record it writes
//! carries the PARSE, so any token shape nobody anticipated makes the two
//! disagree and fails the seal loudly instead of corrupting a seek quietly.
//!
//! # What certifies a cache
//!
//! The encoder is not self-certifying and does not claim to be. It parses each
//! hour, and it writes records; whether the RECORDS it wrote reconstruct those
//! hours -- and therefore whether the value model preserved their spelling at
//! all -- is proved by
//! [`ReplayCacheReader::verify_every_hour`], which decodes the finished
//! artifact's own bytes and compares every reconstructed hour against its
//! recorded `world_json_sha256`. A caller that builds a cache and skips that
//! call has an unverified cache, whatever `push` returned. The production seal
//! path (`replay_decoder::build_certified_replay_artifacts`) runs it
//! unconditionally before it returns artifacts, and nothing else in this
//! repository publishes a cache.

use crate::canon_value::{
    is_json_number_token, is_json_string_body, is_valid_object_key, key_cmp, CanonValue,
    MAX_RECURSION_DEPTH,
};
use crate::sha256::{to_hex, Sha256};
use crate::zip::MAX_CONTAINER_ENTRY_BYTES;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

pub const HOUR_INDEX_ROLE: &str = "replay_decoder_hour_index";
pub const CHECKPOINT_CACHE_ROLE: &str = "replay_decoder_checkpoint_cache";
pub const HOUR_INDEX_FILE: &str = "replay_decoder/hour_index.h4ix";
pub const CHECKPOINT_CACHE_FILE: &str = "replay_decoder/checkpoints.h4cp";
/// Certifier-produced receipt for how far the fold's PUBLISHED hours reach.
///
/// A sibling of the two artifacts above because it answers a question neither
/// can. The hour index stretches its LAST hour's `h4cr_end_offset` to the
/// decoded length so a seek can replay that hour, so "the index covers byte N"
/// is not the same claim as "the cache holds an hour built from byte N". On a
/// stream ending in a terminal `FRAME_REFUSED` the two disagree by exactly the
/// refused hour, whose bytes reached no published hour at all.
///
/// The receipt is bound to the decoded source identity and to both artifact
/// digests, so a consumer verifies it instead of being told it. Anything that
/// deletes source bytes because the cache holds them must read this.
pub const PUBLISHED_COVERAGE_FILE: &str = "replay_decoder/published_coverage.json";
pub const PUBLISHED_COVERAGE_SCHEMA: &str = "battle-plan-published-coverage-v1";
pub const DEFAULT_CHECKPOINT_INTERVAL_HOURS: u32 = 168;
pub const MAX_CHECKPOINT_INTERVAL_HOURS: u32 = 24 * 31;

const INDEX_MAGIC: &[u8; 4] = b"H4IX";
const CACHE_MAGIC: &[u8; 4] = b"H4CP";
/// Version 4 carries a PER-RECORD digest on every served row, so damage to the
/// packaged H4CP is attributable to the records it actually reached instead of
/// refusing the whole cache (owner ruling 2026-08-12, task #191). Without it
/// the only binding between index and cache was one whole-cache digest: a
/// flipped byte anywhere failed it, and nothing could say which record moved.
///
/// Version 3 carried EXCLUDED-HOUR rows beside the served ones, so a recording
/// whose middle is injured is still a composition rather than a refusal (owner
/// ruling 2026-08-12, task #189). Version 2 carried scalar SPELLINGS and
/// compact integers but had no way to say "this hour is a hole": every row was
/// an hour it served, and a gap between two rows was a structural violation.
/// Version 1 stored `serde_json`-normalised numbers behind fixed 32-bit length
/// prefixes. None is readable here and none ever will be — pre-production
/// carries no backwards compatibility, and an older index can answer neither
/// which hours its recording vouches for nor which of its records survived.
///
/// The H4CP header carries this same number, and deliberately so: an index and
/// a cache from different format generations must never pair, and the per-hour
/// digests only mean anything over records whose framing the index agrees with.
/// Version 7 adds [`PATCH_ARRAY_EDIT`], and version 6 added
/// [`PATCH_NUMBER_DELTA`]. Every recording of an earlier version stops opening,
/// which is what pre-production means: the header carries this number and a
/// reader refuses a cache that does not state it.
const FORMAT_VERSION: u16 = 7;
const RECORD_CHECKPOINT: u8 = 1;
const RECORD_DELTA: u8 = 2;
const CACHE_HEADER_LEN: u64 = 4 + 2 + 2 + 32;
const MAX_STRING_BYTES: usize = 256 * 1024 * 1024;
const MAX_HOURS: usize = 2_000_000;
/// An exclusion reason is PROSE naming one injured hour, not a payload. The
/// bound is deliberately far tighter than [`MAX_STRING_BYTES`]: the longest
/// reason the recorder actually mints is a few hundred bytes, and without a
/// bound of its own an index could carry two million rows of quarter-gigabyte
/// strings. It is enforced at both ends — an encoder cannot write a longer one
/// and a reader will not admit one.
const MAX_EXCLUSION_REASON_BYTES: usize = 4096;

/// Fixed prefix of every cache record: kind, three reserved bytes, the hour,
/// the payload length, and one trailing reserved byte.
const RECORD_HEADER_LEN: u64 = 1 + 3 + 8 + 8 + 1;

/// Largest single cache RECORD a reader will materialize.
///
/// This is the old whole-entry ceiling, re-scoped to the unit a reader now
/// actually materializes, and it is deliberately the SAME number rather than a
/// tighter one. Before this existed a record was bounded by the cache and the
/// cache was bounded by the container's in-RAM cap, so no reader ever admitted
/// a record above 2 GiB; keeping the number identical means the re-scoping
/// introduced no new refusal.
///
/// It stays that number at v4. The old rationale — "no previously sealed
/// container can be refused by it" — retired with the format bump (v4 makes
/// every older H4CP unreadable anyway, which is what pre-production means), and
/// what remains is the reason that never depended on compatibility: the encoder
/// has no per-record ceiling of its own, and a whole-world CHECKPOINT can be
/// arbitrarily large without violating the 256 MiB per-string bound, so any
/// tighter figure would be an unrelated new restriction on how big one game
/// hour may be. A smaller derived figure (512 MiB — twice [`MAX_STRING_BYTES`],
/// a sixteenth of the 8 GiB floor) was considered and rejected for exactly that
/// reason.
///
/// It is not a formality. The cache entry itself is now packed under the 2 TiB
/// streaming ceiling, so without this a corrupted or hostile `cache_record_len`
/// could ask a reader for terabytes. What the reader may hold at once has not
/// changed; only what counts as "one thing".
///
/// For scale: the largest real record is 30,710,425 bytes in t31's 602 and
/// 28,451,685 in t32's 656 — about 70x under the bound.
pub const MAX_CACHE_RECORD_BYTES: u64 = MAX_CONTAINER_ENTRY_BYTES;

/// Bytes read per pass when hashing a file-backed cache.
const CACHE_HASH_CHUNK: usize = 1024 * 1024;

// Binary value tags. Every value has exactly ONE legal encoding, and the
// decoder rejects any other spelling of the same value, so a cache's bytes are
// a function of its content and the cache SHA is stable.
const TAG_NULL: u8 = 0;
const TAG_FALSE: u8 = 1;
const TAG_TRUE: u8 = 2;
/// A number that is not a canonical decimal integer in `INLINE`/`UINT`/`NEGINT`
/// range: varint length + the source token bytes.
const TAG_NUMBER_TOKEN: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_ARRAY: u8 = 5;
const TAG_OBJECT: u8 = 6;
/// A canonical non-negative decimal integer above [`INLINE_UINT_MAX`].
const TAG_UINT: u8 = 7;
/// A canonical negative decimal integer: varint of the magnitude.
const TAG_NEGINT: u8 = 8;
/// `INLINE_UINT_BASE + n` spells the integer `n` in a single byte.
const INLINE_UINT_BASE: u8 = 0x80;
const INLINE_UINT_MAX: u64 = 0xFF - INLINE_UINT_BASE as u64;

const PATCH_KEEP: u8 = 0;
const PATCH_REPLACE: u8 = 1;
const PATCH_OBJECT: u8 = 2;
const PATCH_ARRAY: u8 = 3;
/// "This member is exactly what it last was." One byte.
///
/// THE CADENCE, NOT THE WORLD, IS WHAT MOST DELTA BYTES PAY FOR. A domain that
/// folds out for an hour leaves `null` behind, and when it returns the next
/// hour the old encoder had nothing to diff against -- `null` is structurally
/// incompatible with an object, so the whole subtree was re-emitted in full,
/// however little of it had actually changed. Measured over 32 consecutive
/// t53 hours that re-emission was 44% of every delta byte written.
///
/// A revive re-bases such a member on ITS OWN last non-null value instead of on
/// the null. Both sides derive that base from records they have already walked
/// -- see [`Shadow`] -- so the delta chain stays linear and seek still costs one
/// checkpoint plus its deltas.
const PATCH_REVIVE_SAME: u8 = 4;
/// A revive that still needs a patch on top of the remembered value.
const PATCH_REVIVE_PATCH: u8 = 5;
/// A changed number spelled as the DIFFERENCE from the number it replaces:
/// the new token's fraction-digit count, then a zigzag varint of the change.
///
/// THE MUST-SHIP DOMAINS CANNOT BE DERIVED AWAY, SO THE ONLY LEVER ON THEM IS
/// HOW A CHANGE IS SPELLED. A division's organisation moving from 47.36551 to
/// 47.4 costs a whole fresh decimal token today; what actually changed is 3449
/// hundred-thousandths. Measured over 59 consecutive t53 hours, spelling every
/// eligible number change this way took the sealed delta stream from 160,351 to
/// 141,466 bytes per game hour -- 11.8% -- while moving the RAW stream by 1.0%.
/// That inversion is the point: a fresh decimal token is seven or eight bytes of
/// high-entropy digits that zstd cannot predict, and a small varint is one or
/// two bytes it can. `divisions[]` and `supply.by_province.<id>.divisions[]`
/// are 78% of the saving.
///
/// EXACT, NOT APPROXIMATE. The reconstruction is `old` rescaled to the wider of
/// the two fraction widths, plus the delta, respelled with the NEW token's
/// fraction width -- so 47.4 comes back as `47.4` and never as `47.40000`.
/// Where that respelling is not byte-identical to the token the recording
/// carried, [`number_delta_between`] refuses and the change stays a `Replace`.
const PATCH_NUMBER_DELTA: u8 = 6;
/// Fraction digits a number delta may carry, and the digit ceiling on the
/// tokens it may join. Bounds the rescaling to `i128` and keeps a hostile
/// `fraction_digits` from asking for a power of ten nobody can hold. The widest
/// fraction the fold actually mints is five (`FIXED_SCALE` is 100,000).
const MAX_NUMBER_DELTA_DIGITS: u32 = 30;
/// An array whose LENGTH changed, aligned by a key its own rows carry, as a
/// left-to-right edit script over the old array.
///
/// AN ARRAY THAT GAINS OR LOSES ONE ROW IS RE-EMITTED WHOLE TODAY. `Patch::
/// Array` can only address elements by index, so it is defined only where the
/// two lengths agree; everything else falls to `Replace`, and a replace carries
/// every row of the new array in full. Measured on capture t53 after lane LV,
/// `divisions` cost 10,037 sealed bytes per game hour that way and
/// `supply.by_province.<id>.divisions` another 2,697 -- and because a replace
/// carries VALUES, none of the number deltas inside those rows applies either.
///
/// The script consumes the old array once: [`ARRAY_EDIT_COPY`] takes a run
/// unchanged, [`ARRAY_EDIT_SKIP`] drops one, [`ARRAY_EDIT_PATCH`] carries a
/// nested patch for one, and [`ARRAY_EDIT_INSERT`] carries a row the old array
/// did not have. Rows are matched by a member the encoder DISCOVERS -- one
/// present on every row of both arrays with a scalar value that is unique in
/// each -- so no member name is written into this format and a mod's rows align
/// on whatever identity they carry.
const PATCH_ARRAY_EDIT: u8 = 7;
/// Take the next `n` old elements unchanged. `n >= 1`.
const ARRAY_EDIT_COPY: u64 = 0;
/// Drop the next `n` old elements. `n >= 1`.
const ARRAY_EDIT_SKIP: u64 = 1;
/// Take the next old element with a patch on top.
const ARRAY_EDIT_PATCH: u64 = 2;
/// Emit a value the old array does not have. Consumes no old element.
const ARRAY_EDIT_INSERT: u64 = 3;
/// Smallest encoded edit op: its kind and one varint or value byte.
const MIN_ARRAY_EDIT_OP_BYTES: usize = 1 + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheIdentityMode {
    Legacy,
    IdentityV1,
}

impl CacheIdentityMode {
    fn encode(self) -> u8 {
        match self {
            Self::Legacy => 0,
            Self::IdentityV1 => 1,
        }
    }

    fn decode(value: u8) -> Result<Self, ReplayCacheError> {
        match value {
            0 => Ok(Self::Legacy),
            1 => Ok(Self::IdentityV1),
            _ => Err(ReplayCacheError::new(format!(
                "unknown H4CR identity mode {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayHourEntry {
    hour_index: i64,
    date: String,
    h4cr_start_offset: u64,
    h4cr_end_offset: u64,
    cache_record_offset: u64,
    cache_record_len: u64,
    checkpoint_position: u32,
    delta_steps: u32,
    world_json_sha256: String,
    /// SHA-256 over this hour's CACHE RECORD — its 21-byte header and its
    /// payload, exactly the `[cache_record_offset, +cache_record_len)` window.
    ///
    /// This is the field that makes H4CP damage confinable. `world_json_sha256`
    /// answers "did this hour reconstruct correctly", which can only be asked
    /// after the record decodes; this answers "are these the bytes the seal
    /// wrote", which can be asked of a record that no longer decodes at all,
    /// and asked of every record in one streaming pass.
    ///
    /// Its coverage is total with the tiling the index already proves: the
    /// records abut from [`CACHE_HEADER_LEN`] through `cache_len`, and the
    /// header is fully determined by [`validate_checkpoint_cache_header`], so
    /// every byte of a cache is pinned by an index that carries these.
    cache_record_sha256: String,
}

impl ReplayHourEntry {
    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }
    pub fn date(&self) -> &str {
        &self.date
    }
    pub fn h4cr_start_offset(&self) -> u64 {
        self.h4cr_start_offset
    }
    pub fn h4cr_end_offset(&self) -> u64 {
        self.h4cr_end_offset
    }
    pub fn cache_record_offset(&self) -> u64 {
        self.cache_record_offset
    }
    pub fn cache_record_len(&self) -> u64 {
        self.cache_record_len
    }
    pub fn checkpoint_position(&self) -> u32 {
        self.checkpoint_position
    }
    pub fn delta_steps(&self) -> u32 {
        self.delta_steps
    }
    pub fn world_json_sha256(&self) -> &str {
        &self.world_json_sha256
    }
    /// The digest of this hour's cache record bytes. See the field.
    pub fn cache_record_sha256(&self) -> &str {
        &self.cache_record_sha256
    }
}

/// One game hour the recording does NOT vouch for, and the H4CR bytes it
/// occupies.
///
/// This is what makes an INTERIOR injury representable. Before it existed the
/// index was contiguous by construction — the first row at decoded offset zero,
/// consecutive rows abutting exactly — so an hour dropped out of the middle
/// left a gap the format read as corruption, and the only honest answer was to
/// refuse the whole build (lane GY, residual GY-4).
///
/// A hole row is not a weakening of that law, it is the law's other half: the
/// served rows and the hole rows TOGETHER must still tile `[0, source_len)`
/// exactly, so every source byte is still accounted for — as an hour that is
/// served, or as an hour that is named and refused. What changed is that
/// "accounted for" gained a second, honest spelling.
///
/// `reason` is the exact string the recording's per-hour coverage map gave for
/// this hour. It is never invented here: an injured hour is never guessed, only
/// named, and the name a replay consumer reads out of the format must be the
/// same name the seal published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayExcludedHour {
    hour_index: i64,
    /// Offset of the hour's first H4CR byte. Equal to `h4cr_end_offset` for an
    /// hour that occupies no bytes at all — an hour whose records never reached
    /// the spool is still named, and it is named at the position it would have
    /// occupied.
    h4cr_start_offset: u64,
    h4cr_end_offset: u64,
    reason: String,
}

impl ReplayExcludedHour {
    pub fn new(
        hour_index: i64,
        h4cr_start_offset: u64,
        h4cr_end_offset: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            hour_index,
            h4cr_start_offset,
            h4cr_end_offset,
            reason: reason.into(),
        }
    }

    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }
    pub fn h4cr_start_offset(&self) -> u64 {
        self.h4cr_start_offset
    }
    pub fn h4cr_end_offset(&self) -> u64 {
        self.h4cr_end_offset
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayHourIndex {
    game_id: String,
    schema_version: String,
    identity_mode: CacheIdentityMode,
    source_len: u64,
    source_sha256: String,
    checkpoint_interval_hours: u32,
    cache_len: u64,
    cache_sha256: String,
    hours: Vec<ReplayHourEntry>,
    /// The hours this recording refuses, in hour order. Empty for an
    /// uninjured capture, which is every capture the recorder hopes to make.
    excluded: Vec<ReplayExcludedHour>,
}

impl ReplayHourIndex {
    pub fn game_id(&self) -> &str {
        &self.game_id
    }
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }
    pub fn identity_mode(&self) -> CacheIdentityMode {
        self.identity_mode
    }
    pub fn source_len(&self) -> u64 {
        self.source_len
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn checkpoint_interval_hours(&self) -> u32 {
        self.checkpoint_interval_hours
    }
    pub fn cache_len(&self) -> u64 {
        self.cache_len
    }
    pub fn cache_sha256(&self) -> &str {
        &self.cache_sha256
    }
    pub fn hours(&self) -> &[ReplayHourEntry] {
        &self.hours
    }
    /// The hours this recording names and refuses, in hour order.
    ///
    /// A consumer that asks [`Self::entry`] for one of these gets `None`; this
    /// is where it learns WHY, without having to join against a document
    /// outside the artifact.
    pub fn excluded_hours(&self) -> &[ReplayExcludedHour] {
        &self.excluded
    }
    /// Why this recording refuses `hour_index`, or `None` if it does not refuse
    /// it. `None` is not a claim that the hour is served — ask [`Self::entry`]
    /// for that.
    pub fn exclusion_reason(&self, hour_index: i64) -> Option<&str> {
        self.excluded
            .binary_search_by_key(&hour_index, ReplayExcludedHour::hour_index)
            .ok()
            .map(|position| self.excluded[position].reason())
    }
    pub fn entry(&self, hour_index: i64) -> Option<&ReplayHourEntry> {
        self.hours
            .binary_search_by_key(&hour_index, ReplayHourEntry::hour_index)
            .ok()
            .map(|position| &self.hours[position])
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ReplayCacheError> {
        self.validate()?;
        let mut out = Vec::with_capacity(256 + self.hours.len() * 128);
        out.extend_from_slice(INDEX_MAGIC);
        put_u16(&mut out, FORMAT_VERSION);
        put_u16(&mut out, 0);
        out.push(self.identity_mode.encode());
        out.extend_from_slice(&[0; 3]);
        put_u32(&mut out, self.checkpoint_interval_hours);
        put_u64(&mut out, self.source_len);
        put_u64(&mut out, self.cache_len);
        out.extend_from_slice(&decode_hex32(&self.source_sha256)?);
        out.extend_from_slice(&decode_hex32(&self.cache_sha256)?);
        put_index_string(&mut out, &self.game_id)?;
        put_index_string(&mut out, &self.schema_version)?;
        put_u64(
            &mut out,
            u64::try_from(self.hours.len())
                .map_err(|_| ReplayCacheError::new("hour count does not fit u64"))?,
        );
        for hour in &self.hours {
            put_i64(&mut out, hour.hour_index);
            put_u64(&mut out, hour.h4cr_start_offset);
            put_u64(&mut out, hour.h4cr_end_offset);
            put_u64(&mut out, hour.cache_record_offset);
            put_u64(&mut out, hour.cache_record_len);
            put_u32(&mut out, hour.checkpoint_position);
            put_u32(&mut out, hour.delta_steps);
            out.extend_from_slice(&decode_hex32(&hour.world_json_sha256)?);
            out.extend_from_slice(&decode_hex32(&hour.cache_record_sha256)?);
            put_index_string(&mut out, &hour.date)?;
        }
        put_u64(
            &mut out,
            u64::try_from(self.excluded.len())
                .map_err(|_| ReplayCacheError::new("excluded hour count does not fit u64"))?,
        );
        for hole in &self.excluded {
            put_i64(&mut out, hole.hour_index);
            put_u64(&mut out, hole.h4cr_start_offset);
            put_u64(&mut out, hole.h4cr_end_offset);
            put_index_string(&mut out, &hole.reason)?;
        }
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReplayCacheError> {
        let mut reader = ByteReader::new(bytes);
        if reader.take(4)? != INDEX_MAGIC {
            return Err(ReplayCacheError::new("hour index has invalid H4IX magic"));
        }
        if reader.u16()? != FORMAT_VERSION || reader.u16()? != 0 {
            return Err(ReplayCacheError::new(
                "hour index has unsupported version or flags",
            ));
        }
        let identity_mode = CacheIdentityMode::decode(reader.u8()?)?;
        if reader.take(3)? != [0, 0, 0] {
            return Err(ReplayCacheError::new(
                "hour index reserved identity bytes are nonzero",
            ));
        }
        let checkpoint_interval_hours = reader.u32()?;
        let source_len = reader.u64()?;
        let cache_len = reader.u64()?;
        let source_sha256 = to_hex(reader.array32()?);
        let cache_sha256 = to_hex(reader.array32()?);
        let game_id = reader.index_string()?;
        let schema_version = reader.index_string()?;
        let hour_count = usize::try_from(reader.u64()?)
            .map_err(|_| ReplayCacheError::new("hour count does not fit usize"))?;
        if hour_count > MAX_HOURS {
            return Err(ReplayCacheError::new(format!(
                "hour index count {hour_count} exceeds {MAX_HOURS}"
            )));
        }
        bound_count_by_remaining(hour_count, MIN_HOUR_ENTRY_BYTES, reader.remaining())?;
        let mut hours = Vec::new();
        reserve_exact(&mut hours, hour_count)?;
        for _ in 0..hour_count {
            hours.push(ReplayHourEntry {
                hour_index: reader.i64()?,
                h4cr_start_offset: reader.u64()?,
                h4cr_end_offset: reader.u64()?,
                cache_record_offset: reader.u64()?,
                cache_record_len: reader.u64()?,
                checkpoint_position: reader.u32()?,
                delta_steps: reader.u32()?,
                world_json_sha256: to_hex(reader.array32()?),
                cache_record_sha256: to_hex(reader.array32()?),
                date: reader.index_string()?,
            });
        }
        let excluded_count = usize::try_from(reader.u64()?)
            .map_err(|_| ReplayCacheError::new("excluded hour count does not fit usize"))?;
        if excluded_count > MAX_HOURS {
            return Err(ReplayCacheError::new(format!(
                "hour index excludes {excluded_count} hour(s), which exceeds {MAX_HOURS}"
            )));
        }
        bound_count_by_remaining(excluded_count, MIN_EXCLUDED_ENTRY_BYTES, reader.remaining())?;
        let mut excluded = Vec::new();
        reserve_exact(&mut excluded, excluded_count)?;
        for _ in 0..excluded_count {
            excluded.push(ReplayExcludedHour {
                hour_index: reader.i64()?,
                h4cr_start_offset: reader.u64()?,
                h4cr_end_offset: reader.u64()?,
                // Bounded BEFORE the copy: `validate` would refuse an
                // over-long reason too, but only after every row is resident.
                reason: reader.index_string_bounded(MAX_EXCLUSION_REASON_BYTES)?,
            });
        }
        reader.finish()?;
        let index = Self {
            game_id,
            schema_version,
            identity_mode,
            source_len,
            source_sha256,
            checkpoint_interval_hours,
            cache_len,
            cache_sha256,
            hours,
            excluded,
        };
        index.validate()?;
        Ok(index)
    }

    fn validate(&self) -> Result<(), ReplayCacheError> {
        if self.game_id.trim().is_empty() || self.schema_version.trim().is_empty() {
            return Err(ReplayCacheError::new(
                "hour index game/schema identity is empty",
            ));
        }
        if self.source_len < 4 || self.cache_len < CACHE_HEADER_LEN {
            return Err(ReplayCacheError::new(
                "hour index source/cache length is invalid",
            ));
        }
        decode_hex32(&self.source_sha256)?;
        decode_hex32(&self.cache_sha256)?;
        if !(1..=MAX_CHECKPOINT_INTERVAL_HOURS).contains(&self.checkpoint_interval_hours) {
            return Err(ReplayCacheError::new(format!(
                "checkpoint interval {} is outside 1..={MAX_CHECKPOINT_INTERVAL_HOURS}",
                self.checkpoint_interval_hours
            )));
        }
        if self.hours.is_empty() || self.hours.len() > MAX_HOURS {
            return Err(ReplayCacheError::new(
                "hour index must contain a bounded nonempty hour set",
            ));
        }
        let mut previous_hour = None;
        let mut previous_cache_end = CACHE_HEADER_LEN;
        let mut active_checkpoint = usize::MAX;
        for (position, hour) in self.hours.iter().enumerate() {
            decode_hex32(&hour.world_json_sha256)?;
            decode_hex32(&hour.cache_record_sha256)?;
            if hour.date.trim().is_empty()
                || previous_hour.is_some_and(|value| hour.hour_index <= value)
                || hour.h4cr_start_offset >= hour.h4cr_end_offset
                || hour.h4cr_end_offset > self.source_len
                || hour.cache_record_offset != previous_cache_end
                || hour.cache_record_len < 21
                || hour
                    .cache_record_offset
                    .checked_add(hour.cache_record_len)
                    .is_none_or(|end| end > self.cache_len)
                || usize::try_from(hour.checkpoint_position)
                    .ok()
                    .is_none_or(|checkpoint| checkpoint > position)
                || hour.delta_steps >= self.checkpoint_interval_hours
            {
                return Err(ReplayCacheError::new(format!(
                    "hour index entry {position} violates ordering, bounds, or seek-window invariants"
                )));
            }
            let checkpoint = hour.checkpoint_position as usize;
            if position - checkpoint != hour.delta_steps as usize {
                return Err(ReplayCacheError::new(format!(
                    "hour index entry {position} delta distance disagrees with checkpoint position"
                )));
            }
            if self.hours[checkpoint].delta_steps != 0
                || self.hours[checkpoint].checkpoint_position as usize != checkpoint
            {
                return Err(ReplayCacheError::new(format!(
                    "hour index entry {position} points at a non-checkpoint entry"
                )));
            }
            // ...and at the LATEST one. Pointing past an intervening checkpoint
            // is self-consistent by distance yet unseekable: a seek walks every
            // record between the named checkpoint and the target and requires
            // each to be a delta. Nothing else here notices, because every
            // other rule is local to the entry.
            if hour.delta_steps == 0 {
                active_checkpoint = position;
            }
            if checkpoint != active_checkpoint {
                return Err(ReplayCacheError::new(format!(
                    "hour index entry {position} names checkpoint {checkpoint} with checkpoint {active_checkpoint} between them"
                )));
            }
            previous_hour = Some(hour.hour_index);
            previous_cache_end = hour.cache_record_offset + hour.cache_record_len;
        }
        if previous_cache_end != self.cache_len {
            return Err(ReplayCacheError::new(
                "hour index does not cover the checkpoint cache through its final record",
            ));
        }
        self.validate_h4cr_tiling()
    }

    /// THE TILING LAW: the served hours and the named holes TOGETHER account
    /// for every byte of the H4CR source, once each, in hour order.
    ///
    /// This is the generalisation of the rule it replaces, not a relaxation of
    /// it. The old rule — first served row at offset zero, consecutive served
    /// rows abutting exactly, last row reaching `source_len` — is exactly this
    /// one for a recording with no holes, and it is what made an INTERIOR
    /// injury unrepresentable: dropping the injured hour's row left a gap that
    /// the format could only read as corruption.
    ///
    /// What a hole row buys is the ability to say "these bytes belong to an
    /// hour this recording refuses, and here is why" instead of either lying
    /// about them (stretching a neighbour's range over them, which is hour
    /// misattribution — the owner's stop condition) or refusing the whole
    /// recording. What it must NOT buy is silence: a gap that no hole row
    /// claims is still refused here, so bytes cannot quietly leave the account.
    fn validate_h4cr_tiling(&self) -> Result<(), ReplayCacheError> {
        if self.excluded.len() > MAX_HOURS {
            return Err(ReplayCacheError::new(
                "hour index excludes more hours than the format admits",
            ));
        }
        let mut previous_excluded_hour = None;
        for (position, hole) in self.excluded.iter().enumerate() {
            if hole.reason.trim().is_empty() {
                return Err(ReplayCacheError::new(format!(
                    "excluded hour entry {position} names no reason; an injured hour is never \
                     guessed, only named"
                )));
            }
            if hole.reason.len() > MAX_EXCLUSION_REASON_BYTES {
                return Err(ReplayCacheError::new(format!(
                    "excluded hour entry {position} carries a {}-byte reason, above the \
                     {MAX_EXCLUSION_REASON_BYTES}-byte bound",
                    hole.reason.len()
                )));
            }
            if previous_excluded_hour.is_some_and(|value| hole.hour_index <= value)
                || hole.h4cr_start_offset > hole.h4cr_end_offset
                || hole.h4cr_end_offset > self.source_len
            {
                return Err(ReplayCacheError::new(format!(
                    "excluded hour entry {position} violates ordering or source bounds"
                )));
            }
            previous_excluded_hour = Some(hole.hour_index);
        }

        // Merge the two hour-ordered sequences into one and require it to tile
        // the source. Both inputs are already proved strictly increasing, so a
        // linear merge that also refuses a TIE is a total statement: no hour
        // may be both served and refused.
        let mut served = self.hours.iter().peekable();
        let mut holes = self.excluded.iter().peekable();
        let mut next_offset = 0u64;
        let mut previous_hour: Option<i64> = None;
        loop {
            let take_hole = match (served.peek(), holes.peek()) {
                (None, None) => break,
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (Some(hour), Some(hole)) => {
                    if hour.hour_index == hole.hour_index {
                        return Err(ReplayCacheError::new(format!(
                            "hour index both serves and excludes game hour {}",
                            hour.hour_index
                        )));
                    }
                    hole.hour_index < hour.hour_index
                }
            };
            let (hour_index, start, end) = if take_hole {
                let hole = holes.next().expect("peeked");
                (hole.hour_index, hole.h4cr_start_offset, hole.h4cr_end_offset)
            } else {
                let hour = served.next().expect("peeked");
                (hour.hour_index, hour.h4cr_start_offset, hour.h4cr_end_offset)
            };
            if previous_hour.is_some_and(|value| hour_index <= value) {
                return Err(ReplayCacheError::new(format!(
                    "hour index is not in strictly increasing hour order at game hour {hour_index}"
                )));
            }
            if start != next_offset {
                return Err(ReplayCacheError::new(format!(
                    "game hour {hour_index} starts at H4CR offset {start} but the hours before it \
                     account for the source through {next_offset}; served hours and named holes \
                     must together cover every source byte exactly once"
                )));
            }
            previous_hour = Some(hour_index);
            next_offset = end;
        }
        if next_offset != self.source_len {
            return Err(ReplayCacheError::new(format!(
                "hour index accounts for the H4CR source through {next_offset} of {} byte(s); the \
                 unaccounted tail belongs to no served hour and to no named hole",
                self.source_len
            )));
        }
        Ok(())
    }

    /// THE CONFINEMENT LAW: a damaged cache record costs its own hour and the
    /// rest of its checkpoint window, and nothing beyond it (owner ruling
    /// 2026-08-12, task #191).
    ///
    /// Why exactly that set, and no smaller: a served hour is reconstructed by
    /// decoding its window's checkpoint and applying every delta from there to
    /// it, so an hour's reconstruction reads every record between its
    /// checkpoint and itself. A record at position `p` is therefore read by
    /// every hour at a position `q >= p` in the SAME window, and by no other
    /// hour — the next checkpoint re-anchors the chain from bytes of its own.
    /// Hours before `p` in the window never touch it.
    ///
    /// And no larger: the window is identified by the index's own
    /// `checkpoint_position`, which `validate` has already proved names the
    /// LATEST checkpoint at or before each row. Confining "everything after the
    /// damage" instead would cost the rest of the recording for one flipped
    /// byte, which is the whole-container refusal this replaces wearing a
    /// different name.
    ///
    /// A TRUNCATED cache confines by the same law, from the same set: the audit
    /// puts every position it could not vouch for into `damaged_positions`, so
    /// the window arithmetic below needs no special case. Only the REASON
    /// differs, because "this record is not the one the seal published" and
    /// "this record's bytes are not in the artifact at all" are different facts
    /// about the hour and a consumer asking why is owed the true one.
    ///
    /// The reason string is minted here, once, so the reader's refusal and the
    /// replay authority's coverage row cannot drift into two spellings of the
    /// same injury.
    pub fn confine_damaged_records(&self, verdict: &CacheAuditVerdict) -> Vec<ConfinedHour> {
        let mut confined: BTreeMap<i64, ConfinedHour> = BTreeMap::new();
        for position in verdict.damaged_positions() {
            let Some(damaged) = self.hours.get(*position) else {
                continue;
            };
            let checkpoint_position = damaged.checkpoint_position as usize;
            let Some(checkpoint) = self.hours.get(checkpoint_position) else {
                continue;
            };
            let reason = match verdict.truncation() {
                Some(truncation) if *position >= truncation.first_unvouched_position() => {
                    format!(
                        "the packaged H4CP ends at {} bytes of the {} the sealed H4IX published, \
                         so the record for game hour {} is not wholly in the artifact, and this \
                         hour reconstructs through that record (checkpoint hour {}); the records \
                         before it still hash to the digests the seal published and their hours \
                         are served",
                        truncation.seen_len(),
                        truncation.published_len(),
                        damaged.hour_index,
                        checkpoint.hour_index
                    )
                }
                _ => format!(
                    "the packaged H4CP record for game hour {} is not the record the sealed \
                     H4IX published for it, and this hour reconstructs through that record \
                     (checkpoint hour {}); one damaged record costs its checkpoint window, \
                     never the recording",
                    damaged.hour_index, checkpoint.hour_index
                ),
            };
            for hour in self.hours[*position..]
                .iter()
                .take_while(|hour| hour.checkpoint_position as usize == checkpoint_position)
            {
                // First damaged record in the window wins the naming: a later
                // one in the same window costs no additional hour, and a row
                // that names two causes names neither precisely.
                confined
                    .entry(hour.hour_index)
                    .or_insert_with(|| ConfinedHour {
                        hour_index: hour.hour_index,
                        h4cr_start_offset: hour.h4cr_start_offset,
                        h4cr_end_offset: hour.h4cr_end_offset,
                        damaged_record_hour: damaged.hour_index,
                        checkpoint_hour: checkpoint.hour_index,
                        reason: reason.clone(),
                    });
            }
        }
        confined.into_values().collect()
    }
}

/// One game hour a cache cannot serve because a record it replays through no
/// longer hashes to the digest the sealed index published for it.
///
/// This is post-seal ROT, not a recording-time hole, and the two are different
/// claims about different moments: a [`ReplayExcludedHour`] says the recorder
/// never vouched for the hour, while this says the recorder did vouch for it
/// and the bytes moved afterwards. They are reported in the same vocabulary —
/// an hour, its H4CR byte range, and one named reason — because a replay
/// consumer's question ("can I have this hour, and if not, why") is the same
/// either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedHour {
    hour_index: i64,
    h4cr_start_offset: u64,
    h4cr_end_offset: u64,
    damaged_record_hour: i64,
    checkpoint_hour: i64,
    reason: String,
}

impl ConfinedHour {
    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }
    pub fn h4cr_start_offset(&self) -> u64 {
        self.h4cr_start_offset
    }
    pub fn h4cr_end_offset(&self) -> u64 {
        self.h4cr_end_offset
    }
    /// The hour whose CACHE RECORD was damaged. Equal to [`Self::hour_index`]
    /// for the hour that owns the damage, and different for every other hour
    /// the same damage confines.
    pub fn damaged_record_hour(&self) -> i64 {
        self.damaged_record_hour
    }
    /// The hour holding the checkpoint this hour's window re-anchors from.
    pub fn checkpoint_hour(&self) -> i64 {
        self.checkpoint_hour
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// What one streaming pass over an H4CP proves about it, record by record.
///
/// `damaged_positions` are index positions, not hours, because the audit reads
/// BYTES: it knows which window of the cache disagreed with the sealed index
/// long before anything decodes far enough to have an opinion about hours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheAuditVerdict {
    header: Vec<u8>,
    whole_cache_sha256: String,
    damaged_positions: Vec<usize>,
    truncated: Option<CacheTruncation>,
}

/// A cache whose bytes RAN OUT before the length the sealed index published.
///
/// Everything the audit proved about a truncated cache is proved about its
/// PREFIX: `first_unvouched_position` is the earliest index position whose
/// record did not close and match, so every position below it hashed to the
/// digest the seal published for it, at the offset the seal named. That is a
/// statement about bytes, and truncation cannot weaken it — a preimage of the
/// sealed digest at the sealed offset IS that record, whatever happened to the
/// bytes after it.
///
/// It is deliberately NOT a claim about which end moved. An INSERTION also
/// leaves a prefix that verifies and a tail that does not, and this type does
/// not distinguish the two, because it does not have to: the tail is unvouched
/// under either reading and is confined by name either way. What is refused
/// whole is a cache LONGER than the index published — see [`CacheAudit::finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTruncation {
    published_len: u64,
    seen_len: u64,
    first_unvouched_position: usize,
}

impl CacheTruncation {
    /// The cache length the sealed index published.
    pub fn published_len(&self) -> u64 {
        self.published_len
    }
    /// The bytes that actually arrived.
    pub fn seen_len(&self) -> u64 {
        self.seen_len
    }
    /// The earliest index position the audit could not vouch for. Every
    /// position below it closed against the digest the sealed index published.
    pub fn first_unvouched_position(&self) -> usize {
        self.first_unvouched_position
    }
}

impl CacheAuditVerdict {
    /// The cache's own [`CHECKPOINT_CACHE_HEADER_LEN`]-byte header, retained so
    /// a caller proves its identity from the same pass.
    pub fn header(&self) -> &[u8] {
        &self.header
    }
    /// SHA-256 over every byte that went past. Kept because it is the one check
    /// that does not depend on the per-record account being complete; see
    /// `admit_cache`'s totality argument.
    pub fn whole_cache_sha256(&self) -> &str {
        &self.whole_cache_sha256
    }
    pub fn damaged_positions(&self) -> &[usize] {
        &self.damaged_positions
    }
    pub fn is_intact(&self) -> bool {
        self.damaged_positions.is_empty()
    }
    /// `Some` when the cache ran out before the published length. A truncated
    /// cache is never intact: every position from
    /// [`CacheTruncation::first_unvouched_position`] on is in
    /// [`Self::damaged_positions`].
    pub fn truncation(&self) -> Option<CacheTruncation> {
        self.truncated
    }
}

/// A [`Write`] sink that verifies an H4CP against a sealed index as its bytes
/// stream past, retaining the 40-byte header, two SHA-256 states (one over the
/// whole cache, one over the record in flight) and one index position per
/// DAMAGED record.
///
/// It exists because both consumers of a packaged cache have to reach the same
/// verdict from different plumbing: the reader has the artifact on disk, while
/// the container gate has it inside a ZIP entry it may only stream. Feeding one
/// sink from both is what stops "which records are damaged" from acquiring two
/// implementations that could disagree — and the answer to that question is
/// what decides which hours a recording still serves.
///
/// Memory is O(1) in the cache and O(damaged records) in its damage: a 2 GB
/// cache and a 2 KB one cost the same 40 retained header bytes plus two
/// SHA-256 states whatever the record sizes, and one `usize` per record that
/// disagrees with the sealed index — bounded by the hour count, and zero for
/// every intact cache.
pub struct CacheAudit<'a> {
    index: &'a ReplayHourIndex,
    /// Bytes consumed so far — also the absolute cache offset of the next byte.
    seen: u64,
    header: Vec<u8>,
    whole: Sha256,
    /// The record being hashed, and how many of its bytes are still to come.
    position: usize,
    record: Sha256,
    record_remaining: u64,
    damaged: Vec<usize>,
}

impl<'a> CacheAudit<'a> {
    pub fn new(index: &'a ReplayHourIndex) -> Self {
        Self {
            index,
            seen: 0,
            header: Vec::with_capacity(CACHE_HEADER_LEN as usize),
            whole: Sha256::new(),
            position: 0,
            record: Sha256::new(),
            record_remaining: 0,
            damaged: Vec::new(),
        }
    }

    /// Close the audit.
    ///
    /// # A cache LONGER than the index published is refused whole
    ///
    /// The index accounts for `cache_len` bytes and nothing else. Extra bytes
    /// are described by no row, so no row can be checked against them, and an
    /// insertion ANYWHERE inside the cache produces this shape — every later
    /// record slides out from under the offset the index named. Nothing here
    /// can attribute that to an hour, and an injured hour is never guessed.
    ///
    /// # A cache SHORTER than the index published confines its tail
    ///
    /// This is the narrowing task #192 asks for, and it rests entirely on the
    /// per-record digests H4IX v4 carries. A record at position `p` is hashed
    /// over exactly the bytes `[cache_record_offset, +cache_record_len)` as
    /// counted from the stream's first byte; when that digest equals the one
    /// the SEALED index published for `p`, those bytes are that record. Bytes
    /// that never arrived cannot un-prove it. So the prefix of positions that
    /// closed and matched is vouched, and everything from the first position
    /// that did not — the record in flight when the bytes ran out, and every
    /// position after it — is unvouched and confined by name.
    ///
    /// The old refusal's argument was that an insertion cannot be told from a
    /// truncation without assuming which end moved. That argument is answered
    /// rather than overruled: nothing here decides which end moved. A short
    /// cache with a middle deletion, a short cache with a lost tail and a short
    /// cache with an early insertion all vouch for exactly the positions that
    /// hashed correctly and confine exactly the rest.
    ///
    /// Two shapes stay whole refusals even when short:
    ///
    ///  * fewer bytes than the [`CACHE_HEADER_LEN`]-byte header — the
    ///    artifact's identity is what the header carries, and a cache whose
    ///    identity cannot be established is not this recording's cache;
    ///  * a short cache that nonetheless hashes to the whole-cache digest the
    ///    index published, which is the index describing two different caches.
    ///    Handled in `admit_cache` with the other account disagreements.
    pub fn finish(mut self) -> Result<CacheAuditVerdict, ReplayCacheError> {
        if self.seen > self.index.cache_len {
            return Err(ReplayCacheError::new(format!(
                "the checkpoint cache is {} bytes but the index published {}; the index accounts \
                 for no byte past {}, so the extra bytes belong to no record and every record \
                 offset the index names may have been displaced by them",
                self.seen, self.index.cache_len, self.index.cache_len
            )));
        }
        let truncated = if self.seen < self.index.cache_len {
            if self.seen < CACHE_HEADER_LEN {
                return Err(ReplayCacheError::new(format!(
                    "the checkpoint cache is {} bytes, short of its own {CACHE_HEADER_LEN}-byte \
                     header; the artifact's identity is in that header, so nothing here can say \
                     which recording these bytes belong to, let alone which hour they injure",
                    self.seen
                )));
            }
            // Every position from the one in flight on is unvouched: its bytes
            // either arrived incomplete or did not arrive. Positions already in
            // `self.damaged` closed and disagreed, and stay named.
            let first_unvouched = self.position;
            for position in first_unvouched..self.index.hours.len() {
                self.damaged.push(position);
            }
            self.damaged.sort_unstable();
            self.damaged.dedup();
            Some(CacheTruncation {
                published_len: self.index.cache_len,
                seen_len: self.seen,
                first_unvouched_position: first_unvouched,
            })
        } else {
            // Only reachable when `seen == cache_len` and the last row's record
            // was still open, which the index's own tiling rule forbids;
            // spelled out so a future format change that broke the tiling
            // cannot close an unclosed record silently.
            self.close_record_if_complete();
            None
        };
        Ok(CacheAuditVerdict {
            header: self.header,
            whole_cache_sha256: to_hex(&self.whole.finalize()),
            damaged_positions: self.damaged,
            truncated,
        })
    }

    /// Compare the record just finished against the digest its index row
    /// publishes, and start the next one.
    fn close_record_if_complete(&mut self) {
        if self.record_remaining != 0 {
            return;
        }
        if let Some(entry) = self.index.hours.get(self.position) {
            if self.seen >= entry.cache_record_offset + entry.cache_record_len {
                let digest = to_hex(&std::mem::replace(&mut self.record, Sha256::new()).finalize());
                if digest != entry.cache_record_sha256 {
                    self.damaged.push(self.position);
                }
                self.position += 1;
                self.record_remaining = self
                    .index
                    .hours
                    .get(self.position)
                    .map(|next| next.cache_record_len)
                    .unwrap_or(0);
            }
        }
    }

    /// Route one contiguous run of cache bytes into the header, the record
    /// being hashed, or neither.
    fn absorb(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.seen < CACHE_HEADER_LEN {
                let take = ((CACHE_HEADER_LEN - self.seen) as usize).min(bytes.len());
                self.header.extend_from_slice(&bytes[..take]);
                self.whole.update(&bytes[..take]);
                self.seen += take as u64;
                bytes = &bytes[take..];
                if self.seen == CACHE_HEADER_LEN {
                    self.record_remaining = self
                        .index
                        .hours
                        .first()
                        .map(|first| first.cache_record_len)
                        .unwrap_or(0);
                }
                continue;
            }
            if self.record_remaining == 0 {
                // Bytes belonging to no record. The index's tiling rule makes
                // this unreachable for an index that parsed (the rows abut from
                // the header through `cache_len`), and a stream longer than
                // `cache_len` is refused by `finish`; they are still hashed so
                // the whole-cache digest stays a digest of everything.
                self.whole.update(bytes);
                self.seen += bytes.len() as u64;
                return;
            }
            let take = (self.record_remaining as usize).min(bytes.len());
            self.record.update(&bytes[..take]);
            self.whole.update(&bytes[..take]);
            self.seen += take as u64;
            self.record_remaining -= take as u64;
            bytes = &bytes[take..];
            self.close_record_if_complete();
        }
    }
}

impl Write for CacheAudit<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.absorb(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheBuildHour {
    pub hour_index: i64,
    pub date: String,
    pub h4cr_start_offset: u64,
    pub h4cr_end_offset: u64,
    pub world_json: Vec<u8>,
}

/// Everything about one build hour except its world body.
///
/// Exists so [`ReplayCacheEncoder::push_prepared`] can take a body that is
/// already parsed without also taking a second, unrelated copy of the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheBuildHourMeta {
    pub hour_index: i64,
    pub date: String,
    pub h4cr_start_offset: u64,
    pub h4cr_end_offset: u64,
}

/// One hour's canonical world JSON together with the cache value model's parse
/// of it.
///
/// The pair is the point. A caller that needs to READ the world document -- to
/// externalize an inline render texture, say -- would otherwise parse the same
/// tens of megabytes a second time, with a second parser, purely because the
/// encoder's parse is private to `push`. This type hands that one parse out
/// while keeping the bytes and the tree welded together: the only constructor
/// takes the bytes, parses them, and then owns both, so no caller can pair a
/// value with a body it did not come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalWorldHour {
    /// The digest of the source body, taken at parse time.
    ///
    /// The BODY itself is deliberately not retained. It used to be, all the way
    /// through the delta encode, and the only thing it was ever used for was
    /// this one hash at the very end — so a whole second copy of the world
    /// document (34.5 MB on capture t30) stayed resident beside the parsed tree
    /// it is redundant with, for the whole of the most memory-expensive stretch
    /// of finalize. Hashing at parse and dropping the bytes changes no output:
    /// the digest is of exactly the same bytes it always was.
    world_json_sha256: String,
    value: CanonValue,
    /// What [`Self::value`]'s tree owns, counted during the parse that built
    /// it. Carried so the encoder can price its retained delta base without
    /// walking a million-node tree once per game hour.
    value_heap_bytes: u64,
}

impl CanonicalWorldHour {
    /// Parse an hour's canonical world JSON.
    ///
    /// `hour_index` names the hour in the error only; the value's own identity
    /// members are checked against the index later, by the encoder.
    ///
    /// No whole-document RE-SERIALIZATION happens here. Proving the value model
    /// spells THIS body back was once done inline, by re-emitting the parse and
    /// comparing it against the source -- a second pass over tens of megabytes,
    /// once per hour, entirely in memory. The claim it proved is strictly weaker
    /// than what the seal path proves anyway:
    /// [`ReplayCacheReader::verify_every_hour`] decodes every record of the
    /// FINISHED artifact, applies the decoded patches, re-serializes each
    /// reconstructed hour and requires the recorded `world_json_sha256` -- and
    /// that digest is taken from THESE source bytes, never from the parse. So a
    /// token shape that survives parsing but not serialization still fails the
    /// build loudly, and it fails it on the bytes a reader will actually follow
    /// instead of on an in-memory copy of them.
    /// `world_json` is consumed: its digest is taken here and the bytes are
    /// released, so the caller's copy does not outlive the parse.
    pub fn parse(world_json: Vec<u8>, hour_index: i64) -> Result<Self, ReplayCacheError> {
        let (value, value_heap_bytes) = CanonValue::parse_counted(&world_json).map_err(|error| {
            ReplayCacheError::new(format!(
                "hour {hour_index} is not strict canonical world JSON: {error}"
            ))
        })?;
        let world_json_sha256 = Sha256::hex(&world_json);
        drop(world_json);
        Ok(Self {
            world_json_sha256,
            value,
            value_heap_bytes,
        })
    }

    /// What the parsed tree owns on the heap.
    pub fn value_heap_bytes(&self) -> u64 {
        self.value_heap_bytes
    }

    /// The parsed world body.
    pub fn value(&self) -> &CanonValue {
        &self.value
    }

    /// The digest of the canonical world JSON this value was parsed from.
    pub fn world_json_sha256(&self) -> &str {
        &self.world_json_sha256
    }
}

pub struct ReplayCacheEncoder<W: Write> {
    writer: W,
    hasher: Sha256,
    offset: u64,
    source_sha256: String,
    checkpoint_interval_hours: u32,
    previous: Option<CanonValue>,
    /// Whether this encoder may EMIT revives. On.
    ///
    /// THE DECODER ALWAYS UNDERSTANDS THEM; only the writer is gated, so an
    /// encoder with this off writes byte-for-byte what format 4 wrote.
    ///
    /// It was off for one round, because pricing the revive over t53's 219
    /// hours failed with "revive patch has no remembered value to revive" --
    /// the encoder emitting a revive against a memory the reader did not hold.
    /// The cause was `ShadowBudget` charging `heap_bytes`, which counts vector
    /// CAPACITY: the JSON parser grows its vectors by doubling while the binary
    /// decoder reserves the element count exactly, so equal documents carried
    /// different capacities on the two sides, their monotonic spend diverged,
    /// and near the cap one side admitted a memory the other refused (Codex
    /// review round 7, Q1). Admission now charges `content_bytes`, which is a
    /// function of the document alone.
    ///
    /// With that closed the revive round-trips byte-identically over ALL FOUR
    /// sealed runs -- t51 (602 h), t53 (219 h), t54 (357 h) and t55 (1,416 h)
    /// -- each hour reconstructed by the production reader and required to hash
    /// to the digest the original recording published.
    revive_enabled: bool,
    /// What each currently-null path last held, since the open checkpoint.
    shadow: Shadow,
    /// What that has cost, against the ceiling one window may spend.
    shadow_budget: ShadowBudget,
    entries: Vec<ReplayHourEntry>,
    /// The hours the caller told this encoder to SKIP, each with the reason its
    /// recording gave. They write no cache record — an excluded hour is never
    /// served — but they hold their place in the H4CR account, which is what
    /// lets the served rows around them stay honest about their own byte
    /// ranges.
    excluded: Vec<ReplayExcludedHour>,
    /// The H4CR offset the next hour, served or excluded, must begin at.
    ///
    /// One cursor for both kinds is the whole point: it keeps the encoder's
    /// refusal of an UNEXPLAINED gap exactly as strict as it was before holes
    /// existed. A caller that skips an hour without declaring it still fails
    /// here.
    next_h4cr_offset: u64,
    /// The last hour index accepted through either door, so hours stay strictly
    /// increasing across the merged sequence rather than only within each kind.
    last_hour_index: Option<i64>,
    checkpoint_position: usize,
    /// What `previous`'s tree owns, carried from the parse that built it.
    previous_heap_bytes: u64,
    /// Running total of the heap owned by `entries`' `String` fields.
    ///
    /// Kept incrementally rather than recomputed: `entries` gains a row per
    /// game hour, so walking it to price it would cost O(hours) per hour and
    /// turn a memory question into a quadratic one.
    entries_string_bytes: u64,
}

impl<W: Write> ReplayCacheEncoder<W> {
    pub fn new(
        writer: W,
        source_sha256: impl Into<String>,
        checkpoint_interval_hours: u32,
    ) -> Result<Self, ReplayCacheError> {
        let source_sha256 = source_sha256.into();
        let source_digest = decode_hex32(&source_sha256)?;
        if !(1..=MAX_CHECKPOINT_INTERVAL_HOURS).contains(&checkpoint_interval_hours) {
            return Err(ReplayCacheError::new(format!(
                "checkpoint interval {checkpoint_interval_hours} is outside 1..={MAX_CHECKPOINT_INTERVAL_HOURS}"
            )));
        }
        let mut encoder = Self {
            revive_enabled: true,
            shadow: Shadow::default(),
            shadow_budget: ShadowBudget::default(),
            writer,
            hasher: Sha256::new(),
            offset: 0,
            source_sha256,
            checkpoint_interval_hours,
            previous: None,
            entries: Vec::new(),
            excluded: Vec::new(),
            next_h4cr_offset: 0,
            last_hour_index: None,
            checkpoint_position: 0,
            previous_heap_bytes: 0,
            entries_string_bytes: 0,
        };
        encoder.write_hashed(CACHE_MAGIC)?;
        encoder.write_hashed(&FORMAT_VERSION.to_le_bytes())?;
        encoder.write_hashed(&0u16.to_le_bytes())?;
        encoder.write_hashed(&source_digest)?;
        Ok(encoder)
    }

    pub fn push(&mut self, hour: CacheBuildHour) -> Result<(), ReplayCacheError> {
        let CacheBuildHour {
            hour_index,
            date,
            h4cr_start_offset,
            h4cr_end_offset,
            world_json,
        } = hour;
        let meta = CacheBuildHourMeta {
            hour_index,
            date,
            h4cr_start_offset,
            h4cr_end_offset,
        };
        self.check_ordered_and_contiguous(&meta)?;
        let prepared = CanonicalWorldHour::parse(world_json, meta.hour_index)?;
        self.encode_prepared(meta, prepared)
    }

    /// Encode an hour whose world body the caller has already parsed.
    ///
    /// Identical to [`Self::push`] in every byte it writes and every check it
    /// runs -- [`CanonicalWorldHour::parse`] IS the parse `push` performs
    /// inline. The seam exists so the one consumer that also has to read the
    /// world document does not parse it twice.
    pub fn push_prepared(
        &mut self,
        meta: CacheBuildHourMeta,
        prepared: CanonicalWorldHour,
    ) -> Result<(), ReplayCacheError> {
        self.check_ordered_and_contiguous(&meta)?;
        self.encode_prepared(meta, prepared)
    }

    /// SKIP an hour this recording does not vouch for, naming why.
    ///
    /// The hour writes no cache record, so nothing can ever serve it — that is
    /// the point. What it does write is its place in the H4CR account, so the
    /// next served hour can start where this one ended without the encoder
    /// having to accept an unexplained jump. Declaring the skip is therefore
    /// mandatory: `push`ing across a gap still fails, exactly as it did before
    /// holes were representable.
    ///
    /// `reason` must be the reason the recording's own coverage map gave. This
    /// encoder cannot check that — it has never seen the coverage map — so the
    /// producer asserts it and the replay authority re-joins the two.
    pub fn exclude_hour(&mut self, hole: ReplayExcludedHour) -> Result<(), ReplayCacheError> {
        if hole.reason.trim().is_empty() {
            return Err(ReplayCacheError::new(format!(
                "game hour {} cannot be excluded without a named reason",
                hole.hour_index
            )));
        }
        if hole.reason.len() > MAX_EXCLUSION_REASON_BYTES {
            return Err(ReplayCacheError::new(format!(
                "game hour {}'s exclusion reason is {} bytes, above the \
                 {MAX_EXCLUSION_REASON_BYTES}-byte bound",
                hole.hour_index,
                hole.reason.len()
            )));
        }
        if hole.h4cr_start_offset > hole.h4cr_end_offset {
            return Err(ReplayCacheError::new(format!(
                "game hour {}'s excluded H4CR range runs backwards",
                hole.hour_index
            )));
        }
        self.check_hour_order_and_placement(hole.hour_index, hole.h4cr_start_offset)?;
        self.next_h4cr_offset = hole.h4cr_end_offset;
        self.last_hour_index = Some(hole.hour_index);
        self.excluded.push(hole);
        Ok(())
    }

    fn check_ordered_and_contiguous(
        &self,
        meta: &CacheBuildHourMeta,
    ) -> Result<(), ReplayCacheError> {
        if meta.date.trim().is_empty() || meta.h4cr_start_offset >= meta.h4cr_end_offset {
            return Err(ReplayCacheError::new(
                "cache input hours are not strictly ordered and contiguous",
            ));
        }
        self.check_hour_order_and_placement(meta.hour_index, meta.h4cr_start_offset)
    }

    /// The one placement rule both doors go through: hours arrive in strictly
    /// increasing order and each begins exactly where the previous one ended,
    /// with the first at decoded offset zero.
    fn check_hour_order_and_placement(
        &self,
        hour_index: i64,
        h4cr_start_offset: u64,
    ) -> Result<(), ReplayCacheError> {
        if self
            .last_hour_index
            .is_some_and(|previous| hour_index <= previous)
            || h4cr_start_offset != self.next_h4cr_offset
        {
            return Err(ReplayCacheError::new(
                "cache input hours are not strictly ordered and contiguous",
            ));
        }
        Ok(())
    }

    fn encode_prepared(
        &mut self,
        meta: CacheBuildHourMeta,
        prepared: CanonicalWorldHour,
    ) -> Result<(), ReplayCacheError> {
        let CanonicalWorldHour {
            world_json_sha256,
            value,
            value_heap_bytes,
        } = prepared;
        // `meta` used to be re-wrapped into a `CacheBuildHour` so it could
        // carry the body alongside; with the body gone there is nothing to wrap
        // and the caller's `meta` IS the hour.
        let hour = meta;
        validate_world_identity(&value, hour.hour_index, &hour.date)?;
        let position = self.entries.len();
        let is_checkpoint = position % self.checkpoint_interval_hours as usize == 0;
        let (record_kind, payload, delta_steps) = if is_checkpoint {
            self.checkpoint_position = position;
            // A SEEK STARTS AT A CHECKPOINT AND KNOWS NOTHING BEFORE IT, so the
            // encoder must forget everything before it too, or it would emit
            // revives no reader could resolve.
            self.shadow = Shadow::default();
            self.shadow_budget = ShadowBudget::default();
            (RECORD_CHECKPOINT, encode_value(&value)?, 0)
        } else {
            let previous = self
                .previous
                .as_mut()
                .expect("a non-checkpoint cache record has a previous hour");
            let patch = diff_value(
                previous,
                &value,
                self.revive_enabled.then_some(&self.shadow),
                0,
            )?;
            let payload = encode_patch(&patch)?;
            // CARRY THE SHADOW over the step the reader is about to take, using
            // the reader's own function, over the previous hour this encoder is
            // about to drop anyway. It costs a walk of what CHANGED -- not a
            // clone of the world, which is what the note below refuses.
            apply_patch(previous, &patch, &mut self.shadow, &mut self.shadow_budget, 0)?;
            // No in-memory replay of the patch here. The delta a reader will
            // actually follow is the one DECODED from `payload`, applied to a
            // value decoded from the cache -- and that is exactly what
            // `ReplayCacheReader::verify_every_hour` does, over every record of
            // the finished artifact, against each hour's recorded
            // `world_json_sha256`. Re-applying the in-memory `patch` to a clone
            // of the in-memory `previous` proves a strictly weaker statement
            // (it never touches the encoding) at the price of cloning a
            // whole-world `CanonValue` -- tens of megabytes of `String`
            // allocations -- once per delta hour. The seal path runs
            // `verify_every_hour` unconditionally, so nothing is left unproved.
            (
                RECORD_DELTA,
                payload,
                u32::try_from(position - self.checkpoint_position)
                    .map_err(|_| ReplayCacheError::new("delta distance does not fit u32"))?,
            )
        };
        let record_offset = self.offset;
        let payload_len = u64::try_from(payload.len())
            .map_err(|_| ReplayCacheError::new("cache payload length does not fit u64"))?;
        let mut header = Vec::with_capacity(21);
        header.push(record_kind);
        header.extend_from_slice(&[0; 3]);
        put_i64(&mut header, hour.hour_index);
        put_u64(&mut header, payload_len);
        header.push(0);
        // The record's OWN digest, over exactly the window the index row will
        // publish. Taken here, from the bytes on their way to the writer, so it
        // is a digest of what was written rather than of a second rendering of
        // it. This is the second hash pass over the cache body (the first being
        // the running whole-cache hash); on capture t48's 1.53 GB cache that is
        // one extra SHA-256 sweep per finalize, measured in
        // `runs/ha_entry_confinement/`.
        let mut record_digest = Sha256::new();
        record_digest.update(&header);
        record_digest.update(&payload);
        self.write_hashed(&header)?;
        self.write_hashed(&payload)?;
        let record_len = self.offset - record_offset;
        self.entries.push(ReplayHourEntry {
            hour_index: hour.hour_index,
            date: hour.date,
            h4cr_start_offset: hour.h4cr_start_offset,
            h4cr_end_offset: hour.h4cr_end_offset,
            cache_record_offset: record_offset,
            cache_record_len: record_len,
            checkpoint_position: u32::try_from(self.checkpoint_position)
                .map_err(|_| ReplayCacheError::new("checkpoint position does not fit u32"))?,
            delta_steps,
            world_json_sha256,
            cache_record_sha256: to_hex(&record_digest.finalize()),
        });
        self.next_h4cr_offset = hour.h4cr_end_offset;
        self.last_hour_index = Some(hour.hour_index);
        if let Some(entry) = self.entries.last() {
            self.entries_string_bytes = self.entries_string_bytes.saturating_add(
                (entry.date.capacity()
                    + entry.world_json_sha256.capacity()
                    + entry.cache_record_sha256.capacity()) as u64,
            );
        }
        self.previous = Some(value);
        self.previous_heap_bytes = value_heap_bytes;
        Ok(())
    }

    /// Bytes the retained delta base (the previous hour's canonical tree) owns.
    ///
    /// This is the largest single object the encoder holds between hours, and
    /// it is proportional to the size of the world, not to how long the session
    /// ran.
    ///
    /// O(1): the number was counted during the parse that built the tree, not
    /// measured by walking it. Walking cost about 100 ms per game hour on
    /// capture t30 — an instrument that slowed the thing it was instrumenting
    /// by more than a tenth. `CanonValue::heap_bytes` is still the authority
    /// and the two are held together by
    /// `canon_value::heap_bytes_tests::parse_counted_matches_the_independent_walk`.
    pub fn delta_base_heap_bytes(&self) -> u64 {
        if self.previous.is_some() {
            self.previous_heap_bytes
        } else {
            0
        }
    }

    /// Stop this encoder emitting revives, leaving a format-4-shaped cache.
    ///
    /// Kept because the gate is what made the divergence in
    /// [`Self::revive_enabled`] safe to hold while it was open, and because the
    /// tests that prove the revive earns its bytes need a control that does not
    /// emit one. Nothing in production calls it.
    pub fn forbid_revive(&mut self) {
        self.revive_enabled = false;
    }

    /// An upper bound on what the folded-out world costs this encoder.
    ///
    /// PUBLISHED SEPARATELY because it is a different shape of cost from the
    /// delta base: that one is a world, this one grows with how much of the
    /// world is currently folded out and is released at every checkpoint. It
    /// was invisible to the component memory table when the shadow was first
    /// added, which meant a real allocation with no row (Codex review round 6,
    /// finding S4).
    ///
    /// An UPPER bound, not a measurement: the budget is charged when a memory
    /// is taken and never refunded when one is dropped, so this reads at or
    /// above what is actually resident, and it is bounded by
    /// [`MAX_SHADOW_BYTES`] by construction.
    pub fn shadow_heap_bytes_upper_bound(&self) -> u64 {
        self.shadow_budget.spent
    }

    /// Bytes the per-hour index rows own: the `Vec`'s reservation plus every
    /// row's three `String`s — its date and its two digests.
    ///
    /// This one DOES grow with session length — one row per game hour, for the
    /// whole recording — which is why it is published separately from the
    /// delta base rather than summed into a single "encoder" number.
    pub fn hour_entries_heap_bytes(&self) -> u64 {
        ((self.entries.capacity() * std::mem::size_of::<ReplayHourEntry>()) as u64)
            .saturating_add(self.entries_string_bytes)
    }

    /// Hours encoded so far, for the per-hour growth rate the table reports.
    pub fn hour_entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn finish(
        self,
        game_id: impl Into<String>,
        schema_version: impl Into<String>,
        identity_mode: CacheIdentityMode,
        source_len: u64,
    ) -> Result<(W, ReplayHourIndex), ReplayCacheError> {
        if self.entries.is_empty() {
            return Err(ReplayCacheError::new(
                "cannot finish an empty replay checkpoint cache",
            ));
        }
        let index = ReplayHourIndex {
            game_id: game_id.into(),
            schema_version: schema_version.into(),
            identity_mode,
            source_len,
            source_sha256: self.source_sha256,
            checkpoint_interval_hours: self.checkpoint_interval_hours,
            cache_len: self.offset,
            cache_sha256: to_hex(&self.hasher.finalize()),
            hours: self.entries,
            excluded: self.excluded,
        };
        index.validate()?;
        Ok((self.writer, index))
    }

    fn write_hashed(&mut self, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.writer
            .write_all(bytes)
            .map_err(|error| ReplayCacheError::new(format!("write checkpoint cache: {error}")))?;
        self.hasher.update(bytes);
        self.offset = self
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| ReplayCacheError::new("checkpoint cache length overflow"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaySeekReceipt {
    pub checkpoint_hour: i64,
    pub target_hour: i64,
    pub applied_deltas: u32,
    pub configured_bound: u32,
}

/// Where a certified cache's bytes live.
///
/// Everything this module does with a cache reduces to two questions: what does
/// it stream, and what are the bytes of one indexed record. Nothing needs the
/// whole cache resident, and on a real campaign nothing could hold it: capture
/// t32 emitted 2,209,767,138 bytes for 656 hours (3.37 MB per hour), so a full
/// campaign's cache is tens of gigabytes. A slice backing serves the encoder
/// and the small fixtures; a file backing serves every artifact big enough to
/// matter.
///
/// It used to answer a third question — HOW LONG IS IT — for `admit_cache` to
/// compare against the index before streaming anything. Task #192 moved that
/// judgement into [`CacheAudit::finish`], which measures the length of what
/// actually streamed and has the per-record account in hand when it does, so
/// the backing is no longer asked. Each implementation still bounds its own
/// reads by its own real length through [`record_window`], which is the check
/// that mattered.
pub(crate) trait CacheRecords {
    /// Stream every byte of the cache, in order, past `sink`.
    ///
    /// This replaces the whole-cache `cache_sha256()` this trait used to
    /// expose. The pass is the same pass — one sequential read of the whole
    /// artifact at open — but what it feeds is now a [`CacheAudit`], which
    /// answers the localisable question (which RECORDS are the sealed ones)
    /// as well as the whole-artifact one. A backing that could only produce
    /// one digest could not confine anything.
    fn read_all(&self, sink: &mut dyn Write) -> Result<(), ReplayCacheError>;
    /// The bytes of the record at `offset`, refusing any window that leaves the
    /// cache or exceeds [`MAX_CACHE_RECORD_BYTES`].
    fn record(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>, ReplayCacheError>;
}

fn record_window(cache_len: u64, offset: u64, len: u64) -> Result<(u64, usize), ReplayCacheError> {
    if len > MAX_CACHE_RECORD_BYTES {
        return Err(ReplayCacheError::new(format!(
            "cache record length {len} exceeds the {MAX_CACHE_RECORD_BYTES}-byte per-record reader bound"
        )));
    }
    offset
        .checked_add(len)
        .filter(|end| *end <= cache_len)
        .ok_or_else(|| ReplayCacheError::new("cache record range is out of bounds"))?;
    let len = usize::try_from(len)
        .map_err(|_| ReplayCacheError::new("cache record length does not fit usize"))?;
    Ok((offset, len))
}

/// A resident cache. Used by the encoder's own certification pass and by every
/// fixture small enough that holding it is not a decision.
pub(crate) struct SliceCache<'a>(pub(crate) &'a [u8]);

impl<'a> CacheRecords for SliceCache<'a> {
    fn read_all(&self, sink: &mut dyn Write) -> Result<(), ReplayCacheError> {
        sink.write_all(self.0)
            .map_err(|error| ReplayCacheError::new(format!("audit resident cache: {error}")))
    }

    fn record(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>, ReplayCacheError> {
        let (offset, len) = record_window(self.0.len() as u64, offset, len)?;
        let start = usize::try_from(offset)
            .map_err(|_| ReplayCacheError::new("cache record offset does not fit usize"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| ReplayCacheError::new("cache record range is out of bounds"))?;
        Ok(Cow::Borrowed(&self.0[start..end]))
    }
}

/// A certified cache held as an open file rather than as bytes.
///
/// The handle is retained for the life of the cache, and every read is bounded
/// by the record length the authenticated H4IX declares. The caller owns the
/// file's pathname and lifetime; this type only reads it.
pub struct CertifiedCacheFile {
    file: Mutex<File>,
    len: u64,
}

impl CertifiedCacheFile {
    pub fn new(file: File) -> Result<Self, ReplayCacheError> {
        let len = file
            .metadata()
            .map_err(|error| {
                ReplayCacheError::new(format!("stat file-backed checkpoint cache: {error}"))
            })?
            .len();
        Ok(Self {
            file: Mutex::new(file),
            len,
        })
    }
}

impl CacheRecords for CertifiedCacheFile {
    fn read_all(&self, sink: &mut dyn Write) -> Result<(), ReplayCacheError> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| ReplayCacheError::new("file-backed checkpoint cache lock poisoned"))?;
        file.seek(SeekFrom::Start(0)).map_err(|error| {
            ReplayCacheError::new(format!("seek file-backed checkpoint cache: {error}"))
        })?;
        let mut buf = vec![0u8; CACHE_HASH_CHUNK];
        let mut seen = 0u64;
        loop {
            let read = file.read(&mut buf).map_err(|error| {
                ReplayCacheError::new(format!("read file-backed checkpoint cache: {error}"))
            })?;
            if read == 0 {
                break;
            }
            sink.write_all(&buf[..read]).map_err(|error| {
                ReplayCacheError::new(format!("audit file-backed checkpoint cache: {error}"))
            })?;
            seen = seen.saturating_add(read as u64);
        }
        if seen != self.len {
            return Err(ReplayCacheError::new(format!(
                "file-backed checkpoint cache changed length while being audited (opened {}, read \
                 {seen})",
                self.len
            )));
        }
        Ok(())
    }

    fn record(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>, ReplayCacheError> {
        let (offset, len) = record_window(self.len, offset, len)?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| ReplayCacheError::new("file-backed checkpoint cache lock poisoned"))?;
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            ReplayCacheError::new(format!("seek file-backed checkpoint cache record: {error}"))
        })?;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes).map_err(|error| {
            ReplayCacheError::new(format!("read file-backed checkpoint cache record: {error}"))
        })?;
        Ok(Cow::Owned(bytes))
    }
}

#[derive(Clone)]
enum CacheBacking {
    Memory(Arc<[u8]>),
    File(Arc<CertifiedCacheFile>),
}

impl CacheBacking {
    fn with_records<R>(&self, body: impl FnOnce(&dyn CacheRecords) -> R) -> R {
        match self {
            CacheBacking::Memory(bytes) => body(&SliceCache(bytes.as_ref())),
            CacheBacking::File(file) => body(file.as_ref()),
        }
    }
}

pub struct ReplayCacheReader<'a> {
    index: ReplayHourIndex,
    cache: &'a [u8],
    confined: Vec<ConfinedHour>,
}

/// Bytes of the H4CP file header: magic, format version, flags, source digest.
/// A consumer that only needs to prove the cache's identity — not to seek in it
/// — need retain no more of the entry than this.
pub const CHECKPOINT_CACHE_HEADER_LEN: u64 = CACHE_HEADER_LEN;

/// Prove an H4CP header names this format, this version, and this H4CR source.
///
/// Split out so a consumer that streams the cache past itself (the compact
/// verification gate does exactly that) proves the same header identity a
/// seeking reader proves, from the same code, instead of a second spelling of
/// the check that could drift.
pub fn validate_checkpoint_cache_header(
    header: &[u8],
    expected_source_sha256: &str,
) -> Result<(), ReplayCacheError> {
    let mut header = ByteReader::new(header);
    if header.take(4)? != CACHE_MAGIC
        || header.u16()? != FORMAT_VERSION
        || header.u16()? != 0
        || to_hex(header.array32()?) != expected_source_sha256
    {
        return Err(ReplayCacheError::new(
            "checkpoint cache header identity is invalid",
        ));
    }
    Ok(())
}

/// Bind an index to a cache, prove the cache's own header agrees, and judge the
/// cache RECORD BY RECORD. This is the single admission gate both reader shapes
/// go through.
///
/// What changed with the per-record digest, and why the shape of the refusal
/// changed with it: this used to compare one whole-cache digest and refuse the
/// whole artifact when it disagreed, which meant a single flipped byte cost
/// every hour in the recording. It now streams the same bytes past a
/// [`CacheAudit`] — the same one pass, the same read — and returns the hours
/// the damage actually reaches. Confining is not the same as forgiving: a
/// confined hour is refused by name at every door, and every kind of damage
/// that CANNOT be attributed to a record is still a whole-artifact refusal.
///
/// The unattributable kinds, each refused here:
///
///  * the cache is LONGER than the index published, or shorter than its own
///    header ([`CacheAudit::finish`]);
///  * the cache's HEADER moved — its magic, format version, or the H4CR source
///    it is bound to. That is the artifact's identity, not an hour's, and the
///    per-record account rests on the framing it declares;
///  * the whole-cache digest disagrees while every record agrees;
///  * the whole-cache digest AGREES while a record disagrees, or while the
///    cache is short. Those are the index contradicting itself; see below.
///
/// A cache that is merely SHORT is not one of them (task #192): its verified
/// prefix serves and its tail is confined by name. See [`CacheAudit::finish`].
fn admit_cache(
    index_bytes: &[u8],
    cache: &dyn CacheRecords,
    expected_source_len: u64,
    expected_source_sha256: &str,
) -> Result<(ReplayHourIndex, Vec<ConfinedHour>), ReplayCacheError> {
    let index = ReplayHourIndex::from_bytes(index_bytes)?;
    if index.source_len != expected_source_len || index.source_sha256 != expected_source_sha256 {
        return Err(ReplayCacheError::new(
            "replay index/cache is not bound to the selected canonical H4CR source",
        ));
    }
    // The cache LENGTH is judged by the audit, not here. It used to be part of
    // the binding above, which meant a truncated artifact was refused with a
    // sentence about the H4CR source it is in fact still bound to — and refused
    // before a single per-record digest was read. `CacheAudit::finish` sees the
    // same disagreement one pass later, with the per-record account in hand,
    // and can say which hours it costs.
    let mut audit = CacheAudit::new(&index);
    cache.read_all(&mut audit)?;
    let verdict = audit.finish()?;
    validate_checkpoint_cache_header(verdict.header(), expected_source_sha256)?;
    // THE TWO ACCOUNTS MUST AGREE. The index states one whole-cache digest and
    // one digest per record; over a cache of the published length those are two
    // accounts of the same bytes, because `validate` has already proved the
    // rows tile `[CACHE_HEADER_LEN, cache_len)` exactly and the header is
    // exactly the leading `CACHE_HEADER_LEN` bytes.
    //
    // Nothing checks that the index's two accounts agree with EACH OTHER before
    // a cache is in hand — `validate` has no cache to hash — so an index whose
    // whole digest and row digests describe different caches parses cleanly and
    // arrives here. That is what these arms are for, and both are pinned by
    // `an_index_whose_two_accounts_disagree_is_not_an_authority` (Codex review
    // round 6, finding 2). No recorder produces such an index: the encoder
    // derives both from the same byte stream as it writes it.
    //
    // Both are refused rather than resolved, because an account that does not
    // add up cannot be partially believed.
    //
    // A TRUNCATED cache is exempt from the first arm and only from it: its
    // whole-cache digest is a digest of a PREFIX, so of course it disagrees,
    // and that disagreement is explained by the length the audit measured
    // rather than by an index that does not add up. The second arm still
    // applies with full force — a short cache whose bytes nonetheless hash to
    // the digest published for the full-length cache is an index describing two
    // different caches, and is refused below.
    match (
        verdict.whole_cache_sha256() == index.cache_sha256,
        verdict.is_intact(),
    ) {
        (true, true) | (false, false) => {}
        (false, true) => {
            return Err(ReplayCacheError::new(format!(
                "the checkpoint cache hashes to {} but the index is bound to {}, while every one \
                 of its {} records matches the digest the index published; the per-record account \
                 does not add up, so no part of it is trusted",
                verdict.whole_cache_sha256(),
                index.cache_sha256,
                index.hours.len(),
            )))
        }
        (true, false) => {
            return Err(ReplayCacheError::new(match verdict.truncation() {
                Some(truncation) => format!(
                    "the checkpoint cache is {} bytes of the {} the index published, yet it \
                     hashes to the whole-cache digest {} the index is bound to; a prefix cannot \
                     hash to the whole, so the index describes two different caches and is not \
                     an authority over this one",
                    truncation.seen_len(),
                    truncation.published_len(),
                    index.cache_sha256,
                ),
                None => format!(
                    "the checkpoint cache is byte-identical to the one the index is bound to, \
                     yet {} of its records do not match the per-record digests the same index \
                     publishes; the index describes two different caches, so it is not an \
                     authority over this one",
                    verdict.damaged_positions().len(),
                ),
            }))
        }
    }
    let confined = index.confine_damaged_records(&verdict);
    Ok((index, confined))
}

/// Owned, once-authenticated cache suitable for repeated product seeks.
///
/// Construction hashes and binds the cache exactly once. Individual seeks
/// still validate record structure, target identity, parity SHA, and the
/// configured delta bound.
#[derive(Clone)]
pub struct CertifiedReplayCache {
    index: ReplayHourIndex,
    cache: CacheBacking,
    /// The hours this cache cannot serve because a record they replay through
    /// rotted after sealing. Empty for an intact cache, which is every cache
    /// the recorder writes.
    confined: Vec<ConfinedHour>,
}

impl CertifiedReplayCache {
    pub fn open(
        index_bytes: &[u8],
        cache: Vec<u8>,
        expected_source_len: u64,
        expected_source_sha256: &str,
    ) -> Result<Self, ReplayCacheError> {
        let cache: Arc<[u8]> = Arc::from(cache);
        let (index, confined) = admit_cache(
            index_bytes,
            &SliceCache(cache.as_ref()),
            expected_source_len,
            expected_source_sha256,
        )?;
        Ok(Self {
            index,
            cache: CacheBacking::Memory(cache),
            confined,
        })
    }

    /// Same admission, same seeks, but the cache stays on disk. This is the
    /// only shape that works once a campaign's cache outgrows addressable
    /// working memory, and it is what the seal path and the container readers
    /// use for real captures.
    pub fn open_file(
        index_bytes: &[u8],
        cache: File,
        expected_source_len: u64,
        expected_source_sha256: &str,
    ) -> Result<Self, ReplayCacheError> {
        let cache = CertifiedCacheFile::new(cache)?;
        let (index, confined) = admit_cache(
            index_bytes,
            &cache,
            expected_source_len,
            expected_source_sha256,
        )?;
        Ok(Self {
            index,
            cache: CacheBacking::File(Arc::new(cache)),
            confined,
        })
    }

    pub fn index(&self) -> &ReplayHourIndex {
        &self.index
    }

    /// The hours this cache refuses because of post-seal record damage, in hour
    /// order. See [`ConfinedHour`].
    pub fn confined_hours(&self) -> &[ConfinedHour] {
        &self.confined
    }

    pub fn reconstruct_json(
        &self,
        hour_index: i64,
    ) -> Result<(Vec<u8>, ReplaySeekReceipt), ReplayCacheError> {
        refuse_confined_hour(&self.confined, hour_index)?;
        self.cache
            .with_records(|records| reconstruct_json_from(&self.index, records, hour_index))
    }

    /// See [`ReplayCacheReader::verify_every_hour`]. Identical proof, over
    /// whichever backing this cache was opened with.
    pub fn verify_every_hour(&self) -> Result<u32, ReplayCacheError> {
        refuse_confinement_in_a_total_proof(&self.confined)?;
        self.cache
            .with_records(|records| verify_every_hour_from(&self.index, records))
    }
}

/// Refuse one hour BY NAME because a record it replays through is damaged.
///
/// kills: deleting this call from `reconstruct_json` serves a confined hour out
/// of a cache whose record for it no longer hashes to the sealed digest — the
/// leak the confinement exists to prevent. The refusal carries the sealed
/// reason verbatim so a consumer reads the same sentence the replay authority's
/// coverage row carries.
fn refuse_confined_hour(
    confined: &[ConfinedHour],
    hour_index: i64,
) -> Result<(), ReplayCacheError> {
    if let Ok(position) = confined.binary_search_by_key(&hour_index, ConfinedHour::hour_index) {
        return Err(ReplayCacheError::new(format!(
            "game hour {hour_index} is not served by this recording: {}",
            confined[position].reason()
        )));
    }
    Ok(())
}

/// Refuse a TOTAL proof over a cache that cannot serve every hour it indexes,
/// BY NAME and before the walk starts.
///
/// `verify_every_hour` is the producer's proof — the seal path runs it before
/// it publishes artifacts — and its claim is "every indexed hour reconstructs".
/// A confined hour makes that claim false, so it is refused here rather than
/// quietly narrowed.
///
/// kills: deleting this costs the DIAGNOSIS, not the refusal, and the
/// difference is worth stating precisely (Codex review round 3, finding 1).
/// The walk below would still fail on a moved record — it decodes every record
/// and compares every reconstructed hour against its parity digest — and
/// `spool_trim` refuses on `confined_hours()` of its own accord. What this
/// precheck buys is that the producer is told *which hours the cache cannot
/// serve and why*, instead of being handed whatever the decoder made of the
/// first damaged payload, which for a flipped byte is usually a spurious
/// complaint about the value model. `cache_and_index_corruption_fail_closed`
/// asserts that message, not merely the refusal.
fn refuse_confinement_in_a_total_proof(
    confined: &[ConfinedHour],
) -> Result<(), ReplayCacheError> {
    let Some(first) = confined.first() else {
        return Ok(());
    };
    Err(ReplayCacheError::new(format!(
        "this checkpoint cache cannot serve {} of the hours it indexes, so no proof over every \
         indexed hour is available; the first is game hour {}: {}",
        confined.len(),
        first.hour_index(),
        first.reason()
    )))
}

impl<'a> ReplayCacheReader<'a> {
    pub fn open(
        index_bytes: &[u8],
        cache: &'a [u8],
        expected_source_len: u64,
        expected_source_sha256: &str,
    ) -> Result<Self, ReplayCacheError> {
        let (index, confined) = admit_cache(
            index_bytes,
            &SliceCache(cache),
            expected_source_len,
            expected_source_sha256,
        )?;
        Ok(Self {
            index,
            cache,
            confined,
        })
    }

    pub fn index(&self) -> &ReplayHourIndex {
        &self.index
    }

    /// The hours this cache refuses because of post-seal record damage, in hour
    /// order. See [`ConfinedHour`].
    pub fn confined_hours(&self) -> &[ConfinedHour] {
        &self.confined
    }

    pub fn reconstruct_json(
        &self,
        hour_index: i64,
    ) -> Result<(Vec<u8>, ReplaySeekReceipt), ReplayCacheError> {
        refuse_confined_hour(&self.confined, hour_index)?;
        reconstruct_json_from(&self.index, &SliceCache(self.cache), hour_index)
    }

    /// Prove every indexed hour reconstructs to its recorded bytes, and return
    /// the deepest seek the index asks a reader to perform.
    ///
    /// This is the whole-artifact certification the seal path runs once. It
    /// computes exactly the sequence of values a per-hour
    /// [`Self::reconstruct_json`] sweep would — decode a checkpoint, then apply
    /// that window's deltas in order — but it carries the running value forward
    /// instead of rebuilding it from the checkpoint for every hour, so the pass
    /// is linear in hours rather than hours times the checkpoint interval.
    ///
    /// That is not a cosmetic difference. On capture t26 (644 hours, interval
    /// 168, ~30 MB per hour) the rebuilding form did not finish in 46 minutes;
    /// it asks for ~54,000 patch applications where ~640 carry the same proof.
    pub fn verify_every_hour(&self) -> Result<u32, ReplayCacheError> {
        refuse_confinement_in_a_total_proof(&self.confined)?;
        verify_every_hour_from(&self.index, &SliceCache(self.cache))
    }
}

fn verify_every_hour_from(
    index: &ReplayHourIndex,
    cache: &dyn CacheRecords,
) -> Result<u32, ReplayCacheError> {
    let mut value: Option<CanonValue> = None;
    let mut shadow = Shadow::default();
    let mut budget = ShadowBudget::default();
    let mut max_seek_deltas = 0u32;
    for (position, entry) in index.hours.iter().enumerate() {
        let (kind, record_hour, payload) = cache_record(cache, entry)?;
        if record_hour != entry.hour_index {
            return Err(ReplayCacheError::new(format!(
                "cache record at position {position} is not the indexed hour {}",
                entry.hour_index
            )));
        }
        match (kind, entry.delta_steps) {
            (RECORD_CHECKPOINT, 0) => {
                value = Some(decode_value_exact(payload.as_ref())?);
                shadow = Shadow::default();
                budget = ShadowBudget::default();
            }
            (RECORD_DELTA, steps) if steps != 0 => {
                let running = value.as_mut().ok_or_else(|| {
                    ReplayCacheError::new("a delta record precedes its checkpoint")
                })?;
                apply_patch(
                    running,
                    &decode_patch_exact(payload.as_ref())?,
                    &mut shadow,
                    &mut budget,
                    0,
                )?;
            }
            _ => {
                return Err(ReplayCacheError::new(format!(
                    "cache record kind at position {position} disagrees with its delta distance"
                )))
            }
        }
        drop(payload);
        let running = value.as_ref().expect("a value exists once a checkpoint decoded");
        validate_world_identity(running, entry.hour_index, &entry.date)?;
        if Sha256::hex(&running.to_json_bytes()) != entry.world_json_sha256 {
            return Err(ReplayCacheError::new(format!(
                "reconstructed hour {} does not byte-match replay_decoder parity hash",
                entry.hour_index
            )));
        }
        max_seek_deltas = max_seek_deltas.max(entry.delta_steps);
    }
    if max_seek_deltas >= index.checkpoint_interval_hours {
        return Err(ReplayCacheError::new(
            "reconstructed seek exceeded its certified checkpoint bound",
        ));
    }
    Ok(max_seek_deltas)
}

fn reconstruct_json_from(
    index: &ReplayHourIndex,
    cache: &dyn CacheRecords,
    hour_index: i64,
) -> Result<(Vec<u8>, ReplaySeekReceipt), ReplayCacheError> {
    let target_position = index
        .hours
        .binary_search_by_key(&hour_index, ReplayHourEntry::hour_index)
        .map_err(|_| ReplayCacheError::new(format!("hour {hour_index} is not indexed")))?;
    let target = &index.hours[target_position];
    let checkpoint_position = target.checkpoint_position as usize;
    let checkpoint = &index.hours[checkpoint_position];
    let (kind, checkpoint_hour, payload) = cache_record(cache, checkpoint)?;
    if kind != RECORD_CHECKPOINT || checkpoint_hour != checkpoint.hour_index {
        return Err(ReplayCacheError::new(
            "checkpoint index points at a non-checkpoint cache record",
        ));
    }
    let mut value = decode_value_exact(payload.as_ref())?;
    drop(payload);
    // Rebuilt from the checkpoint forward, which is the whole reason a revive
    // may only ever name a value recorded since one.
    let mut shadow = Shadow::default();
    let mut budget = ShadowBudget::default();
    for entry in &index.hours[checkpoint_position + 1..=target_position] {
        let (kind, record_hour, payload) = cache_record(cache, entry)?;
        if kind != RECORD_DELTA || record_hour != entry.hour_index {
            return Err(ReplayCacheError::new(
                "delta index points at an invalid cache record",
            ));
        }
        let patch = decode_patch_exact(payload.as_ref())?;
        drop(payload);
        apply_patch(&mut value, &patch, &mut shadow, &mut budget, 0)?;
    }
    validate_world_identity(&value, target.hour_index, &target.date)?;
    let json = value.to_json_bytes();
    if Sha256::hex(&json) != target.world_json_sha256 {
        return Err(ReplayCacheError::new(format!(
            "reconstructed hour {} does not byte-match replay_decoder parity hash",
            target.hour_index
        )));
    }
    let receipt = ReplaySeekReceipt {
        checkpoint_hour: checkpoint.hour_index,
        target_hour: target.hour_index,
        applied_deltas: target.delta_steps,
        configured_bound: index.checkpoint_interval_hours - 1,
    };
    if receipt.applied_deltas > receipt.configured_bound {
        return Err(ReplayCacheError::new(
            "reconstructed seek exceeded its certified checkpoint bound",
        ));
    }
    Ok((json, receipt))
}

/// Read one indexed record's kind, hour and payload.
///
/// The 21-byte record header is read and agreed with the index BEFORE the
/// payload is requested, so a record whose self-declared payload length
/// disagrees with the length H4IX published is refused without ever allocating
/// for it. That agreement used to be proved after the whole record was already
/// in hand; it is the same invariant, checked one allocation earlier.
fn cache_record<'a>(
    cache: &'a dyn CacheRecords,
    entry: &ReplayHourEntry,
) -> Result<(u8, i64, Cow<'a, [u8]>), ReplayCacheError> {
    if entry.cache_record_len > MAX_CACHE_RECORD_BYTES {
        return Err(ReplayCacheError::new(format!(
            "cache record length {} exceeds the {MAX_CACHE_RECORD_BYTES}-byte per-record reader bound",
            entry.cache_record_len
        )));
    }
    let header = cache.record(entry.cache_record_offset, RECORD_HEADER_LEN)?;
    let mut reader = ByteReader::new(header.as_ref());
    let kind = reader.u8()?;
    if reader.take(3)? != [0, 0, 0] {
        return Err(ReplayCacheError::new(
            "cache record reserved bytes are nonzero",
        ));
    }
    let hour = reader.i64()?;
    let payload_len = reader.u64()?;
    if reader.u8()? != 0 {
        return Err(ReplayCacheError::new(
            "cache record trailing reserved byte is nonzero",
        ));
    }
    reader.finish()?;
    if payload_len.checked_add(RECORD_HEADER_LEN) != Some(entry.cache_record_len) {
        return Err(ReplayCacheError::new(format!(
            "cache record declares a {payload_len}-byte payload but the index published a {}-byte record",
            entry.cache_record_len
        )));
    }
    // The header read already proved `offset + 21 <= cache_len`, so this cannot
    // overflow; it is spelled `checked_add` anyway so the proof does not have to
    // be re-derived from two functions away.
    let payload_offset = entry
        .cache_record_offset
        .checked_add(RECORD_HEADER_LEN)
        .ok_or_else(|| ReplayCacheError::new("cache record range is out of bounds"))?;
    let payload = cache.record(payload_offset, payload_len)?;
    Ok((kind, hour, payload))
}

#[derive(Debug, Clone)]
enum Patch {
    Keep,
    Replace(CanonValue),
    Object {
        removals: Vec<String>,
        changes: Vec<(String, Patch)>,
    },
    Array(Vec<(u32, Patch)>),
    /// The target is an array of a DIFFERENT length, spelled as an edit script
    /// over it. See [`PATCH_ARRAY_EDIT`] and [`array_edit_between`].
    ArrayEdit(Vec<ArrayEditOp>),
    /// The target is a number and becomes another number, spelled as the
    /// difference. See [`PATCH_NUMBER_DELTA`] and [`number_delta_between`].
    NumberDelta {
        fraction_digits: u32,
        delta: i64,
    },
    /// The target is `null` and becomes exactly its own remembered value.
    ReviveSame,
    /// The target is `null` and becomes its remembered value with a patch on
    /// top. The inner patch is always an object or array patch: if the
    /// remembered value were not structurally compatible with the new one there
    /// would be nothing to diff and the encoder emits a plain `Replace`.
    Revive(Box<Patch>),
}

/// One step of a [`Patch::ArrayEdit`] script. See [`PATCH_ARRAY_EDIT`].
#[derive(Debug, Clone)]
enum ArrayEditOp {
    Copy(u32),
    Skip(u32),
    Patch(Patch),
    Insert(CanonValue),
}

/// What each currently-`null` path last held, for as long as it stays null.
///
/// SPARSE, AND ONLY THE DISAPPEARED. A path enters the shadow at the moment it
/// goes null and leaves the moment it is non-null again, so this holds the
/// folded-out part of the world and nothing else -- not a second copy of it.
/// Children under a remembered value are dropped with it, because the
/// remembered subtree already contains them.
///
/// Encoder and decoder both maintain it from `(value before the patch, patch)`
/// through the SAME function, [`apply_patch`], walked over the same records in
/// the same order, and it is rebuilt from empty at every checkpoint so a seek
/// that starts there derives exactly what the encoder had. Any disagreement
/// would decode some hour to different bytes, which is what
/// `verify_every_hour` already checks against `replay_decoder`'s own hash for
/// every hour of every sealed cache.
#[derive(Debug, Default)]
struct Shadow {
    remembered: Option<CanonValue>,
    /// What [`ShadowBudget`] was charged for `remembered`, so releasing it
    /// refunds exactly that and never a recomputation that could drift.
    charge: u64,
    members: std::collections::HashMap<String, Shadow>,
    elements: std::collections::HashMap<u32, Shadow>,
}

/// The ceiling on what one checkpoint window may remember, and what it is
/// holding right now.
///
/// WITHOUT THIS THE SHADOW IS UNBOUNDED. A window is up to 744 hours; a
/// recording that nulls a different large member every hour and leaves it null
/// would accumulate a remembered subtree per hour, so the memory would grow
/// with the WINDOW rather than with the world -- hundreds of times the size of
/// the document it describes (Codex review round 6, finding S4).
///
/// IT IS A BALANCE, NOT A METER. `spent` is the content of the memories that
/// are LIVE at this instant: every charge is refunded at the moment its memory
/// is dropped -- spent on a revive, overwritten by a new memory, cleared
/// because the path became non-null, or removed with its member. The first cut
/// of this never refunded, on the argument that an upper bound was all the cap
/// needed to be; measured on capture t53 that argument cost the whole lever.
/// A domain that folds out every fourth hour is charged every fourth hour, and
/// on t53 the 256 MiB cap was exhausted at position 61 of a 168-hour window
/// (`runs/lz_size_batch2/CADENCE_t53.txt`): from there to the end of the window
/// EVERY revive was declined -- 1,326 memories, 1.15 GB of them, including 41
/// of `map._r2_weather_live`'s 57 fold-outs -- and each declined revive is a
/// whole subtree re-emitted in full.
///
/// A balance is bounded by the WORLD and not by the window, which is what the
/// cap was really for: the memories live at one instant sit at disjoint paths
/// (a memory's own descendants are released when it is taken), so their total
/// is at most the content of one world.
///
/// It is a SIZE lever with no correctness edge: while the cap is reached the
/// encoder simply stops offering revives and spends full replaces instead.
/// Encoder and reader run this same rule over the same records in the same
/// order -- charges and refunds alike are driven by [`apply_patch`], which both
/// sides call -- so they hold the same balance at every hour and never disagree
/// about whether a memory exists.
#[derive(Debug)]
struct ShadowBudget {
    spent: u64,
    cap: u64,
}

/// 256 MiB. The encoder already holds a whole world (34.5 MB on capture t30)
/// plus the hour it is encoding, and the ship floor is a 4-core/16 GB machine,
/// so this leaves room for many simultaneous folded-out domains while keeping
/// the worst case a fixed number rather than a multiple of the window length.
const MAX_SHADOW_BYTES: u64 = 256 * 1024 * 1024;

impl Default for ShadowBudget {
    fn default() -> Self {
        ShadowBudget {
            spent: 0,
            cap: MAX_SHADOW_BYTES,
        }
    }
}

impl ShadowBudget {
    /// Give back what a dropped memory was charged.
    ///
    /// `saturating_sub` rather than a panic on underflow: the charge refunded
    /// is the charge recorded beside the memory, so the balance cannot go
    /// negative unless the two drifted, and a size lever must never be able to
    /// refuse a recording.
    fn refund(&mut self, charge: u64) {
        self.spent = self.spent.saturating_sub(charge);
    }

    /// Charge for a memory and report what it cost, or decline it. Declining
    /// costs bytes, never truth.
    fn admits(&mut self, value: &CanonValue) -> Option<u64> {
        // CONTENT, NEVER CAPACITY. Both sides must reach the same verdict from
        // the same document, and `heap_bytes` cannot deliver that: the JSON
        // parser doubles its vectors while the binary decoder reserves exactly,
        // so equal values carry different capacities on the two sides. Charging
        // that made admission depend on allocation history, and near the cap one
        // side kept a memory the other had refused -- which is a revive the
        // reader cannot resolve (Codex review round 7, Q1).
        let cost = value.content_bytes();
        match self.spent.checked_add(cost) {
            Some(spent) if spent <= self.cap => {
                self.spent = spent;
                Some(cost)
            }
            _ => None,
        }
    }
}

impl Shadow {
    fn member(&self, key: &str) -> Option<&Shadow> {
        self.members.get(key)
    }

    fn element(&self, index: u32) -> Option<&Shadow> {
        self.elements.get(&index)
    }

    fn is_empty(&self) -> bool {
        self.remembered.is_none() && self.members.is_empty() && self.elements.is_empty()
    }

    /// Record that this path has just gone null, holding `was`, if the window's
    /// budget still admits it.
    ///
    /// The children go with it EITHER WAY: they describe paths inside `was`,
    /// which the remembered subtree now carries itself, and if the budget
    /// declines the memory then those paths are gone from the world with
    /// nothing standing in for them. Releasing them is also what keeps the
    /// balance honest -- their charges are refunded here rather than stranded.
    fn remember(&mut self, was: CanonValue, budget: &mut ShadowBudget) {
        self.release(budget);
        match budget.admits(&was) {
            Some(charge) => {
                self.remembered = Some(was);
                self.charge = charge;
            }
            None => {
                self.remembered = None;
                self.charge = 0;
            }
        }
    }

    /// Take this path's memory, refunding what it was charged.
    ///
    /// TAKEN, not read: a revive spends the memory in the same step it uses it,
    /// so the balance stops holding it at exactly that moment.
    fn take_remembered(&mut self, budget: &mut ShadowBudget) -> Option<CanonValue> {
        let taken = self.remembered.take();
        if taken.is_some() {
            budget.refund(self.charge);
            self.charge = 0;
        }
        taken
    }

    /// Drop this subtree's memories and refund every one of them.
    ///
    /// The recursion is over the SHADOW, which holds only the folded-out paths
    /// -- not over the world.
    fn release(&mut self, budget: &mut ShadowBudget) {
        self.take_remembered(budget);
        for (_, mut child) in self.members.drain() {
            child.release(budget);
        }
        for (_, mut child) in self.elements.drain() {
            child.release(budget);
        }
    }
}

/// Whether a remembered value can serve as a PATCH base for `new`.
///
/// Exactly the shapes [`diff_value`] can express structurally. If they do not
/// match there is no patched revive to emit, because the inner patch would be a
/// `Replace` -- one byte MORE than the plain replace it would wrap.
///
/// This does NOT decide whether a revive is available at all: a memory equal to
/// `new` is revivable whatever its type, including a scalar, and that case is
/// [`revive_is_mandatory`]'s first arm.
fn revive_base_fits(base: &CanonValue, new: &CanonValue) -> bool {
    match (base, new) {
        (CanonValue::Object(_), CanonValue::Object(_)) => true,
        (CanonValue::Array(base), CanonValue::Array(new)) => base.len() == new.len(),
        _ => false,
    }
}

/// Whether a null target with this memory MUST be spelled as a revive.
///
/// One delta, one encoding. A remembered value that is simply back unchanged is
/// one byte as `PATCH_REVIVE_SAME` however large it is -- including a scalar,
/// which the first cut of this excluded and thereby left `Replace(7)` and
/// `ReviveSame` both legal for the same delta (Codex review round 6, finding
/// S2). Both the encoder and `apply_patch`'s refusal read THIS function, so the
/// rule cannot drift between the side that writes and the side that checks.
fn revive_is_mandatory(base: &CanonValue, new: &CanonValue) -> bool {
    base == new || revive_base_fits(base, new)
}

/// A plain decimal token as (value scaled by `10^fraction_digits`, fraction
/// digits), or `None` if the token is not one.
///
/// PLAIN means what the fold actually mints for a fixed-point measurement:
/// an optional minus, decimal digits, and at most one decimal point. An
/// exponent form (`3e-05`), a token wider than [`MAX_NUMBER_DELTA_DIGITS`]
/// digits, or anything else is not eligible, and its change stays a `Replace`.
fn plain_decimal(token: &str) -> Option<(i128, u32)> {
    let (negative, body) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let (integer, fraction) = match body.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (body, ""),
    };
    if integer.is_empty() || !integer.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if body.contains('.') && (fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let digits = integer.len() + fraction.len();
    if digits > MAX_NUMBER_DELTA_DIGITS as usize {
        return None;
    }
    let mut scaled: i128 = 0;
    for byte in integer.bytes().chain(fraction.bytes()) {
        scaled = scaled * 10 + i128::from(byte - b'0');
    }
    Some((
        if negative { -scaled } else { scaled },
        fraction.len() as u32,
    ))
}

/// Spell `scaled / 10^fraction_digits` with exactly `fraction_digits` after the
/// point, which is the one form a number delta ever reconstructs.
fn spell_decimal(scaled: i128, fraction_digits: u32) -> String {
    let negative = scaled < 0;
    let digits = scaled.unsigned_abs().to_string();
    let body = if fraction_digits == 0 {
        digits
    } else {
        let width = fraction_digits as usize + 1;
        let padded = if digits.len() < width {
            format!("{}{digits}", "0".repeat(width - digits.len()))
        } else {
            digits
        };
        let split = padded.len() - fraction_digits as usize;
        format!("{}.{}", &padded[..split], &padded[split..])
    };
    if negative {
        format!("-{body}")
    } else {
        body
    }
}

fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// THE ONE RULE BOTH SIDES READ: what a number delta from `old` to `new` is,
/// and whether it is admissible at all.
///
/// `diff_value` calls it to choose the spelling and `apply_patch` calls it to
/// refuse any other, so a delta series has exactly one encoding and therefore
/// one cache SHA -- the same discipline [`revive_is_mandatory`] keeps for the
/// revive. Three conditions, all of them checked here:
///
///   * both values are plain decimal tokens, and rescaling them to the wider
///     fraction width does not overflow;
///   * respelling the reconstruction with the NEW token's fraction width gives
///     that token BYTE FOR BYTE -- which is what rejects `-0.0`, a leading-zero
///     spelling, and anything else whose bytes the arithmetic cannot restore;
///   * the delta is strictly SHORTER than the `Replace` it stands in for. A
///     tie would leave two legal spellings of one change.
fn number_delta_between(old: &CanonValue, new: &CanonValue) -> Option<(u32, i64)> {
    let (CanonValue::Number(old_token), CanonValue::Number(new_token)) = (old, new) else {
        return None;
    };
    let (old_scaled, old_fraction) = plain_decimal(old_token)?;
    let (new_scaled, new_fraction) = plain_decimal(new_token)?;
    if spell_decimal(new_scaled, new_fraction) != *new_token {
        return None;
    }
    let wider = old_fraction.max(new_fraction);
    let old_at_wider = rescale(old_scaled, wider - old_fraction)?;
    let new_at_wider = rescale(new_scaled, wider - new_fraction)?;
    let delta = i64::try_from(new_at_wider.checked_sub(old_at_wider)?).ok()?;
    // A zero delta at the same width is `Keep`'s job, and `diff_value` never
    // reaches here for equal values; refusing it keeps the decoder honest.
    if delta == 0 && old_fraction == new_fraction {
        return None;
    }
    let cost = 1 + varint_len(u64::from(new_fraction)) + varint_len(zigzag(delta));
    let replace_cost = 1 + encoded_number_len(new_token)?;
    (cost < replace_cost).then_some((new_fraction, delta))
}

/// Bytes [`encode_value_into`] spends on this number token, without building
/// them. Called once per changed scalar on the encode path AND again on the
/// apply path for every replacement, so it does not allocate; it mirrors that
/// function's number arm exactly and drifting from it would let one side price
/// a spelling the other does not.
fn encoded_number_len(token: &str) -> Option<usize> {
    Some(match canonical_integer(token) {
        Some(CanonicalInteger::NonNegative(value)) if value <= INLINE_UINT_MAX => 1,
        Some(CanonicalInteger::NonNegative(value)) => 1 + varint_len(value),
        Some(CanonicalInteger::Negative(magnitude)) => 1 + varint_len(magnitude),
        // `put_text` refuses a body past the per-string bound, so a token that
        // long has no encoded length to report. Unreachable through
        // `number_delta_between` -- `plain_decimal` stops at
        // `MAX_NUMBER_DELTA_DIGITS` -- but the contract above says EVERY token,
        // and a helper that is total only by luck stops being total the moment
        // someone reuses it (Codex round 2, P3).
        None if token.len() > MAX_STRING_BYTES => return None,
        None => 1 + varint_len(u64::try_from(token.len()).ok()?) + token.len(),
    })
}

fn rescale(value: i128, places: u32) -> Option<i128> {
    value.checked_mul(10i128.checked_pow(places)?)
}

fn varint_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// The member name both arrays can be ALIGNED by, or `None` if they carry no
/// such identity.
///
/// DISCOVERED, NEVER NAMED. The recorder folds mod content as readily as
/// vanilla content, so a format that hard-coded `div_id` would align vanilla
/// divisions and give up on anything a mod adds. What this looks for is the
/// property an identity actually has: a member that EVERY row of both arrays
/// carries, whose value is a scalar, and which no two rows of the same array
/// share. The first such name in member order is taken, so both arrays -- and
/// any later re-derivation of this choice -- reach the same one.
///
/// A row without it is not an error; the arrays simply have no alignment and
/// the change stays a `Replace`.
///
/// PICKING THE WRONG MEMBER COSTS BYTES, NEVER TRUTH. Any member that happens
/// to be unique in both arrays qualifies, and one that is not really an
/// identity -- a measurement whose values happen not to repeat -- simply
/// produces a worse alignment: more inserts and skips, a longer script, and
/// then `diff_value` keeps the `Replace` instead because it is shorter. The
/// reconstruction is right either way, because the script is built by walking
/// the NEW array and every row is either taken from the old array and patched
/// until it equals the new one, or inserted whole.
fn array_alignment_key(old: &[CanonValue], new: &[CanonValue]) -> Option<String> {
    fn scalar_members(value: &CanonValue) -> Option<std::collections::HashSet<&str>> {
        let CanonValue::Object(members) = value else {
            return None;
        };
        Some(
            members
                .iter()
                .filter(|(_, value)| {
                    matches!(value, CanonValue::Number(_) | CanonValue::String(_))
                })
                .map(|(name, _)| name.as_str())
                .collect(),
        )
    }

    // A SET, not a scan: the arrays this matters on are thousands of rows of
    // dozens of members each, and an intersection by linear search over every
    // row is the difference between a walk and a stall at finalize.
    let mut candidates: Vec<String> = scalar_members(old.first()?)?
        .into_iter()
        .map(str::to_string)
        .collect();
    for row in old.iter().chain(new.iter()) {
        let present = scalar_members(row)?;
        candidates.retain(|name| present.contains(name.as_str()));
        if candidates.is_empty() {
            return None;
        }
    }
    candidates.sort_by(|left, right| key_cmp(left, right));
    candidates.into_iter().find(|name| {
        [old, new].iter().all(|rows| {
            let mut seen = std::collections::HashSet::with_capacity(rows.len());
            rows.iter()
                .all(|row| row_key(row, name).is_some_and(|value| seen.insert(value)))
        })
    })
}

/// The alignment key's value on one row, if the row carries it as a scalar.
///
/// Returned as (is a string, its token) rather than as the value, so identical
/// tokens of different types -- the number `7` and the string `"7"` -- are two
/// different identities and can key a map.
fn row_key<'value>(row: &'value CanonValue, name: &str) -> Option<(bool, &'value str)> {
    let CanonValue::Object(members) = row else {
        return None;
    };
    members
        .iter()
        .find(|(member, _)| member == name)
        .and_then(|(_, value)| match value {
            CanonValue::Number(token) => Some((false, token.as_str())),
            CanonValue::String(token) => Some((true, token.as_str())),
            _ => None,
        })
}

/// THE ONE RULE BOTH SIDES READ for an array whose length changed: the edit
/// script that turns `old` into `new`, or `None` if there is no alignment to
/// build one from.
///
/// The script is produced in NEW order with a monotone cursor over the old
/// array, so it is a left-to-right edit and never revisits a row it has passed:
/// a row whose key matches one still ahead of the cursor is copied or patched,
/// every old row the cursor steps over is skipped, and a row whose key the old
/// array does not have (or has already passed) is inserted whole. Adjacent
/// copies and adjacent skips are merged, so the script has exactly one shape
/// for one pair of arrays.
fn array_edit_between(
    old: &[CanonValue],
    new: &[CanonValue],
    shadow: Option<&Shadow>,
    depth: usize,
) -> Result<Option<Vec<ArrayEditOp>>, ReplayCacheError> {
    let Some(key) = array_alignment_key(old, new) else {
        return Ok(None);
    };
    let old_at: std::collections::HashMap<(bool, &str), usize> = old
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row_key(row, &key).map(|value| (value, index)))
        .collect();

    let mut ops: Vec<ArrayEditOp> = Vec::new();
    let mut cursor = 0usize;
    // Adjacent runs are merged as they are appended, which is what makes the
    // script canonical enough for the decoder to refuse a longer spelling of it.
    fn push_copy(ops: &mut Vec<ArrayEditOp>, count: u32) {
        match ops.last_mut() {
            Some(ArrayEditOp::Copy(run)) => *run += count,
            _ => ops.push(ArrayEditOp::Copy(count)),
        }
    }
    fn push_skip(ops: &mut Vec<ArrayEditOp>, count: u32) {
        match ops.last_mut() {
            Some(ArrayEditOp::Skip(run)) => *run += count,
            _ => ops.push(ArrayEditOp::Skip(count)),
        }
    }

    for row in new {
        let matched = row_key(row, &key)
            .and_then(|value| old_at.get(&value).copied())
            .filter(|index| *index >= cursor);
        match matched {
            Some(index) => {
                if index > cursor {
                    push_skip(&mut ops, count_u32(index - cursor)?);
                }
                if old[index] == *row {
                    push_copy(&mut ops, 1);
                } else {
                    let element = count_u32(index)?;
                    ops.push(ArrayEditOp::Patch(diff_value(
                        &old[index],
                        row,
                        shadow.and_then(|shadow| shadow.element(element)),
                        depth + 1,
                    )?));
                }
                cursor = index + 1;
            }
            None => ops.push(ArrayEditOp::Insert(row.clone())),
        }
    }
    if cursor < old.len() {
        push_skip(&mut ops, count_u32(old.len() - cursor)?);
    }
    // An edit that produces nothing is a `Replace` with an empty array and
    // always shorter that way; an edit that neither skips nor inserts is a
    // `Patch::Array`, which cannot happen here because the lengths differ, but
    // the decoder refuses both shapes and the encoder must not mint them.
    if new.is_empty()
        || !ops
            .iter()
            .any(|op| matches!(op, ArrayEditOp::Skip(_) | ArrayEditOp::Insert(_)))
    {
        return Ok(None);
    }
    Ok(Some(ops))
}

fn diff_value(
    old: &CanonValue,
    new: &CanonValue,
    shadow: Option<&Shadow>,
    depth: usize,
) -> Result<Patch, ReplayCacheError> {
    require_depth(depth)?;
    if old == new {
        return Ok(Patch::Keep);
    }
    // THE REVIVE IS TRIED FIRST, and it is MANDATORY where it applies -- the
    // decoder refuses a plain replace over a null whose remembered value fits
    // (see `apply_patch`), because two spellings of one delta would mean two
    // cache SHAs for one recording.
    if matches!(old, CanonValue::Null) {
        if let Some(base) = shadow.and_then(|shadow| shadow.remembered.as_ref()) {
            if revive_is_mandatory(base, new) {
                // No nested shadow: `Shadow::remember` dropped the children when
                // it took this subtree, precisely so the base is self-contained.
                return Ok(match diff_value(base, new, None, depth + 1)? {
                    Patch::Keep => Patch::ReviveSame,
                    inner => Patch::Revive(Box::new(inner)),
                });
            }
        }
    }
    // A NUMBER CHANGING TO ANOTHER NUMBER IS SPELLED AS THE DIFFERENCE where
    // that is shorter, and `apply_patch` refuses the `Replace` it displaces --
    // one delta, one encoding, exactly as for the revive above.
    if let Some((fraction_digits, delta)) = number_delta_between(old, new) {
        return Ok(Patch::NumberDelta {
            fraction_digits,
            delta,
        });
    }
    match (old, new) {
        (CanonValue::Object(old), CanonValue::Object(new)) => {
            // Both member lists are strictly increasing, so one merge walk
            // finds every removal, addition and change.
            let mut removals = Vec::new();
            let mut changes = Vec::new();
            let (mut left, mut right) = (0usize, 0usize);
            while left < old.len() || right < new.len() {
                let order = match (old.get(left), new.get(right)) {
                    (Some((old_key, _)), Some((new_key, _))) => key_cmp(old_key, new_key),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => unreachable!("the merge walk ends when both sides are drained"),
                };
                match order {
                    Ordering::Less => {
                        removals.push(old[left].0.clone());
                        left += 1;
                    }
                    Ordering::Greater => {
                        changes.push((new[right].0.clone(), Patch::Replace(new[right].1.clone())));
                        right += 1;
                    }
                    Ordering::Equal => {
                        let patch = diff_value(
                            &old[left].1,
                            &new[right].1,
                            shadow.and_then(|shadow| shadow.member(&new[right].0)),
                            depth + 1,
                        )?;
                        if !matches!(patch, Patch::Keep) {
                            changes.push((new[right].0.clone(), patch));
                        }
                        left += 1;
                        right += 1;
                    }
                }
            }
            Ok(Patch::Object { removals, changes })
        }
        (CanonValue::Array(old), CanonValue::Array(new)) if old.len() == new.len() => {
            let mut changes = Vec::new();
            for (index, (old_value, new_value)) in old.iter().zip(new).enumerate() {
                let element = u32::try_from(index)
                    .map_err(|_| ReplayCacheError::new("array index does not fit u32"))?;
                let patch = diff_value(
                    old_value,
                    new_value,
                    shadow.and_then(|shadow| shadow.element(element)),
                    depth + 1,
                )?;
                if !matches!(patch, Patch::Keep) {
                    changes.push((element, patch));
                }
            }
            Ok(Patch::Array(changes))
        }
        // AN ARRAY WHOSE LENGTH CHANGED IS ALIGNED BY ITS ROWS' OWN IDENTITY
        // where that is shorter than restating it. The comparison is against
        // the `Replace` this displaces and it is made on the ENCODED bytes,
        // because that is the thing the owner set a bar on; a script that is
        // not shorter is not written, so `apply_patch` can refuse it.
        (CanonValue::Array(old_rows), CanonValue::Array(new_rows)) => {
            if let Some(ops) = array_edit_between(old_rows, new_rows, shadow, depth)? {
                let edit = Patch::ArrayEdit(ops);
                let replace = Patch::Replace(new.clone());
                if encoded_patch_len(&edit, depth)? < encoded_patch_len(&replace, depth)? {
                    return Ok(edit);
                }
            }
            Ok(Patch::Replace(new.clone()))
        }
        _ => Ok(Patch::Replace(new.clone())),
    }
}

/// Apply `patch` to `target`, AND carry the [`Shadow`] forward across the same
/// step.
///
/// The two are one function on purpose. The shadow update needs the value
/// BEFORE the patch and the patch together, which is exactly this moment and
/// nowhere else; splitting them would mean either keeping a second copy of the
/// world or writing the rule twice and hoping the copies agree. The encoder
/// drives this over the previous hour it is about to discard, so both sides run
/// this code and no second implementation exists to drift.
fn apply_patch(
    target: &mut CanonValue,
    patch: &Patch,
    shadow: &mut Shadow,
    budget: &mut ShadowBudget,
    depth: usize,
) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match patch {
        Patch::Keep => Ok(()),
        Patch::NumberDelta {
            fraction_digits,
            delta,
        } => {
            let CanonValue::Number(old_token) = &*target else {
                return Err(ReplayCacheError::new(
                    "number delta patch targets a value that is not a number",
                ));
            };
            let (old_scaled, old_fraction) = plain_decimal(old_token).ok_or_else(|| {
                ReplayCacheError::new("number delta patch targets a number it cannot join")
            })?;
            let wider = old_fraction.max(*fraction_digits);
            let reconstructed = rescale(old_scaled, wider - old_fraction)
                .and_then(|old_at_wider| old_at_wider.checked_add(i128::from(*delta)))
                .ok_or_else(|| ReplayCacheError::new("number delta patch overflows its target"))?;
            let narrowing = 10i128
                .checked_pow(wider - *fraction_digits)
                .ok_or_else(|| ReplayCacheError::new("number delta patch fraction width is out of range"))?;
            if reconstructed % narrowing != 0 {
                return Err(ReplayCacheError::new(
                    "number delta patch does not land on its stated fraction width",
                ));
            }
            let value = CanonValue::Number(spell_decimal(reconstructed / narrowing, *fraction_digits));
            // THE SINGLE-ENCODING LAW, checked at the target rather than
            // assumed from the bytes: this must be exactly the delta the
            // encoder would have chosen for this change, so no second spelling
            // of one delta series can decode.
            if number_delta_between(target, &value) != Some((*fraction_digits, *delta)) {
                return Err(ReplayCacheError::new(
                    "number delta patch is not the encoder's spelling of this change",
                ));
            }
            *target = value;
            // A number has no children and was not null, so nothing can be
            // remembered under it; released for the same reason `Replace`
            // releases rather than on evidence that anything is there.
            shadow.release(budget);
            Ok(())
        }
        Patch::ReviveSame | Patch::Revive(_) => {
            if !matches!(target, CanonValue::Null) {
                return Err(ReplayCacheError::new(
                    "revive patch targets a value that is not null",
                ));
            }
            // TAKEN, not read: the path is about to be non-null again, so the
            // memory is spent in the same step it is used. That is also what
            // frees the borrow the inner patch would otherwise contend for.
            let Some(base) = shadow.take_remembered(budget) else {
                return Err(ReplayCacheError::new(
                    "revive patch has no remembered value to revive",
                ));
            };
            let mut value = base;
            if let Patch::Revive(inner) = patch {
                // A revive whose inner patch is a keep is spelled
                // `PATCH_REVIVE_SAME`, and one whose base does not fit is
                // spelled as a plain replace; both would be second encodings.
                // A NUMBER DELTA CANNOT BE AN INNER PATCH EITHER: a scalar
                // memory is revivable only when it EQUALS the new value
                // (`revive_is_mandatory`), and that case is `ReviveSame`.
                if matches!(
                    **inner,
                    Patch::Keep | Patch::Replace(_) | Patch::NumberDelta { .. }
                ) {
                    return Err(ReplayCacheError::new(
                        "revive patch restates a delta with a shorter spelling",
                    ));
                }
                // THE REVIVED SUBTREE KEEPS WHATEVER ITS OWN PATCH REMEMBERS.
                // `shadow` is the right node for that and it is free to use:
                // the memory was TAKEN above, and its children were dropped
                // when it was remembered, so this starts empty and ends holding
                // exactly the paths the inner patch nulled. Handing the inner
                // application a throwaway `Shadow::default()` instead -- which
                // is what the first cut did -- silently discarded those, so a
                // member nulled by the same patch that revived its parent could
                // never be revived afterwards (Codex review round 6, S1).
                apply_patch(&mut value, inner, shadow, budget, depth + 1)?;
            }
            if matches!(value, CanonValue::Null) {
                return Err(ReplayCacheError::new(
                    "revive patch reconstructed a null value",
                ));
            }
            *target = value;
            Ok(())
        }
        Patch::Replace(value) => {
            // A replacement over an EXISTING value is only how `diff_value`
            // spells a change it cannot express structurally. Rejecting the
            // three shapes it would never emit is what makes one delta series
            // have exactly one encoding, and therefore one cache SHA.
            if value == target {
                return Err(ReplayCacheError::new(
                    "replacement patch does not change its target and must be omitted",
                ));
            }
            let structural = match (&*target, value) {
                (CanonValue::Object(_), CanonValue::Object(_)) => true,
                (CanonValue::Array(old), CanonValue::Array(new)) => old.len() == new.len(),
                _ => false,
            };
            if structural {
                return Err(ReplayCacheError::new(
                    "replacement patch restates a value the encoder would have diffed",
                ));
            }
            if number_delta_between(target, value).is_some() {
                return Err(ReplayCacheError::new(
                    "replacement patch restates a number change with a shorter spelling",
                ));
            }
            // A REPLACE OVER A NULL WHOSE MEMORY FITS IS A MISSED SAVING, NOT
            // AN ERROR, and this is the one place the canonicality law is
            // deliberately not enforced by refusal.
            //
            // Refusing it was tried and is wrong. `divisions` is an array and a
            // memory under it is keyed by INDEX, so when the array is replaced
            // wholesale -- which is how a length change is spelled -- the two
            // sides can legitimately reach a member with different memories
            // beneath it, and the encoder then spells as a replace what the
            // decoder could have revived. Measured: `.divisions[2969].equipment`
            // on t54. The refusal turned that into a hard round-trip failure.
            //
            // Nothing unsound gets through. A replace CARRIES ITS OWN VALUE, so
            // it reconstructs the same document whatever the shadow holds; only
            // a REVIVE depends on the memory, and a revive resolved against a
            // base the encoder did not intend produces different bytes, which
            // `verify_every_hour` already catches against replay_decoder's own
            // per-hour hash on every seal. So the invariant that matters -- one
            // recording, one reconstruction -- is enforced where it can be
            // enforced, and the one this cannot is a byte, not a fact.
            //
            // The residual, named for whoever prices the next lever: a
            // hand-built cache could spell such a member either way and both
            // decode to the same document. Only this encoder writes caches, and
            // its choice is deterministic, so one recording still has one SHA.
            let was = std::mem::replace(target, value.clone());
            if matches!(target, CanonValue::Null) {
                // The path has just gone null. Remember what it held, which is
                // the whole point of the structure.
                shadow.remember(was, budget);
            } else {
                // Non-null again, and the subtree it replaces is gone, so every
                // memory underneath it is stale.
                shadow.release(budget);
            }
            Ok(())
        }
        Patch::Object { removals, changes } => {
            let CanonValue::Object(members) = target else {
                return Err(ReplayCacheError::new(
                    "object patch targets a non-object value",
                ));
            };
            let child_shadow = &mut shadow.members;
            // Removals first, then changes, both by merge walk over the
            // strictly increasing member list — the result stays sorted, so a
            // later delta in the same run still sees a canonical object.
            let mut removed = 0usize;
            let mut kept = Vec::with_capacity(members.len());
            for member in std::mem::take(members) {
                if removals
                    .get(removed)
                    .is_some_and(|key| key_cmp(key, &member.0) == Ordering::Equal)
                {
                    // The encoder copies the member's own spelling, so a patch
                    // that names it some other decodable way is a second
                    // encoding of the same delta.
                    if removals[removed] != member.0 {
                        return Err(ReplayCacheError::new(
                            "object patch removal spells its member name differently from the target",
                        ));
                    }
                    // The member is gone, not nulled. Nothing may revive from
                    // a path the world no longer has.
                    if let Some(mut child) = child_shadow.remove(&member.0) {
                        child.release(budget);
                    }
                    removed += 1;
                    continue;
                }
                kept.push(member);
            }
            if removed != removals.len() {
                return Err(ReplayCacheError::new(
                    "object patch removes an absent member",
                ));
            }
            let mut out = Vec::with_capacity(kept.len() + changes.len());
            let mut remaining = kept.into_iter().peekable();
            for (key, change) in changes {
                while remaining
                    .peek()
                    .is_some_and(|(candidate, _)| key_cmp(candidate, key) == Ordering::Less)
                {
                    out.push(remaining.next().expect("peeked member"));
                }
                if remaining
                    .peek()
                    .is_some_and(|(candidate, _)| key_cmp(candidate, key) == Ordering::Equal)
                {
                    let (existing_key, mut existing) = remaining.next().expect("peeked member");
                    if *key != existing_key {
                        return Err(ReplayCacheError::new(
                            "object patch change spells its member name differently from the target",
                        ));
                    }
                    let mut child = child_shadow.remove(&existing_key).unwrap_or_default();
                    let applied = apply_patch(&mut existing, change, &mut child, budget, depth + 1);
                    if !child.is_empty() {
                        child_shadow.insert(existing_key.clone(), child);
                    }
                    applied?;
                    out.push((existing_key, existing));
                } else {
                    match change {
                        Patch::Replace(value) => {
                            // An ADDED member has no history at this path: any
                            // memory here belonged to something removed earlier
                            // and must not become a revive base.
                            if let Some(mut child) = child_shadow.remove(key) {
                                child.release(budget);
                            }
                            out.push((key.clone(), value.clone()));
                        }
                        _ => {
                            return Err(ReplayCacheError::new(
                                "object patch changes an absent non-replacement member",
                            ))
                        }
                    }
                }
            }
            out.extend(remaining);
            *members = out;
            Ok(())
        }
        Patch::ArrayEdit(ops) => {
            let CanonValue::Array(array) = target else {
                return Err(ReplayCacheError::new(
                    "array edit patch targets a non-array value",
                ));
            };
            let old = std::mem::take(array);
            // THE MEMORIES MOVE WITH THEIR ROWS. An element shadow is keyed by
            // INDEX, and this is the one patch that changes what index a row
            // sits at, so a copied or patched row carries its memory to the
            // slot it lands in and a skipped row's memory is released. Getting
            // this wrong does not corrupt a document -- it strands a revive
            // base -- and a stranded base is a refused recording, loudly, at
            // `verify_every_hour`.
            let mut old_shadows = std::mem::take(&mut shadow.elements);
            let mut new_shadows = std::collections::HashMap::new();
            let mut out: Vec<CanonValue> = Vec::new();
            let mut old = old.into_iter();
            let mut cursor = 0usize;
            for op in ops {
                match op {
                    ArrayEditOp::Copy(run) => {
                        for _ in 0..*run {
                            let row = old.next().ok_or_else(|| {
                                ReplayCacheError::new(
                                    "array edit script copies past the end of its target",
                                )
                            })?;
                            if let Some(carried) = old_shadows.remove(&count_u32(cursor)?) {
                                new_shadows.insert(count_u32(out.len())?, carried);
                            }
                            out.push(row);
                            cursor += 1;
                        }
                    }
                    ArrayEditOp::Skip(run) => {
                        for _ in 0..*run {
                            old.next().ok_or_else(|| {
                                ReplayCacheError::new(
                                    "array edit script skips past the end of its target",
                                )
                            })?;
                            if let Some(mut dropped) =
                                old_shadows.remove(&count_u32(cursor)?)
                            {
                                dropped.release(budget);
                            }
                            cursor += 1;
                        }
                    }
                    ArrayEditOp::Patch(patch) => {
                        let mut row = old.next().ok_or_else(|| {
                            ReplayCacheError::new(
                                "array edit script patches past the end of its target",
                            )
                        })?;
                        let mut child = old_shadows
                            .remove(&count_u32(cursor)?)
                            .unwrap_or_default();
                        let applied = apply_patch(&mut row, patch, &mut child, budget, depth + 1);
                        if !child.is_empty() {
                            new_shadows.insert(count_u32(out.len())?, child);
                        }
                        applied?;
                        out.push(row);
                        cursor += 1;
                    }
                    ArrayEditOp::Insert(value) => {
                        // An inserted row has no history at the slot it lands
                        // in, so it starts with no memory under it.
                        out.push(value.clone());
                    }
                }
            }
            if old.next().is_some() {
                return Err(ReplayCacheError::new(
                    "array edit script leaves elements of its target unconsumed",
                ));
            }
            if out.is_empty() {
                return Err(ReplayCacheError::new(
                    "array edit script empties its target and must be a replacement",
                ));
            }
            // AN ARRAY THAT KEEPS ITS LENGTH IS `Patch::Array`'S JOB, and this
            // is where that is knowable: the encoder only reaches for a script
            // when the two lengths differ, so a script that lands on the same
            // length is a second spelling of an array patch (Codex round 1,
            // finding 5, second example).
            if out.len() == cursor {
                return Err(ReplayCacheError::new(
                    "array edit script keeps its target's length and must be an array patch",
                ));
            }
            // Whatever the script did not carry forward is gone with the rows
            // it belonged to.
            for (_, mut stranded) in old_shadows.drain() {
                stranded.release(budget);
            }
            shadow.elements = new_shadows;
            *target = CanonValue::Array(out);
            Ok(())
        }
        Patch::Array(changes) => {
            let CanonValue::Array(array) = target else {
                return Err(ReplayCacheError::new(
                    "array patch targets a non-array value",
                ));
            };
            let mut previous = None;
            for (slot, change) in changes {
                let index = *slot as usize;
                if previous.is_some_and(|value| index <= value) || index >= array.len() {
                    return Err(ReplayCacheError::new(
                        "array patch indices are not strictly increasing and in bounds",
                    ));
                }
                let mut child = shadow.elements.remove(slot).unwrap_or_default();
                let applied = apply_patch(&mut array[index], change, &mut child, budget, depth + 1);
                if !child.is_empty() {
                    shadow.elements.insert(*slot, child);
                }
                applied?;
                previous = Some(index);
            }
            Ok(())
        }
    }
}

/// Bytes `patch` encodes to, for choosing between two spellings of one change.
///
/// It BUILDS them rather than pricing them from a formula, because a formula
/// that drifted from [`encode_patch_into`] would pick the longer spelling and
/// nothing would say so.
fn encoded_patch_len(patch: &Patch, depth: usize) -> Result<usize, ReplayCacheError> {
    let mut out = Vec::new();
    encode_patch_into(patch, &mut out, depth)?;
    Ok(out.len())
}

fn encode_patch(patch: &Patch) -> Result<Vec<u8>, ReplayCacheError> {
    let mut out = Vec::new();
    encode_patch_into(patch, &mut out, 0)?;
    Ok(out)
}

fn encode_patch_into(
    patch: &Patch,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match patch {
        Patch::Keep => out.push(PATCH_KEEP),
        Patch::ReviveSame => out.push(PATCH_REVIVE_SAME),
        Patch::Revive(inner) => {
            out.push(PATCH_REVIVE_PATCH);
            encode_patch_into(inner, out, depth + 1)?;
        }
        Patch::Replace(value) => {
            out.push(PATCH_REPLACE);
            encode_value_into(value, out, depth + 1)?;
        }
        Patch::NumberDelta {
            fraction_digits,
            delta,
        } => {
            out.push(PATCH_NUMBER_DELTA);
            put_varint(out, u64::from(*fraction_digits));
            put_varint(out, zigzag(*delta));
        }
        Patch::Object { removals, changes } => {
            out.push(PATCH_OBJECT);
            put_varint(out, removals.len() as u64);
            for key in removals {
                put_text(out, key)?;
            }
            put_varint(out, changes.len() as u64);
            for (key, change) in changes {
                put_text(out, key)?;
                encode_patch_into(change, out, depth + 1)?;
            }
        }
        Patch::ArrayEdit(ops) => {
            out.push(PATCH_ARRAY_EDIT);
            put_varint(out, ops.len() as u64);
            for op in ops {
                match op {
                    ArrayEditOp::Copy(run) => {
                        put_varint(out, ARRAY_EDIT_COPY);
                        put_varint(out, u64::from(*run));
                    }
                    ArrayEditOp::Skip(run) => {
                        put_varint(out, ARRAY_EDIT_SKIP);
                        put_varint(out, u64::from(*run));
                    }
                    ArrayEditOp::Patch(patch) => {
                        put_varint(out, ARRAY_EDIT_PATCH);
                        encode_patch_into(patch, out, depth + 1)?;
                    }
                    ArrayEditOp::Insert(value) => {
                        put_varint(out, ARRAY_EDIT_INSERT);
                        encode_value_into(value, out, depth + 1)?;
                    }
                }
            }
        }
        Patch::Array(changes) => {
            out.push(PATCH_ARRAY);
            put_varint(out, changes.len() as u64);
            // Indices are strictly increasing, so the gap is what carries
            // information; a dense run of touched slots costs one byte each.
            let mut previous: Option<u32> = None;
            for (index, change) in changes {
                let gap = match previous {
                    None => u64::from(*index),
                    Some(previous) => u64::from(
                        index
                            .checked_sub(previous)
                            .and_then(|delta| delta.checked_sub(1))
                            .ok_or_else(|| {
                                ReplayCacheError::new(
                                    "array patch indices are not strictly increasing",
                                )
                            })?,
                    ),
                };
                put_varint(out, gap);
                encode_patch_into(change, out, depth + 1)?;
                previous = Some(*index);
            }
        }
    }
    Ok(())
}

/// A copy or skip run length: at least one element, and it must fit `u32` for
/// the same reason an array index does.
fn array_edit_run(reader: &mut ByteReader<'_>) -> Result<u32, ReplayCacheError> {
    let run = u32::try_from(reader.varint()?)
        .map_err(|_| ReplayCacheError::new("array edit run does not fit u32"))?;
    if run == 0 {
        return Err(ReplayCacheError::new(
            "array edit run of zero elements does nothing and must be omitted",
        ));
    }
    Ok(run)
}

fn decode_patch_exact(bytes: &[u8]) -> Result<Patch, ReplayCacheError> {
    let mut reader = ByteReader::new(bytes);
    let patch = decode_patch(&mut reader, 0)?;
    reader.finish()?;
    Ok(patch)
}

fn decode_patch(reader: &mut ByteReader<'_>, depth: usize) -> Result<Patch, ReplayCacheError> {
    require_depth(depth)?;
    match reader.u8()? {
        PATCH_KEEP if depth == 0 => Ok(Patch::Keep),
        // A nested keep is redundant with omitting the member or element, which
        // is what `diff_value` does. Accepting it would be a second encoding of
        // the same delta. Only a whole hour that changed nothing keeps.
        PATCH_KEEP => Err(ReplayCacheError::new(
            "nested keep patch must be omitted, not encoded",
        )),
        PATCH_REPLACE => Ok(Patch::Replace(decode_value(reader, depth + 1)?)),
        // THE WORLD ROOT IS ALWAYS AN OBJECT, so a number delta can only ever
        // be nested; one at depth 0 is a shape the encoder cannot produce.
        PATCH_NUMBER_DELTA if depth == 0 => Err(ReplayCacheError::new(
            "number delta patch at the world root, which is never a number",
        )),
        PATCH_NUMBER_DELTA => {
            let fraction_digits = reader.varint()?;
            if fraction_digits > u64::from(MAX_NUMBER_DELTA_DIGITS) {
                return Err(ReplayCacheError::new(
                    "number delta patch states more fraction digits than a number may carry",
                ));
            }
            Ok(Patch::NumberDelta {
                fraction_digits: fraction_digits as u32,
                delta: unzigzag(reader.varint()?),
            })
        }
        // THE WORLD ROOT IS NEVER NULL, so a revive can only ever be nested;
        // one at depth 0 is a shape the encoder cannot produce.
        PATCH_REVIVE_SAME | PATCH_REVIVE_PATCH if depth == 0 => Err(ReplayCacheError::new(
            "revive patch at the world root, which is never null",
        )),
        PATCH_REVIVE_SAME => Ok(Patch::ReviveSame),
        PATCH_REVIVE_PATCH => {
            let inner = decode_patch(reader, depth + 1)?;
            // Spelled out here as well as in `apply_patch` so a patch that
            // never reaches a target is still rejected: a keep is
            // `PATCH_REVIVE_SAME`, an unfitting base is a plain replace, and a
            // revivable scalar memory equals the new value and so is
            // `PATCH_REVIVE_SAME` too.
            if matches!(
                inner,
                Patch::Keep | Patch::Replace(_) | Patch::NumberDelta { .. }
            ) {
                return Err(ReplayCacheError::new(
                    "revive patch restates a delta with a shorter spelling",
                ));
            }
            Ok(Patch::Revive(Box::new(inner)))
        }
        PATCH_OBJECT => {
            let removal_count = reader.count(MIN_TEXT_BYTES)?;
            let mut removals = Vec::new();
            reserve_exact(&mut removals, removal_count)?;
            for _ in 0..removal_count {
                removals.push(reader.text()?);
            }
            if !strictly_increasing_keys(&removals) {
                return Err(ReplayCacheError::new(
                    "object patch removals are not strictly ordered",
                ));
            }
            let change_count = reader.count(MIN_KEYED_VALUE_BYTES)?;
            let mut changes = Vec::new();
            reserve_exact(&mut changes, change_count)?;
            for _ in 0..change_count {
                let key = reader.text()?;
                if !is_valid_object_key(&key) {
                    return Err(ReplayCacheError::new(
                        "object patch member name does not decode",
                    ));
                }
                changes.push((key, decode_patch(reader, depth + 1)?));
            }
            if !changes
                .windows(2)
                .all(|pair| key_cmp(&pair[0].0, &pair[1].0) == Ordering::Less)
            {
                return Err(ReplayCacheError::new(
                    "object patch changes are not strictly ordered",
                ));
            }
            // An object patch that touches nothing is a `Keep` spelled long,
            // and a name that is both removed and re-added is a `Replace`
            // spelled long. `diff_value` emits neither; accepting them would
            // give one delta series more than one set of cache bytes, and
            // therefore more than one cache SHA for the same content.
            if removals.is_empty() && changes.is_empty() {
                return Err(ReplayCacheError::new(
                    "object patch touches no member and must be encoded as a keep",
                ));
            }
            let mut removal_walk = 0usize;
            for (key, _) in &changes {
                while removal_walk < removals.len()
                    && key_cmp(&removals[removal_walk], key) == Ordering::Less
                {
                    removal_walk += 1;
                }
                if removals
                    .get(removal_walk)
                    .is_some_and(|removal| key_cmp(removal, key) == Ordering::Equal)
                {
                    return Err(ReplayCacheError::new(
                        "object patch removes and changes the same member name",
                    ));
                }
            }
            Ok(Patch::Object { removals, changes })
        }
        PATCH_ARRAY => {
            let count = reader.count(MIN_INDEXED_PATCH_BYTES)?;
            let mut changes: Vec<(u32, Patch)> = Vec::new();
            reserve_exact(&mut changes, count)?;
            let mut previous: Option<u32> = None;
            for _ in 0..count {
                let gap = u32::try_from(reader.varint()?).map_err(|_| {
                    ReplayCacheError::new("array patch index gap does not fit u32")
                })?;
                let index = match previous {
                    None => gap,
                    Some(previous) => previous
                        .checked_add(1)
                        .and_then(|next| next.checked_add(gap))
                        .ok_or_else(|| {
                            ReplayCacheError::new("array patch index overflows u32")
                        })?,
                };
                previous = Some(index);
                changes.push((index, decode_patch(reader, depth + 1)?));
            }
            if changes.is_empty() {
                return Err(ReplayCacheError::new(
                    "array patch touches no element and must be encoded as a keep",
                ));
            }
            Ok(Patch::Array(changes))
        }
        // THE WORLD ROOT IS ALWAYS AN OBJECT, so an array edit can only ever be
        // nested; one at depth 0 is a shape the encoder cannot produce -- the
        // same rule the number delta and the revive state here.
        PATCH_ARRAY_EDIT if depth == 0 => Err(ReplayCacheError::new(
            "array edit patch at the world root, which is never an array",
        )),
        PATCH_ARRAY_EDIT => {
            let count = reader.count(MIN_ARRAY_EDIT_OP_BYTES)?;
            let mut ops: Vec<ArrayEditOp> = Vec::new();
            reserve_exact(&mut ops, count)?;
            for _ in 0..count {
                let op = match reader.varint()? {
                    ARRAY_EDIT_COPY => ArrayEditOp::Copy(array_edit_run(reader)?),
                    ARRAY_EDIT_SKIP => ArrayEditOp::Skip(array_edit_run(reader)?),
                    // A nested keep is already refused by `decode_patch`, which
                    // is what stops a patch op standing in for a copy.
                    //
                    // AND A ROW PATCH IS ALWAYS AN OBJECT PATCH. Rows only
                    // align when EVERY row of both arrays is an object
                    // (`array_alignment_key`), and `diff_value` of two objects
                    // that are not equal is an object patch -- never a replace,
                    // a number delta or a revive. Without this rule an edit
                    // script could carry `PATCH(Replace(x)); SKIP 1` and spell
                    // a change the encoder writes as a plain `Replace` of the
                    // whole array (Codex round 2, finding 2).
                    ARRAY_EDIT_PATCH => {
                        let inner = decode_patch(reader, depth + 1)?;
                        if !matches!(inner, Patch::Object { .. }) {
                            return Err(ReplayCacheError::new(
                                "array edit row patch is not an object patch, which is the only \
                                 shape an aligned row can take",
                            ));
                        }
                        ArrayEditOp::Patch(inner)
                    }
                    ARRAY_EDIT_INSERT => ArrayEditOp::Insert(decode_value(reader, depth + 1)?),
                    kind => {
                        return Err(ReplayCacheError::new(format!(
                            "unknown array edit op {kind}"
                        )))
                    }
                };
                // Adjacent runs of one kind are ONE op with a bigger count, so
                // a split run is a second spelling of the same script.
                let merged = matches!(
                    (ops.last(), &op),
                    (Some(ArrayEditOp::Copy(_)), ArrayEditOp::Copy(_))
                        | (Some(ArrayEditOp::Skip(_)), ArrayEditOp::Skip(_))
                );
                if merged {
                    return Err(ReplayCacheError::new(
                        "array edit script splits a run that must be one op",
                    ));
                }
                ops.push(op);
            }
            if ops.is_empty() {
                return Err(ReplayCacheError::new(
                    "array edit script has no ops and must be encoded as a keep",
                ));
            }
            // An edit that only copies and patches is a `Patch::Array` spelled
            // long, and an array that keeps its length is `Patch::Array`'s job.
            if !ops
                .iter()
                .any(|op| matches!(op, ArrayEditOp::Skip(_) | ArrayEditOp::Insert(_)))
            {
                return Err(ReplayCacheError::new(
                    "array edit script neither skips nor inserts and must be an array patch",
                ));
            }
            // AND AN EDIT THAT KEEPS NO ROW OF ITS TARGET IS A REPLACEMENT,
            // which is always shorter: every row it inserts costs an op kind on
            // top of the row itself, and the skip that drops the old rows costs
            // two bytes more again. The encoder writes the shorter of the two,
            // so it can never mint this shape (Codex round 1, finding 5, first
            // two examples).
            if !ops
                .iter()
                .any(|op| matches!(op, ArrayEditOp::Copy(_) | ArrayEditOp::Patch(_)))
            {
                return Err(ReplayCacheError::new(
                    "array edit script keeps no row of its target and must be a replacement",
                ));
            }
            Ok(Patch::ArrayEdit(ops))
        }
        tag => Err(ReplayCacheError::new(format!(
            "unknown binary patch tag {tag}"
        ))),
    }
}

fn encode_value(value: &CanonValue) -> Result<Vec<u8>, ReplayCacheError> {
    let mut out = Vec::new();
    encode_value_into(value, &mut out, 0)?;
    Ok(out)
}

fn encode_value_into(
    value: &CanonValue,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match value {
        CanonValue::Null => out.push(TAG_NULL),
        CanonValue::Bool(false) => out.push(TAG_FALSE),
        CanonValue::Bool(true) => out.push(TAG_TRUE),
        CanonValue::Number(token) => match canonical_integer(token) {
            Some(CanonicalInteger::NonNegative(value)) if value <= INLINE_UINT_MAX => {
                out.push(INLINE_UINT_BASE + value as u8);
            }
            Some(CanonicalInteger::NonNegative(value)) => {
                out.push(TAG_UINT);
                put_varint(out, value);
            }
            Some(CanonicalInteger::Negative(magnitude)) => {
                out.push(TAG_NEGINT);
                put_varint(out, magnitude);
            }
            None => {
                out.push(TAG_NUMBER_TOKEN);
                put_text(out, token)?;
            }
        },
        CanonValue::String(body) => {
            out.push(TAG_STRING);
            put_text(out, body)?;
        }
        CanonValue::Array(values) => {
            out.push(TAG_ARRAY);
            put_varint(out, values.len() as u64);
            for value in values {
                encode_value_into(value, out, depth + 1)?;
            }
        }
        CanonValue::Object(members) => {
            out.push(TAG_OBJECT);
            put_varint(out, members.len() as u64);
            for (key, value) in members {
                put_text(out, key)?;
                encode_value_into(value, out, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn decode_value_exact(bytes: &[u8]) -> Result<CanonValue, ReplayCacheError> {
    let mut reader = ByteReader::new(bytes);
    let value = decode_value(&mut reader, 0)?;
    reader.finish()?;
    Ok(value)
}

fn decode_value(
    reader: &mut ByteReader<'_>,
    depth: usize,
) -> Result<CanonValue, ReplayCacheError> {
    require_depth(depth)?;
    let tag = reader.u8()?;
    if tag >= INLINE_UINT_BASE {
        return Ok(CanonValue::Number(
            u64::from(tag - INLINE_UINT_BASE).to_string(),
        ));
    }
    match tag {
        TAG_NULL => Ok(CanonValue::Null),
        TAG_FALSE => Ok(CanonValue::Bool(false)),
        TAG_TRUE => Ok(CanonValue::Bool(true)),
        TAG_NUMBER_TOKEN => {
            let token = reader.text()?;
            if !is_json_number_token(&token) {
                return Err(ReplayCacheError::new(format!(
                    "binary JSON number token {token:?} is not a JSON number"
                )));
            }
            if canonical_integer(&token).is_some() {
                return Err(ReplayCacheError::new(format!(
                    "binary JSON number {token:?} must use the compact integer encoding"
                )));
            }
            Ok(CanonValue::Number(token))
        }
        TAG_UINT => {
            let value = reader.varint()?;
            if value <= INLINE_UINT_MAX {
                return Err(ReplayCacheError::new(
                    "binary JSON integer must use the inline encoding",
                ));
            }
            Ok(CanonValue::Number(value.to_string()))
        }
        TAG_NEGINT => {
            let magnitude = reader.varint()?;
            if magnitude == 0 {
                return Err(ReplayCacheError::new(
                    "binary JSON negative integer must not spell zero",
                ));
            }
            Ok(CanonValue::Number(format!("-{magnitude}")))
        }
        TAG_STRING => {
            let body = reader.text()?;
            if !is_json_string_body(&body) {
                return Err(ReplayCacheError::new(
                    "binary JSON string body is not a JSON string body",
                ));
            }
            Ok(CanonValue::String(body))
        }
        TAG_ARRAY => {
            let count = reader.count(MIN_VALUE_BYTES)?;
            let mut values = Vec::new();
            reserve_exact(&mut values, count)?;
            for _ in 0..count {
                values.push(decode_value(reader, depth + 1)?);
            }
            Ok(CanonValue::Array(values))
        }
        TAG_OBJECT => {
            let count = reader.count(MIN_KEYED_VALUE_BYTES)?;
            let mut members: Vec<(String, CanonValue)> = Vec::new();
            reserve_exact(&mut members, count)?;
            for _ in 0..count {
                let key = reader.text()?;
                if !is_valid_object_key(&key) {
                    return Err(ReplayCacheError::new(
                        "binary JSON object member name does not decode",
                    ));
                }
                if members
                    .last()
                    .is_some_and(|(previous, _)| key_cmp(previous, &key) != Ordering::Less)
                {
                    return Err(ReplayCacheError::new(
                        "binary JSON object keys are not strictly ordered",
                    ));
                }
                let value = decode_value(reader, depth + 1)?;
                members.push((key, value));
            }
            Ok(CanonValue::Object(members))
        }
        tag => Err(ReplayCacheError::new(format!(
            "unknown binary JSON value tag {tag}"
        ))),
    }
}

enum CanonicalInteger {
    NonNegative(u64),
    Negative(u64),
}

/// The integer a token spells, when the token is exactly how [`u64`] /
/// `-`[`u64`] spells it. `1.0`, `1e3`, `-0`, `01` and anything past `u64` are
/// all `None` — they keep their token.
fn canonical_integer(token: &str) -> Option<CanonicalInteger> {
    let (negative, digits) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
        return None;
    }
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let magnitude: u64 = digits.parse().ok()?;
    if negative {
        // `-0` is not how anything spells zero here, and it would collide with
        // the non-negative encoding.
        (magnitude != 0).then_some(CanonicalInteger::Negative(magnitude))
    } else {
        Some(CanonicalInteger::NonNegative(magnitude))
    }
}

fn validate_world_identity(
    value: &CanonValue,
    hour_index: i64,
    date: &str,
) -> Result<(), ReplayCacheError> {
    if value.member("hour_index").and_then(CanonValue::as_i64) != Some(hour_index)
        || value
            .member("date")
            .and_then(CanonValue::as_decoded_str)
            .as_deref()
            != Some(date)
    {
        return Err(ReplayCacheError::new(format!(
            "world body identity disagrees with indexed hour {hour_index}"
        )));
    }
    Ok(())
}

fn require_depth(depth: usize) -> Result<(), ReplayCacheError> {
    if depth > MAX_RECURSION_DEPTH {
        Err(ReplayCacheError::new(
            "binary replay value exceeds the recursion bound",
        ))
    } else {
        Ok(())
    }
}

fn count_u32(value: usize) -> Result<u32, ReplayCacheError> {
    u32::try_from(value).map_err(|_| ReplayCacheError::new("binary collection count exceeds u32"))
}

/// Exact encoded byte floors for count-bearing collections.
const MIN_VALUE_BYTES: usize = 1;
const MIN_TEXT_BYTES: usize = 1;
const MIN_KEYED_VALUE_BYTES: usize = MIN_TEXT_BYTES + MIN_VALUE_BYTES;
const MIN_INDEXED_PATCH_BYTES: usize = 1 + MIN_VALUE_BYTES;
/// i64 + four u64s + two u32s + TWO digests (the reconstructed hour's and the
/// cache record's) + an empty u32-length-prefixed date.
const MIN_HOUR_ENTRY_BYTES: usize = 8 + 8 * 4 + 4 * 2 + 32 * 2 + 4;

/// Smallest encoded excluded-hour row: hour, two offsets, an empty length
/// prefix. Feeds the same count-vs-remaining bound every other counted
/// collection here goes through, so a forged count cannot buy an allocation.
const MIN_EXCLUDED_ENTRY_BYTES: usize = 8 + 8 + 8 + 4;

/// Reserve a count-fed collection fallibly so hostile input can never turn an
/// allocator refusal into a process abort.
fn reserve_exact<T>(vector: &mut Vec<T>, count: usize) -> Result<(), ReplayCacheError> {
    vector.try_reserve_exact(count).map_err(|_| {
        ReplayCacheError::new(format!(
            "binary replay artifact declares {count} elements of {} bytes, which this process cannot allocate",
            std::mem::size_of::<T>()
        ))
    })
}

fn bound_count_by_remaining(
    count: usize,
    min_bytes_per_element: usize,
    remaining: usize,
) -> Result<(), ReplayCacheError> {
    let floor = count.checked_mul(min_bytes_per_element).ok_or_else(|| {
        ReplayCacheError::new("binary collection count overflows its minimum encoded size")
    })?;
    if floor > remaining {
        return Err(ReplayCacheError::new(format!(
            "binary collection declares {count} elements needing at least {floor} bytes, but only {remaining} input bytes remain"
        )));
    }
    Ok(())
}

fn strictly_increasing_keys(values: &[String]) -> bool {
    values.iter().all(|key| is_valid_object_key(key))
        && values
            .windows(2)
            .all(|pair| key_cmp(&pair[0], &pair[1]) == Ordering::Less)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Minimal-length base-128 varint, little-endian groups, high bit = continue.
fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A varint-prefixed UTF-8 run inside a cache record.
fn put_text(out: &mut Vec<u8>, value: &str) -> Result<(), ReplayCacheError> {
    if value.len() > MAX_STRING_BYTES {
        return Err(ReplayCacheError::new("binary string exceeds the size bound"));
    }
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

/// A 32-bit-prefixed UTF-8 run inside the HOUR INDEX, whose per-hour rows are
/// fixed-width by design so a reader can size them without scanning.
fn put_index_string(out: &mut Vec<u8>, value: &str) -> Result<(), ReplayCacheError> {
    if value.len() > MAX_STRING_BYTES {
        return Err(ReplayCacheError::new("binary string exceeds the size bound"));
    }
    put_u32(out, count_u32(value.len())?);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn decode_hex32(value: &str) -> Result<[u8; 32], ReplayCacheError> {
    if value.len() != 64 {
        return Err(ReplayCacheError::new(
            "SHA-256 identity is not 64 lowercase hex characters",
        ));
    }
    let mut out = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_nibble(value: u8) -> Result<u8, ReplayCacheError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(ReplayCacheError::new(
            "SHA-256 identity is not lowercase hexadecimal",
        )),
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ReplayCacheError> {
        let end = self
            .position
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| ReplayCacheError::new("binary replay artifact is truncated"))?;
        let bytes = &self.bytes[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, ReplayCacheError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ReplayCacheError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, ReplayCacheError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ReplayCacheError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, ReplayCacheError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn array32(&mut self) -> Result<&'a [u8; 32], ReplayCacheError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// A varint, rejecting any non-minimal spelling so one number has one
    /// encoding and the cache SHA stays a function of the content.
    fn varint(&mut self) -> Result<u64, ReplayCacheError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            let payload = u64::from(byte & 0x7f);
            if shift >= 64 || (shift == 63 && payload > 1) {
                return Err(ReplayCacheError::new("binary varint overflows u64"));
            }
            value |= payload << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift != 0 {
                    return Err(ReplayCacheError::new("binary varint is not minimal"));
                }
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn count(&mut self, min_bytes_per_element: usize) -> Result<usize, ReplayCacheError> {
        let value = self.varint()?;
        let count = usize::try_from(value)
            .map_err(|_| ReplayCacheError::new("binary collection count does not fit usize"))?;
        bound_count_by_remaining(count, min_bytes_per_element, self.remaining())?;
        Ok(count)
    }

    fn text(&mut self) -> Result<String, ReplayCacheError> {
        let len = usize::try_from(self.varint()?)
            .map_err(|_| ReplayCacheError::new("binary string length does not fit usize"))?;
        if len > MAX_STRING_BYTES {
            return Err(ReplayCacheError::new("binary string exceeds the size bound"));
        }
        let raw = self.take(len)?;
        let mut bytes = Vec::new();
        reserve_exact(&mut bytes, raw.len())?;
        bytes.extend_from_slice(raw);
        String::from_utf8(bytes)
            .map_err(|_| ReplayCacheError::new("binary replay string is not UTF-8"))
    }

    fn index_string(&mut self) -> Result<String, ReplayCacheError> {
        self.index_string_bounded(MAX_STRING_BYTES)
    }

    /// Decode a length-prefixed string, refusing an over-long one BEFORE the
    /// copy rather than after.
    ///
    /// The order matters. A field with a tight semantic ceiling — an exclusion
    /// reason is prose naming one hour — inherits nothing from a validation
    /// pass that runs once every row is already materialized: an authenticated
    /// H4IX could carry rows whose declared reasons sum to hundreds of
    /// megabytes, and the reader would duplicate all of them before producing
    /// the typed refusal the bound exists to produce (Codex review round 1,
    /// finding 3).
    fn index_string_bounded(&mut self, max_bytes: usize) -> Result<String, ReplayCacheError> {
        let len = self.u32()? as usize;
        if len > max_bytes {
            return Err(ReplayCacheError::new(format!(
                "binary string declares {len} bytes, above the {max_bytes}-byte bound"
            )));
        }
        String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| ReplayCacheError::new("binary replay string is not UTF-8"))
    }

    fn finish(&self) -> Result<(), ReplayCacheError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(ReplayCacheError::new(
                "binary replay artifact has trailing bytes",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayCacheError {
    message: String,
}

impl ReplayCacheError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ReplayCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReplayCacheError {}

/// Lane CE size-lever measurement instrument. It walks the v2 record grammar
/// from inside this module — the grammar's own private constants and reader —
/// so the decomposition it publishes cannot drift from the encoder it is
/// measuring. Never on by default and never packaged.
#[cfg(feature = "ce-cache-anatomy")]
#[path = "ce_anatomy.rs"]
pub mod ce_anatomy;

/// Lane DQ H4CP v3 prototype instrument: price ONE section's v2 delta with the
/// production encoder.
///
/// The prototype's central byte question is whether a mutation journal emits
/// more or fewer bytes per hour than the structural diff it would replace, per
/// section. Answering it with a reimplemented diff would answer a different
/// question, so the instrument reaches the real [`diff_value`] and
/// [`encode_patch`] -- the very pair `encode_prepared` runs -- and applies them
/// to a subtree instead of to a whole document. Never on by default and never
/// packaged; it writes nothing and no production path calls it.
#[cfg(feature = "dq-v3-prototype")]
pub fn measure_section_patch_bytes(
    old: &CanonValue,
    new: &CanonValue,
) -> Result<usize, ReplayCacheError> {
    let patch = diff_value(old, new, None, 0)?;
    Ok(encode_patch(&patch)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon_value::NON_IDENTITY_TOKENS;
    use serde_json::Value;

    /// The recorded per-hour digest is of the SOURCE body, and it stayed that
    /// way when the body stopped being retained.
    ///
    /// `CanonicalWorldHour` used to hold `world_json` all the way through the
    /// delta encode so the digest could be taken at the end; now the digest is
    /// taken at parse and the body is released. The number a reader follows
    /// must not have moved. This fails if the digest is ever re-derived from
    /// the parsed value (re-serializing a `CanonValue` is NOT guaranteed to
    /// reproduce a body byte for byte) or taken over a different input.
    #[test]
    fn the_recorded_hour_digest_is_of_the_source_body_not_of_the_parse() {
        let body = world_json(0, r#"{"n":189352.52932739258}"#);
        let expected = Sha256::hex(&body);
        let prepared = CanonicalWorldHour::parse(body.clone(), 0).unwrap();
        assert_eq!(prepared.world_json_sha256(), expected);

        // And it survives the encode: the index row carries the same value.
        let (_, index, _) = build(1, &[(0, r#"{"n":189352.52932739258}"#)]);
        assert_eq!(index.hours()[0].world_json_sha256(), expected);
    }

    /// `push` (which parses internally) and `push_prepared` (which is handed an
    /// already-parsed hour) must remain byte-identical producers. The seam
    /// exists only to avoid a second parse; if the digest change had made them
    /// diverge, a recorder and a re-encode would disagree on the same hours.
    #[test]
    fn push_and_push_prepared_produce_identical_cache_bytes() {
        let source_sha = "cd".repeat(32);
        let hours = [(0i64, r#"{"a":1}"#), (1, r#"{"a":2}"#)];

        let mut via_push = ReplayCacheEncoder::new(Vec::new(), &source_sha, 8).unwrap();
        for (hour, body) in hours {
            via_push.push(world(hour, body)).unwrap();
        }
        let (push_bytes, push_index) = via_push
            .finish("g", "1", CacheIdentityMode::Legacy, 200)
            .unwrap();

        let mut via_prepared = ReplayCacheEncoder::new(Vec::new(), &source_sha, 8).unwrap();
        for (hour, body) in hours {
            let built = world(hour, body);
            let meta = CacheBuildHourMeta {
                hour_index: built.hour_index,
                date: built.date.clone(),
                h4cr_start_offset: built.h4cr_start_offset,
                h4cr_end_offset: built.h4cr_end_offset,
            };
            let prepared = CanonicalWorldHour::parse(built.world_json, built.hour_index).unwrap();
            via_prepared.push_prepared(meta, prepared).unwrap();
        }
        let (prepared_bytes, prepared_index) = via_prepared
            .finish("g", "1", CacheIdentityMode::Legacy, 200)
            .unwrap();

        assert_eq!(push_bytes, prepared_bytes);
        assert_eq!(
            push_index.to_bytes().unwrap(),
            prepared_index.to_bytes().unwrap()
        );
    }

    /// The encoder's own accounting must track what it retains. A delta base
    /// that is present must price above zero, and the per-hour index rows must
    /// grow with the hours — this is the number the recorder's component table
    /// publishes, so a stub returning a constant is a defect.
    #[test]
    fn encoder_prices_its_retained_delta_base_and_hour_rows() {
        let source_sha = "ef".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 8).unwrap();
        assert_eq!(encoder.delta_base_heap_bytes(), 0);
        assert_eq!(encoder.hour_entry_count(), 0);

        encoder.push(world(0, r#"{"a":1}"#)).unwrap();
        let one_hour_rows = encoder.hour_entries_heap_bytes();
        let base = encoder.delta_base_heap_bytes();
        assert!(base > 0, "a retained delta base must not price as zero");
        assert!(one_hour_rows > 0);

        encoder.push(world(1, r#"{"a":2}"#)).unwrap();
        assert_eq!(encoder.hour_entry_count(), 2);
        assert!(
            encoder.hour_entries_heap_bytes() > one_hour_rows,
            "hour rows are a session-length scaler and must be seen to grow"
        );
    }

    fn world_json(hour: i64, body: &str) -> Vec<u8> {
        // Written by hand rather than through `serde_json`, so a fixture can
        // carry a spelling `serde_json` would never emit.
        format!(
            r#"{{"date":"1936.1.1.{}","game_id":"g","hour_index":{hour},"schema_version":"1","world":{body}}}"#,
            hour + 1
        )
        .into_bytes()
    }

    fn world(hour: i64, body: &str) -> CacheBuildHour {
        CacheBuildHour {
            hour_index: hour,
            date: format!("1936.1.1.{}", hour + 1),
            h4cr_start_offset: (hour as u64) * 100,
            h4cr_end_offset: (hour as u64 + 1) * 100,
            world_json: world_json(hour, body),
        }
    }

    fn build(interval: u32, hours: &[(i64, &str)]) -> (Vec<u8>, ReplayHourIndex, String) {
        build_inner(interval, hours, false)
    }

    /// `build`, spelled for the tests that mean to exercise the revive.
    ///
    /// `build` itself FORBIDS it, so it stays a control: the pairs below assert
    /// that the same hours cost less with revives than without, which needs one
    /// of each.
    fn build_revive(interval: u32, hours: &[(i64, &str)]) -> (Vec<u8>, ReplayHourIndex, String) {
        build_inner(interval, hours, true)
    }

    fn build_inner(
        interval: u32,
        hours: &[(i64, &str)],
        revive: bool,
    ) -> (Vec<u8>, ReplayHourIndex, String) {
        let source_sha = "ab".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, interval).unwrap();
        if !revive {
            encoder.forbid_revive();
        }
        for (hour, body) in hours {
            encoder.push(world(*hour, body)).unwrap();
        }
        let source_len = (hours.last().unwrap().0 as u64 + 1) * 100;
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::IdentityV1, source_len)
            .unwrap();
        (cache, index, source_sha)
    }

    /// THE REVIVE IS WHAT THE SHIPPED ENCODER WRITES, and the gate that held it
    /// back for one round still works as a control.
    ///
    /// kills: flipping the default back off; making `forbid_revive` a no-op
    /// (the control then costs the same as the default and the first assertion
    /// fails).
    #[test]
    fn the_shipped_encoder_writes_the_revive_and_the_control_does_not() {
        let big = r#"{"a":[1,2,3,4,5,6,7,8],"b":"a long enough value to notice"}"#;
        let hours = [
            (0, format!(r#"{{"dom":{big}}}"#)),
            (1, r#"{"dom":null}"#.to_string()),
            (2, format!(r#"{{"dom":{big}}}"#)),
        ];
        let hours: Vec<(i64, &str)> = hours.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (plain, plain_index, sha) = build(168, &hours);
        let (revived, revived_index, _) = build_revive(168, &hours);

        // The gate is real: the same hours cost strictly less with it open.
        let payload = |index: &ReplayHourIndex, position: usize| {
            index.hours()[position].cache_record_len() - RECORD_HEADER_LEN
        };
        assert!(
            payload(&revived_index, 2) < payload(&plain_index, 2),
            "with revives allowed hour 2 must be cheaper: {} against {}",
            payload(&revived_index, 2),
            payload(&plain_index, 2)
        );

        // And the default really re-emits, which is the behaviour a revive
        // replaces: its cost TRACKS THE BODY, while the opt-in encoder's does
        // not. Scanning the bytes for a tag value would prove nothing -- 4 and
        // 5 occur all over a payload as varints and value tags.
        let wide = format!(
            r#"{{"a":[{}],"b":"a long enough value to notice"}}"#,
            (0..400).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
        );
        let wide_hours = [
            (0, format!(r#"{{"dom":{wide}}}"#)),
            (1, r#"{"dom":null}"#.to_string()),
            (2, format!(r#"{{"dom":{wide}}}"#)),
        ];
        let wide_hours: Vec<(i64, &str)> =
            wide_hours.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (_, wide_plain, _) = build(168, &wide_hours);
        let (_, wide_revived, _) = build_revive(168, &wide_hours);
        assert!(
            payload(&wide_plain, 2) > payload(&plain_index, 2) * 4,
            "the default encoder re-emits, so a wider body costs more"
        );
        assert_eq!(
            payload(&wide_revived, 2),
            payload(&revived_index, 2),
            "the opt-in encoder revives, so a wider body costs the same"
        );
        let _ = (&plain, &revived);

        let reader =
            ReplayCacheReader::open(&plain_index.to_bytes().unwrap(), &plain, 300, &sha).unwrap();
        reader.verify_every_hour().unwrap();
    }

    /// THE CADENCE FIX, END TO END: a domain that folds out and comes back
    /// costs ONE BYTE, not a re-emission of itself.
    ///
    /// This is the shape that dominates the runs which bust the owner's 5 GB
    /// bar. Hour 1 nulls a domain -- the ordinary cadence, not damage -- and
    /// hour 2 brings back exactly what was there. The old encoder had only
    /// `null` to diff against, and `null` is structurally incompatible with an
    /// object, so it re-emitted the whole subtree; measured over 32 consecutive
    /// t53 hours that re-emission was 44% of every delta byte written.
    ///
    /// kills: dropping `Patch::ReviveSame` (hour 2 re-emits and the assertion
    /// on its size fails); basing the revive on the checkpoint instead of on
    /// the member's own last non-null value (hour 4 here revives to the WRONG
    /// body and `verify_every_hour` fails); forgetting to carry the shadow on
    /// the encoder side (no revive is emitted at all); carrying it on the
    /// encoder but not the reader (the reader cannot resolve the revive).
    #[test]
    fn a_domain_that_folds_out_and_returns_costs_one_byte() {
        let big = r#"{"a":[1,2,3,4,5,6,7,8],"b":"a long enough value to notice","c":{"d":true}}"#;
        let (cache, index, sha) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"dom":{big}}}"#)),
                (1, r#"{"dom":null}"#),
                (2, &format!(r#"{{"dom":{big}}}"#)),
                (3, r#"{"dom":null}"#),
                (4, &format!(r#"{{"dom":{big},"e":1}}"#)),
            ],
        );

        // THE PROPERTY IS THAT THE COST DOES NOT TRACK THE BODY. A byte count
        // would only pin this fixture; running the SAME sequence with a body an
        // order of magnitude larger and requiring the revived hour not to grow
        // by one byte pins the thing that matters, and it is exactly what a
        // re-emission could never satisfy.
        let payload = |index: &ReplayHourIndex, position: usize| {
            index.hours()[position].cache_record_len() - RECORD_HEADER_LEN
        };
        let wide = format!(
            r#"{{"a":[{}],"b":"a long enough value to notice","c":{{"d":true}}}}"#,
            (0..400).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
        );
        let (_, wider, _) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"dom":{wide}}}"#)),
                (1, r#"{"dom":null}"#),
                (2, &format!(r#"{{"dom":{wide}}}"#)),
                (3, r#"{"dom":null}"#),
                (4, &format!(r#"{{"dom":{wide},"e":1}}"#)),
            ],
        );
        assert!(
            payload(&wider, 0) > payload(&index, 0) * 4,
            "the wider fixture must really carry a much larger body"
        );
        assert_eq!(
            payload(&wider, 2),
            payload(&index, 2),
            "a bare revive must cost the same however large the body it brings back"
        );
        assert!(
            payload(&index, 2) * 2 < payload(&index, 0),
            "a bare revive must not re-emit the body: {} against the body's {}",
            payload(&index, 2),
            payload(&index, 0)
        );
        assert_eq!(
            payload(&wider, 4),
            payload(&index, 4),
            "a revive with a patch on top must also not track the body it revives"
        );

        // And every hour still reconstructs to exactly what it was.
        let reader = ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 500, &sha).unwrap();
        reader.verify_every_hour().unwrap();
    }

    /// A REVIVE MAY NOT REACH ACROSS A CHECKPOINT, because a seek that starts
    /// at one has never seen what came before it.
    ///
    /// Interval 2 puts a checkpoint at hour 2, so the body nulled at hour 1 is
    /// out of reach by hour 3 and that hour must pay for it in full.
    ///
    /// kills: keeping the shadow across `self.shadow = Shadow::default()` at a
    /// checkpoint -- the encoder would emit a revive that `reconstruct_json`,
    /// which starts from the checkpoint, could not resolve.
    #[test]
    fn a_revive_never_reaches_across_a_checkpoint() {
        let big = r#"{"a":[1,2,3,4,5,6,7,8],"b":"a long enough value to notice"}"#;
        let (cache, index, sha) = build_revive(
            2,
            &[
                (0, &format!(r#"{{"dom":{big}}}"#)),
                (1, r#"{"dom":null}"#),
                (2, r#"{"dom":null}"#),
                (3, &format!(r#"{{"dom":{big}}}"#)),
            ],
        );
        let payload = |position: usize| index.hours()[position].cache_record_len() - RECORD_HEADER_LEN;
        assert!(
            payload(3) > 40,
            "hour 3 is in a new window and must re-emit the body, spent only {}",
            payload(3)
        );

        // Every single-hour seek must resolve, which is the property a
        // cross-checkpoint revive would break.
        let reader = ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 400, &sha).unwrap();
        reader.verify_every_hour().unwrap();
        for hour in 0..4 {
            reader.reconstruct_json(hour).unwrap();
        }
    }

    /// A REVIVE THAT CANNOT BE RESOLVED IS REFUSED, never guessed at.
    ///
    /// A revive is the one patch whose meaning depends on state the record does
    /// not carry, so every shape where that state is missing or where the
    /// spelling is not the one the encoder produces has to fail closed --
    /// otherwise a damaged or hand-built record could decode to a document the
    /// recording never held.
    ///
    /// kills: accepting `PATCH_REVIVE_PATCH` wrapping a keep or a replace, both
    /// of which have shorter spellings the encoder would have used; accepting a
    /// revive at the world root, which is never null; accepting a revive with
    /// nothing remembered; accepting one over a live value.
    #[test]
    fn every_unresolvable_revive_is_refused() {
        // A revive wrapping a keep is `PATCH_REVIVE_SAME`; one wrapping a
        // replace is a plain replace. Both are refused at decode.
        for inner in [vec![PATCH_KEEP], vec![PATCH_REPLACE, TAG_NULL]] {
            let mut bytes = vec![PATCH_OBJECT, 0, 1, 1, b'x', PATCH_REVIVE_PATCH];
            bytes.extend_from_slice(&inner);
            let error = decode_patch_exact(&bytes)
                .expect_err("a revive with a shorter spelling must be refused");
            // A wrapped keep trips the nested-keep rule one level down before
            // it reaches the revive check; both refusals are the same law.
            let message = error.to_string();
            assert!(
                message.contains("shorter spelling") || message.contains("nested keep"),
                "{message}"
            );
        }

        // The world root is never null.
        for tag in [PATCH_REVIVE_SAME, PATCH_REVIVE_PATCH] {
            let error = decode_patch_exact(&[tag])
                .expect_err("a revive at the root must be refused");
            assert!(error.to_string().contains("world root"), "{error}");
        }

        // And a revive with nothing remembered has no base to name.
        let mut target = CanonValue::Null;
        let error = apply_patch(&mut target, &Patch::ReviveSame, &mut Shadow::default(), &mut ShadowBudget::default(), 1)
            .expect_err("a revive without a memory must be refused");
        assert!(error.to_string().contains("no remembered value"), "{error}");

        // A revive over a value that is not null is not a revive at all.
        let mut shadow = Shadow::default();
        shadow.remember(CanonValue::parse(br#"{"a":1}"#).unwrap(), &mut ShadowBudget::default());
        let mut target = CanonValue::parse(br#"{"b":1}"#).unwrap();
        let error = apply_patch(&mut target, &Patch::ReviveSame, &mut shadow, &mut ShadowBudget::default(), 1)
            .expect_err("a revive over a live value must be refused");
        assert!(error.to_string().contains("not null"), "{error}");
    }

    /// A REVIVED SUBTREE STILL REMEMBERS ITS OWN FOLDED-OUT PARTS.
    ///
    /// The parent comes back at hour 2 with a member already nulled inside it,
    /// and that member returns at hour 3. The inner null is a memory like any
    /// other, so hour 3 must be a bare revive -- but the first cut of this
    /// handed the inner patch a throwaway shadow and dropped everything it
    /// recorded, so hour 3 re-emitted instead (Codex review round 6, S1).
    ///
    /// kills: passing `&mut Shadow::default()` to the inner application instead
    /// of the node whose memory was just taken.
    #[test]
    fn a_revived_subtree_keeps_the_memories_its_own_patch_makes() {
        let big = r#"{"a":[1,2,3,4,5,6,7,8],"b":"a long enough value to notice"}"#;
        let (cache, index, sha) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"p":{{"x":{big}}}}}"#)),
                (1, r#"{"p":null}"#),
                (2, r#"{"p":{"x":null}}"#),
                (3, &format!(r#"{{"p":{{"x":{big}}}}}"#)),
            ],
        );
        let payload = |index: &ReplayHourIndex, position: usize| {
            index.hours()[position].cache_record_len() - RECORD_HEADER_LEN
        };
        // Same test with a much wider body: if hour 3 re-emitted, its cost
        // would track the body.
        let wide = format!(
            r#"{{"a":[{}],"b":"a long enough value to notice"}}"#,
            (0..400).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
        );
        let (_, wider, _) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"p":{{"x":{wide}}}}}"#)),
                (1, r#"{"p":null}"#),
                (2, r#"{"p":{"x":null}}"#),
                (3, &format!(r#"{{"p":{{"x":{wide}}}}}"#)),
            ],
        );
        assert_eq!(
            payload(&wider, 3),
            payload(&index, 3),
            "the member nulled INSIDE a revive must itself be revivable"
        );
        let reader = ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 400, &sha).unwrap();
        reader.verify_every_hour().unwrap();
    }

    /// A SCALAR THAT COMES BACK UNCHANGED IS ONE BYTE.
    ///
    /// The first cut let `revive_base_fits` decide whether a revive was
    /// available at all, and it admits only objects and equal-length arrays. So
    /// for `7 -> null -> 7` the encoder emitted `Replace(7)` while the decoder
    /// would ALSO have accepted `ReviveSame` -- one delta with two encodings,
    /// therefore two possible cache SHAs for one recording (Codex review round
    /// 6, S2).
    ///
    /// kills: restoring `revive_base_fits` as the availability test in either
    /// `diff_value` or the replace refusal; making the two disagree.
    #[test]
    fn a_returning_scalar_is_spelled_as_a_revive() {
        let (cache, index, sha) = build_revive(
            168,
            &[
                (0, r#"{"n":123456789,"s":"a string worth several bytes"}"#),
                (1, r#"{"n":null,"s":null}"#),
                (2, r#"{"n":123456789,"s":"a string worth several bytes"}"#),
            ],
        );
        // Two revives cost two bytes WHATEVER the scalars are worth, which a
        // re-emission could not manage.
        let payload = |index: &ReplayHourIndex, position: usize| {
            index.hours()[position].cache_record_len() - RECORD_HEADER_LEN
        };
        let long = "a string worth a great many more bytes than the other one,                     padded until it could not hide inside a rounding error";
        let (_, wider, _) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"n":123456789123456789,"s":"{long}"}}"#)),
                (1, r#"{"n":null,"s":null}"#),
                (2, &format!(r#"{{"n":123456789123456789,"s":"{long}"}}"#)),
            ],
        );
        assert!(
            payload(&wider, 0) > payload(&index, 0) + 60,
            "the wider fixture must really carry larger scalars"
        );
        assert_eq!(
            payload(&wider, 2),
            payload(&index, 2),
            "returning scalars must revive, not re-emit"
        );

        // The long spelling still DECODES -- deliberately, see the note in
        // `apply_patch` -- and it decodes to the same value, which is why
        // tolerating it costs a byte rather than a fact.
        let mut shadow = Shadow::default();
        shadow.remember(
            CanonValue::parse(b"123456789").unwrap(),
            &mut ShadowBudget::default(),
        );
        let mut target = CanonValue::Null;
        apply_patch(
            &mut target,
            &Patch::Replace(CanonValue::parse(b"123456789").unwrap()),
            &mut shadow,
            &mut ShadowBudget::default(),
            1,
        )
        .expect("a replace carries its own value and always applies");
        assert_eq!(target, CanonValue::parse(b"123456789").unwrap());

        let reader = ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 300, &sha).unwrap();
        reader.verify_every_hour().unwrap();
    }

    /// THE SHADOW HAS A CEILING, and reaching it costs bytes rather than truth.
    ///
    /// A window is up to 744 hours, so a recording that nulls a different large
    /// member every hour would otherwise accumulate a remembered subtree per
    /// hour -- memory that grows with the WINDOW, not with the world (Codex
    /// review round 6, S4). Past the cap the encoder simply stops offering
    /// revives; both sides apply the same rule over the same records, so they
    /// stop together and no delta becomes unresolvable.
    ///
    /// kills: removing the budget check from `Shadow::remember`; charging the
    /// encoder but not the reader.
    ///
    /// The ceiling is a BALANCE and refunds are part of it (see
    /// [`ShadowBudget`]), so "refunding on drop" is no longer in this test's
    /// kill list -- it is the behaviour, and
    /// [`a_spent_memory_is_refunded_to_the_window`] is what holds it. The
    /// refund cannot make the two sides disagree: nothing is admitted during a
    /// release, the amount given back is the charge recorded beside the memory
    /// rather than a re-measurement, and both sides drive every charge and
    /// every refund from [`apply_patch`] over the same records in the same
    /// order -- so the balance after a step is the same number whatever order a
    /// hash map happened to drain in.
    #[test]
    fn the_shadow_stops_remembering_at_its_ceiling() {
        let mut budget = ShadowBudget {
            spent: 0,
            cap: 64,
        };
        let small = CanonValue::parse(br#"{"a":1}"#).unwrap();
        let large = CanonValue::parse(br#"{"a":"a value far past a sixty-four byte ceiling for sure"}"#).unwrap();

        let mut kept = Shadow::default();
        kept.remember(small.clone(), &mut budget);
        assert!(
            kept.remembered.is_some(),
            "a memory inside the ceiling is kept"
        );

        let mut shadow = Shadow::default();
        shadow.remember(large, &mut budget);
        assert!(
            shadow.remembered.is_none(),
            "a memory past the ceiling is declined, not truncated"
        );
        // Declining must not charge for what it did not keep.
        assert!(budget.spent <= 64, "a declined memory is not charged");

        // The ceiling is a real number, not a placeholder.
        assert_eq!(MAX_SHADOW_BYTES, 256 * 1024 * 1024);
    }

    /// A MEMORY THE WINDOW NO LONGER HOLDS IS NOT STILL PAID FOR.
    ///
    /// This is the whole of lane LZ's first size lever. The budget used to be a
    /// meter: every fold-out charged and nothing ever gave the charge back, so
    /// a domain that folds out every fourth hour spent its own size every
    /// fourth hour against a 256 MiB cap that never moved. On capture t53 that
    /// exhausted the cap at position 61 of a 168-hour window and declined 1,326
    /// revives after it -- each one a whole subtree re-emitted in full.
    ///
    /// The four ways a memory stops existing are all exercised here, because a
    /// refund missing from any one of them leaks the balance in exactly the way
    /// that emptied the budget.
    ///
    /// kills: dropping the refund in `take_remembered` (the revive arm), the
    /// `release` in `Shadow::remember`, the `release` on a replace that lands
    /// non-null, or the `release` on an object-patch removal -- each leaves the
    /// balance holding a memory that is gone, and the assertion that the window
    /// can remember again is what fails.
    #[test]
    fn a_spent_memory_is_refunded_to_the_window() {
        let body = CanonValue::parse(br#"{"a":"sixty-four bytes of body is more than this cap allows"}"#)
            .unwrap();
        let charge = body.content_bytes();
        let cap = charge + 1;

        // (1) SPENT ON A REVIVE. The memory is taken, so the window gets it
        // back and can hold the next one.
        let mut budget = ShadowBudget { spent: 0, cap };
        let mut shadow = Shadow::default();
        shadow.remember(body.clone(), &mut budget);
        assert_eq!(budget.spent, charge);
        let mut target = CanonValue::Null;
        apply_patch(&mut target, &Patch::ReviveSame, &mut shadow, &mut budget, 1)
            .expect("a remembered value revives");
        assert_eq!(budget.spent, 0, "a revive spends the memory and the charge");

        // (2) OVERWRITTEN BY THE NEXT MEMORY AT THE SAME PATH.
        let mut budget = ShadowBudget { spent: 0, cap };
        let mut shadow = Shadow::default();
        shadow.remember(body.clone(), &mut budget);
        shadow.remember(body.clone(), &mut budget);
        assert_eq!(
            budget.spent, charge,
            "one path holds one memory and is charged for one"
        );
        assert!(
            shadow.remembered.is_some(),
            "the second memory fits because the first was given back"
        );

        // (3) CLEARED BECAUSE THE PATH IS NON-NULL AGAIN.
        let mut budget = ShadowBudget { spent: 0, cap };
        let mut shadow = Shadow::default();
        shadow.remember(body.clone(), &mut budget);
        let mut target = CanonValue::Null;
        apply_patch(
            &mut target,
            &Patch::Replace(CanonValue::parse(b"7").unwrap()),
            &mut shadow,
            &mut budget,
            1,
        )
        .expect("a replace over a null applies");
        assert_eq!(budget.spent, 0, "a path that is back is not still remembered");

        // (4) REMOVED WITH ITS MEMBER. The child shadow goes, and its charge
        // with it.
        let mut budget = ShadowBudget { spent: 0, cap };
        let mut shadow = Shadow::default();
        let mut child = Shadow::default();
        child.remember(body.clone(), &mut budget);
        shadow.members.insert("gone".into(), child);
        let mut target = CanonValue::parse(br#"{"gone":null}"#).unwrap();
        apply_patch(
            &mut target,
            &Patch::Object {
                removals: vec!["gone".into()],
                changes: Vec::new(),
            },
            &mut shadow,
            &mut budget,
            0,
        )
        .expect("an object patch removes a member");
        assert_eq!(
            budget.spent, 0,
            "a removed member's memory is not still charged"
        );
    }

    /// AN ARRAY THAT GAINED A ROW IS NOT RESTATED, and what it costs tracks
    /// what CHANGED rather than how long the array is.
    ///
    /// This is the whole of [`PATCH_ARRAY_EDIT`]. The old encoder could only
    /// address array elements by index, so any length change fell to a
    /// `Replace` carrying every row in full; on capture t53 that was 10,037
    /// sealed bytes per game hour on `divisions` alone.
    ///
    /// The second half is what stops a cheaper-looking claim from being
    /// vacuous: the wide array's delta must NOT grow with the array, which is
    /// the property a replace fails and an edit script keeps.
    ///
    /// kills: dropping the `ArrayEdit` arm from `diff_value` (both assertions
    /// fail); aligning on position instead of on the rows' own key (every row
    /// after the inserted one becomes a patch, and the wide delta grows with
    /// the array); reversing the shorter-of-the-two choice.
    #[test]
    fn an_array_that_gains_a_row_is_edited_not_restated() {
        let rows = |count: usize| {
            (0..count)
                .map(|id| format!(r#"{{"id":{id},"v":"row body {id} with some width to it"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        };
        let narrow = [
            (0, format!(r#"{{"rows":[{}]}}"#, rows(8))),
            (1, format!(r#"{{"rows":[{},{}]}}"#, rows(8),
                        r#"{"id":999,"v":"row body 999 with some width to it"}"#)),
        ];
        let narrow: Vec<(i64, &str)> = narrow.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (cache, index, sha) = build_revive(168, &narrow);
        let delta = index.hours()[1].cache_record_len() - RECORD_HEADER_LEN;

        // One row was added to an array of eight. A replace would carry all
        // nine; the row itself is about 45 bytes.
        assert!(
            delta < 100,
            "an added row must cost about the row, not the array: {delta} bytes"
        );
        ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 200, &sha)
            .unwrap()
            .verify_every_hour()
            .unwrap();

        // THE COST DOES NOT TRACK THE ARRAY. Four hundred rows, same single
        // insertion, and the delta stays the size of one row.
        let wide = [
            (0, format!(r#"{{"rows":[{}]}}"#, rows(400))),
            (1, format!(r#"{{"rows":[{},{}]}}"#, rows(400),
                        r#"{"id":999,"v":"row body 999 with some width to it"}"#)),
        ];
        let wide: Vec<(i64, &str)> = wide.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (wide_cache, wide_index, wide_sha) = build_revive(168, &wide);
        let wide_delta = wide_index.hours()[1].cache_record_len() - RECORD_HEADER_LEN;
        assert!(
            wide_delta < delta + 8,
            "fifty times the array for the same one insertion: {wide_delta} against {delta}"
        );
        ReplayCacheReader::open(&wide_index.to_bytes().unwrap(), &wide_cache, 200, &wide_sha)
            .unwrap()
            .verify_every_hour()
            .unwrap();

        // A ROW REMOVED FROM THE MIDDLE, and a row changed beside it, both
        // reconstruct exactly -- the interesting direction, because the cursor
        // has to skip and patch in one script.
        let mixed = [
            (0, format!(r#"{{"rows":[{}]}}"#, rows(8))),
            (
                1,
                r#"{"rows":[{"id":0,"v":"row body 0 with some width to it"},{"id":1,"v":"changed"},{"id":3,"v":"row body 3 with some width to it"},{"id":4,"v":"row body 4 with some width to it"},{"id":5,"v":"row body 5 with some width to it"},{"id":6,"v":"row body 6 with some width to it"},{"id":7,"v":"row body 7 with some width to it"}]}"#
                    .to_string(),
            ),
        ];
        let mixed: Vec<(i64, &str)> = mixed.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (mixed_cache, mixed_index, mixed_sha) = build_revive(168, &mixed);
        ReplayCacheReader::open(
            &mixed_index.to_bytes().unwrap(),
            &mixed_cache,
            200,
            &mixed_sha,
        )
        .unwrap()
        .verify_every_hour()
        .unwrap();
        // And it COSTS the removal and the one changed row, not the array: the
        // six untouched rows are about 45 bytes each, so a replacement of the
        // seven surviving rows could not fit in this.
        let mixed_delta = mixed_index.hours()[1].cache_record_len() - RECORD_HEADER_LEN;
        assert!(
            mixed_delta < 90,
            "a middle removal beside a changed row must cost the two, not the seven: \
             {mixed_delta} bytes"
        );
    }

    /// A ROW'S MEMORY MOVES WITH THE ROW.
    ///
    /// An element shadow is keyed by index, and the edit script is the one
    /// patch that changes which index a row sits at. A member that folded out
    /// while its row was at index 1 must still revive when the row has been
    /// pushed to index 2 by an insertion in front of it.
    ///
    /// kills: leaving `shadow.elements` keyed by the OLD index across an edit
    /// (the revive resolves against the wrong row's memory or none at all, and
    /// `verify_every_hour` refuses the recording).
    #[test]
    fn an_edited_arrays_memories_follow_their_rows() {
        let body = r#"{"deep":[1,2,3,4,5,6,7,8],"wide":"a long enough value to notice"}"#;
        let hours = [
            // Two rows; the second row's `dom` is live.
            (
                0,
                format!(r#"{{"rows":[{{"dom":null,"id":1}},{{"dom":{body},"id":2}}]}}"#),
            ),
            // It folds out. The memory is filed under element 1.
            (
                1,
                r#"{"rows":[{"dom":null,"id":1},{"dom":null,"id":2}]}"#.to_string(),
            ),
            // A row is INSERTED in front of it, so the row that owns the memory
            // is now element 2, and the memory has to have moved with it.
            (
                2,
                r#"{"rows":[{"dom":null,"id":0},{"dom":null,"id":1},{"dom":null,"id":2}]}"#
                    .to_string(),
            ),
            // And it comes back unchanged, which is a revive or nothing.
            (
                3,
                format!(
                    r#"{{"rows":[{{"dom":null,"id":0}},{{"dom":null,"id":1}},{{"dom":{body},"id":2}}]}}"#
                ),
            ),
        ];
        let hours: Vec<(i64, &str)> = hours.iter().map(|(h, b)| (*h, b.as_str())).collect();
        let (cache, index, sha) = build_revive(168, &hours);
        ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 400, &sha)
            .unwrap()
            .verify_every_hour()
            .unwrap();

        // The memory really was used, against the control that cannot use one:
        // the same hours through the encoder with revives forbidden pay for the
        // body again, and this one does not.
        let (_, control, _) = build(168, &hours);
        let revived = index.hours()[3].cache_record_len();
        let re_emitted = control.hours()[3].cache_record_len();
        assert!(
            revived + (body.len() as u64 / 2) < re_emitted,
            "a row whose memory moved with it must revive: {revived} against {re_emitted}"
        );
    }

    /// EVERY SECOND SPELLING OF AN EDIT SCRIPT IS REFUSED.
    ///
    /// The script is only worth its tag if one change has one spelling, so the
    /// decoder rejects the shapes the encoder cannot produce: a run of zero, a
    /// run split into two ops, a script that only copies and patches (which is
    /// `Patch::Array`), one that empties its target (which is a `Replace`), and
    /// one whose cursor does not consume its target exactly.
    ///
    /// kills: removing any one of those five checks from `decode_patch` or
    /// `apply_patch`.
    #[test]
    fn array_edit_scripts_with_a_second_spelling_are_refused() {
        // [tag][ops][kind][arg]..., decoded one level down because the world
        // root is an object and the decoder refuses an edit at depth 0.
        let nest = |script: &[u8]| {
            let mut bytes = vec![PATCH_OBJECT, 0, 1, 1, b'x'];
            bytes.extend_from_slice(script);
            decode_patch_exact(&bytes)
        };
        assert!(
            decode_patch_exact(&[PATCH_ARRAY_EDIT, 1, 1, 1])
                .unwrap_err()
                .to_string()
                .contains("world root"),
            "the world root is an object, never an array"
        );

        let zero_run = [PATCH_ARRAY_EDIT, 2, 0, 0, 1, 1];
        assert!(
            nest(&zero_run)
                .unwrap_err()
                .to_string()
                .contains("run of zero"),
            "a run of zero elements is not an op"
        );

        let split_run = [PATCH_ARRAY_EDIT, 3, 0, 1, 0, 1, 1, 1];
        assert!(
            nest(&split_run)
                .unwrap_err()
                .to_string()
                .contains("splits a run"),
            "two copies in a row are one copy"
        );

        let copies_only = [PATCH_ARRAY_EDIT, 1, 0, 3];
        assert!(
            nest(&copies_only)
                .unwrap_err()
                .to_string()
                .contains("neither skips nor inserts"),
            "an edit that only copies is an array patch"
        );

        // Applied shapes: the cursor must land exactly on the end of the target.
        let mut target = CanonValue::parse(br#"[1,2,3]"#).unwrap();
        let short = Patch::ArrayEdit(vec![ArrayEditOp::Copy(1), ArrayEditOp::Skip(1)]);
        assert!(
            apply_patch(
                &mut target,
                &short,
                &mut Shadow::default(),
                &mut ShadowBudget::default(),
                1
            )
            .unwrap_err()
            .to_string()
            .contains("unconsumed"),
            "a script that stops early leaves rows unaccounted for"
        );

        let mut target = CanonValue::parse(br#"[1,2]"#).unwrap();
        let over = Patch::ArrayEdit(vec![ArrayEditOp::Copy(3), ArrayEditOp::Skip(1)]);
        assert!(
            apply_patch(
                &mut target,
                &over,
                &mut Shadow::default(),
                &mut ShadowBudget::default(),
                1
            )
            .unwrap_err()
            .to_string()
            .contains("past the end"),
            "a script cannot copy rows its target does not have"
        );

        let mut target = CanonValue::parse(br#"[1,2]"#).unwrap();
        let emptied = Patch::ArrayEdit(vec![ArrayEditOp::Skip(2)]);
        assert!(
            apply_patch(
                &mut target,
                &emptied,
                &mut Shadow::default(),
                &mut ShadowBudget::default(),
                1
            )
            .unwrap_err()
            .to_string()
            .contains("empties its target"),
            "an emptied array is a replacement, which is shorter"
        );

        // A SCRIPT THAT KEEPS NO ROW is a replacement too, and it is refused at
        // the bytes rather than at the target -- `SKIP 2; INSERT 3` over `[1,2]`
        // reconstructs `[3]` perfectly and is still not this format's spelling
        // of it (Codex round 1, finding 5).
        let keeps_nothing = [PATCH_ARRAY_EDIT, 2, 1, 2, 3, INLINE_UINT_BASE + 3];
        assert!(
            nest(&keeps_nothing)
                .unwrap_err()
                .to_string()
                .contains("keeps no row"),
            "an edit that keeps nothing is a replacement"
        );

        // A ROW PATCH THAT IS NOT AN OBJECT PATCH is the same hole one step
        // along: `PATCH(Replace(3)); SKIP 1` over `[1,2]` reconstructs `[3]`,
        // keeps a row by the letter of the rule above, and is still not this
        // format's spelling -- rows only align when they are objects (Codex
        // round 2, finding 2).
        let scalar_row_patch = [PATCH_ARRAY_EDIT, 2, 2, PATCH_REPLACE, INLINE_UINT_BASE + 3, 1, 1];
        assert!(
            nest(&scalar_row_patch)
                .unwrap_err()
                .to_string()
                .contains("not an object patch"),
            "an aligned row is an object, so its patch is an object patch"
        );

        // AND A SCRIPT THAT LANDS ON THE SAME LENGTH is an array patch: the
        // encoder only reaches for a script when the lengths differ.
        let mut target = CanonValue::parse(br#"[{"id":1},{"id":2}]"#).unwrap();
        let same_length = Patch::ArrayEdit(vec![
            ArrayEditOp::Copy(1),
            ArrayEditOp::Skip(1),
            ArrayEditOp::Insert(CanonValue::parse(br#"{"id":3}"#).unwrap()),
        ]);
        assert!(
            apply_patch(
                &mut target,
                &same_length,
                &mut Shadow::default(),
                &mut ShadowBudget::default(),
                1
            )
            .unwrap_err()
            .to_string()
            .contains("keeps its target's length"),
            "an array that keeps its length is an array patch"
        );
    }

    /// AN ARRAY WITH NO IDENTITY ON ITS ROWS STAYS A REPLACE, and so does one
    /// where the script would not be shorter.
    ///
    /// The key is DISCOVERED -- a member every row of both arrays carries, with
    /// a scalar value unique in each -- so rows of scalars, rows that share a
    /// value, and rows that do not all carry the same members have no alignment
    /// at all. That is not a failure: it is the case where restating the array
    /// is the honest spelling.
    ///
    /// kills: naming a key member in the format instead of discovering one;
    /// accepting a key whose values repeat (the map from key to index would
    /// then drop rows silently); emitting an edit that is longer than the
    /// replace it displaces.
    #[test]
    fn an_array_with_no_row_identity_stays_a_replace() {
        let numbers = CanonValue::parse(br#"[1,2,3]"#).unwrap();
        let more_numbers = CanonValue::parse(br#"[1,2,3,4]"#).unwrap();
        assert!(
            matches!(
                diff_value(&numbers, &more_numbers, None, 0).unwrap(),
                Patch::Replace(_)
            ),
            "rows that are not objects carry no identity"
        );

        let repeated = CanonValue::parse(br#"[{"id":1},{"id":1}]"#).unwrap();
        let repeated_more = CanonValue::parse(br#"[{"id":1},{"id":1},{"id":2}]"#).unwrap();
        assert!(
            matches!(
                diff_value(&repeated, &repeated_more, None, 0).unwrap(),
                Patch::Replace(_)
            ),
            "a member that repeats is not an identity"
        );

        let mismatched = CanonValue::parse(br#"[{"id":1},{"other":2}]"#).unwrap();
        let mismatched_more = CanonValue::parse(br#"[{"id":1},{"other":2},{"id":3}]"#).unwrap();
        assert!(
            matches!(
                diff_value(&mismatched, &mismatched_more, None, 0).unwrap(),
                Patch::Replace(_)
            ),
            "a member only some rows carry is not an identity"
        );

        // Aligned -- every row carries `id` and no two share one -- but not one
        // row of the old array survives, so the script is two inserts and a
        // skip where a replace is two rows. The longer spelling is not written.
        let tiny = CanonValue::parse(br#"[{"id":1}]"#).unwrap();
        let replaced = CanonValue::parse(br#"[{"id":2},{"id":3}]"#).unwrap();
        assert!(
            matches!(
                diff_value(&tiny, &replaced, None, 0).unwrap(),
                Patch::Replace(_)
            ),
            "an edit that is not shorter is not written"
        );

        // The same shape one row wider IS shorter as a script, which is what
        // makes the assertion above a measurement rather than a coincidence.
        let kept = CanonValue::parse(br#"[{"id":1},{"id":2}]"#).unwrap();
        let kept_more = CanonValue::parse(br#"[{"id":1},{"id":2},{"id":3}]"#).unwrap();
        assert!(
            matches!(
                diff_value(&kept, &kept_more, None, 0).unwrap(),
                Patch::ArrayEdit(_)
            ),
            "rows that survive are copied, not restated"
        );

        // THE KEY IS DISCOVERED, NOT THE NAME `id`. These rows carry a
        // mod-shaped identity under a name this format has never heard of, as a
        // STRING, and it does not sort first among their members. An aligner
        // that knew a name instead of a property would restate these.
        let modded = CanonValue::parse(
            br#"[{"aaa_first":1,"zz_unit_key":"BRIG-a"},{"aaa_first":1,"zz_unit_key":"BRIG-b"}]"#,
        )
        .unwrap();
        let modded_more = CanonValue::parse(
            br#"[{"aaa_first":1,"zz_unit_key":"BRIG-a"},{"aaa_first":1,"zz_unit_key":"BRIG-b"},{"aaa_first":1,"zz_unit_key":"BRIG-c"}]"#,
        )
        .unwrap();
        assert_eq!(
            array_alignment_key(
                match &modded {
                    CanonValue::Array(rows) => rows,
                    _ => unreachable!("parsed as an array"),
                },
                match &modded_more {
                    CanonValue::Array(rows) => rows,
                    _ => unreachable!("parsed as an array"),
                }
            )
            .as_deref(),
            Some("zz_unit_key"),
            "the key is the member that IS an identity, whatever it is called"
        );
        assert!(
            matches!(
                diff_value(&modded, &modded_more, None, 0).unwrap(),
                Patch::ArrayEdit(_)
            ),
            "rows a mod named itself align like any other"
        );
    }

    /// A MEMORY BELONGS TO A PATH, NOT TO A NAME. Removing a member and adding
    /// one back with the same name must not revive the removed body.
    ///
    /// The difference matters because a removed member and a nulled one look
    /// alike from the outside and mean opposite things: the world no longer has
    /// that path at all, versus it has it and it is currently empty.
    ///
    /// kills: leaving the child shadow in place across an object removal, or
    /// across an insertion of a member that was not there.
    #[test]
    fn a_removed_member_leaves_no_memory_to_revive() {
        let big = r#"{"a":[1,2,3,4,5,6,7,8],"b":"a long enough value to notice"}"#;
        let (cache, index, sha) = build_revive(
            168,
            &[
                (0, &format!(r#"{{"dom":{big}}}"#)),
                (1, r#"{"other":1}"#),
                (2, &format!(r#"{{"dom":{big},"other":1}}"#)),
            ],
        );
        let payload = |position: usize| index.hours()[position].cache_record_len() - RECORD_HEADER_LEN;
        assert!(
            payload(2) > 40,
            "a member that was REMOVED, not nulled, has no memory: spent only {}",
            payload(2)
        );
        let reader = ReplayCacheReader::open(&index.to_bytes().unwrap(), &cache, 300, &sha).unwrap();
        reader.verify_every_hour().unwrap();
    }

    /// THE CONFINEMENT LAW, at the resolution it is stated in: one damaged
    /// record costs its own hour and the rest of its checkpoint window.
    ///
    /// Nine hours at interval 4 give three windows — 0-3, 4-7, and the one-hour
    /// remainder at 8 — so the damaged record here sits strictly inside the
    /// second one, with intact hours before it in the same window and intact
    /// windows on both sides. Hour 8 is not padding: it is what the "do not
    /// widen past the window" assertion rests on.
    ///
    /// kills: confining the whole recording after the damage (hour 8 stops
    /// serving); confining only the damaged hour (hours 6 and 7 are served out
    /// of a delta chain that reads the moved record); confining hours before
    /// the damage in the same window (hour 4 stops serving, though it never
    /// reads that record).
    ///
    /// It does NOT pin that the window comes from the index's own
    /// `checkpoint_position` rather than from `position % interval`: every
    /// index the encoder can write places its checkpoints regularly, so the two
    /// agree here (Codex review round 1, finding 1). What separates them is a
    /// named hole, and that is
    /// `a_named_hole_does_not_move_the_confined_window`.
    #[test]
    fn a_damaged_record_confines_its_checkpoint_window_and_nothing_else() {
        let bodies: Vec<(i64, String)> = (0..9)
            .map(|hour| (hour, format!(r#"{{"v":{hour}}}"#)))
            .collect();
        let hours: Vec<(i64, &str)> = bodies
            .iter()
            .map(|(hour, body)| (*hour, body.as_str()))
            .collect();
        let (cache, index, source_sha) = build(4, &hours);
        let index_bytes = index.to_bytes().unwrap();
        let source_len = 900;

        // Position 5: the second window is 4..=7, so the damage costs 5, 6, 7.
        let mut damaged = cache.clone();
        let row = &index.hours()[5];
        let last = (row.cache_record_offset() + row.cache_record_len() - 1) as usize;
        damaged[last] ^= 0xff;

        let reader =
            ReplayCacheReader::open(&index_bytes, &damaged, source_len, &source_sha).unwrap();
        assert_eq!(
            reader
                .confined_hours()
                .iter()
                .map(ConfinedHour::hour_index)
                .collect::<Vec<i64>>(),
            vec![5, 6, 7]
        );
        for confined in reader.confined_hours() {
            assert_eq!(confined.damaged_record_hour(), 5);
            assert_eq!(confined.checkpoint_hour(), 4);
        }
        for hour in [0, 1, 2, 3, 4, 8] {
            let (json, _) = reader
                .reconstruct_json(hour)
                .unwrap_or_else(|error| panic!("hour {hour} must still serve: {error}"));
            assert_eq!(json, world_json(hour, &bodies[hour as usize].1));
        }
        for hour in [5, 6, 7] {
            let error = reader.reconstruct_json(hour).unwrap_err();
            assert!(error.message().contains(&format!("game hour {hour}")), "{error}");
        }
    }

    /// A NAMED HOLE does not move the checkpoint window a damaged record
    /// confines.
    ///
    /// The two features meet here for the first time: lane GZ's holes are game
    /// hours the recording never vouched for, so they hold no cache record and
    /// take no position in the index's served rows — which is exactly why the
    /// delta chain, and therefore the window, is counted over SERVED positions.
    /// Six hours at interval 2 with hour 2 excluded gives served rows
    /// 0,1,3,4,5, whose windows are {0,1}, {3,4} and {5}. Damaging hour 3's
    /// record must cost hours 3 and 4 — not hour 5, and not hour 2, which is
    /// refused for its own, older reason.
    ///
    /// kills: computing the window from GAME HOURS rather than from the index's
    /// served positions and its `checkpoint_position` — the hole then shifts
    /// the arithmetic and hour 5 is confined with the rest, or hour 4 escapes.
    #[test]
    fn a_named_hole_does_not_move_the_confined_window() {
        let bodies: Vec<(i64, String)> = (0..6)
            .map(|hour| (hour, format!(r#"{{"v":{hour}}}"#)))
            .collect();
        let hours: Vec<(i64, &str)> = bodies
            .iter()
            .map(|(hour, body)| (*hour, body.as_str()))
            .collect();
        let (cache, index, source_sha) = build_with_hole(2, 2, &hours);
        assert_eq!(
            index
                .hours()
                .iter()
                .map(ReplayHourEntry::hour_index)
                .collect::<Vec<i64>>(),
            vec![0, 1, 3, 4, 5]
        );
        assert_eq!(index.hours()[2].hour_index(), 3, "position 2 serves hour 3");
        assert_eq!(index.hours()[2].delta_steps(), 0, "and is a checkpoint");

        let mut damaged = cache.clone();
        let row = &index.hours()[2];
        let last = (row.cache_record_offset() + row.cache_record_len() - 1) as usize;
        damaged[last] ^= 0xff;
        let index_bytes = index.to_bytes().unwrap();
        let reader =
            ReplayCacheReader::open(&index_bytes, &damaged, index.source_len(), &source_sha)
                .unwrap();
        assert_eq!(
            reader
                .confined_hours()
                .iter()
                .map(ConfinedHour::hour_index)
                .collect::<Vec<i64>>(),
            vec![3, 4]
        );
        for hour in [0, 1, 5] {
            reader
                .reconstruct_json(hour)
                .unwrap_or_else(|error| panic!("hour {hour} must still serve: {error}"));
        }
        let hole = reader.reconstruct_json(2).unwrap_err();
        assert!(
            hole.message().contains("is not indexed"),
            "an hour the recording never vouched for is refused as absent, not as confined: {hole}"
        );
    }

    /// AN INDEX WHOSE TWO ACCOUNTS OF ITS CACHE DISAGREE IS NOT AN AUTHORITY
    /// OVER IT.
    ///
    /// The index publishes a whole-cache digest and a digest per record. Over a
    /// cache of the published length those describe the same bytes, but nothing
    /// proves they agree with each other until a cache is in hand — `validate`
    /// has no cache to hash — so an index that carries one honest account and
    /// one forged one parses cleanly. Both directions are refused, and refused
    /// WHOLE: a self-contradictory index cannot be trusted to attribute damage
    /// to an hour, so confining anything on its word would be a guess.
    ///
    /// kills: either arm of the `(whole matches, records intact)` match in
    /// `admit_cache`. Dropping `(false, true)` admits an index whose row
    /// digests were rewritten to launder a substituted cache; dropping
    /// `(true, false)` confines hours over an index that says, in its own two
    /// voices, that the cache in hand both is and is not the sealed one.
    #[test]
    fn an_index_whose_two_accounts_disagree_is_not_an_authority() {
        let (cache, index, source_sha) = build(2, &[(0, r#"{"v":0}"#), (1, r#"{"v":1}"#)]);

        // The whole-cache digest is the honest one; a row's digest is not.
        let mut forged_row = index.clone();
        forged_row.hours[1].cache_record_sha256 = "ab".repeat(32);
        let error = match ReplayCacheReader::open(
            &forged_row.to_bytes().unwrap(),
            &cache,
            200,
            &source_sha,
        ) {
            Ok(_) => panic!("an index that contradicts itself is refused"),
            Err(error) => error,
        };
        assert!(
            error.message().contains("describes two different caches"),
            "{error}"
        );

        // The rows are honest; the whole-cache digest is not.
        let mut forged_whole = index;
        forged_whole.cache_sha256 = "cd".repeat(32);
        let error = match ReplayCacheReader::open(
            &forged_whole.to_bytes().unwrap(),
            &cache,
            200,
            &source_sha,
        ) {
            Ok(_) => panic!("an index that contradicts itself is refused"),
            Err(error) => error,
        };
        assert!(
            error.message().contains("does not add up"),
            "{error}"
        );
    }

    /// The three lengths a streamed cache can have, and the three different
    /// things the audit says about them (task #192).
    ///
    /// * SHORT — the verified prefix stands and the tail is confined. Every
    ///   position that closed against the digest the sealed index published is
    ///   vouched by a statement about bytes that missing bytes cannot revoke;
    ///   the record in flight and everything after it is unvouched and named.
    /// * LONG — refused whole. The index accounts for `cache_len` bytes and no
    ///   more, so extra bytes are described by no row, and an insertion
    ///   anywhere produces exactly this shape.
    /// * EXACT — the ordinary intact verdict, and it must survive being fed in
    ///   chunk sizes that fall nowhere near a record boundary.
    ///
    /// Asked of the audit directly: this is the only place the STREAMED length
    /// is known. The readers see a backing whose length they could compare
    /// themselves, but the container gate is handed an entry and cannot know
    /// its length in advance, so both go through this one verdict.
    ///
    /// kills: deleting `finish`'s `seen > cache_len` refusal admits an extended
    /// cache whose every named record still matches, over bytes no row
    /// describes. Deleting the `seen < CACHE_HEADER_LEN` refusal admits an
    /// artifact with no identity. Returning `truncated: None` for a short
    /// stream drops every unreached position out of `damaged_positions`, so a
    /// truncated cache is admitted as INTACT and its lost hours serve — the
    /// `first_unvouched` and `damaged_positions` assertions below are what
    /// catch it.
    #[test]
    fn an_audit_confines_a_short_cache_and_refuses_a_long_one() {
        let (cache, index, _) = build(2, &[(0, r#"{"v":0}"#), (1, r#"{"v":1}"#)]);

        // SHORT: one byte gone takes hour 1's record, which is the one in
        // flight when the bytes run out. Hour 0's record closed and matched
        // before it, so position 0 is vouched.
        let mut short = CacheAudit::new(&index);
        short.write_all(&cache[..cache.len() - 1]).unwrap();
        let verdict = short.finish().unwrap();
        let truncation = verdict
            .truncation()
            .expect("a stream that stopped short is reported as truncated");
        assert_eq!(truncation.published_len(), index.cache_len());
        assert_eq!(truncation.seen_len(), cache.len() as u64 - 1);
        assert_eq!(truncation.first_unvouched_position(), 1);
        assert_eq!(verdict.damaged_positions(), &[1]);
        assert!(!verdict.is_intact());

        // SHORT, before the header is even complete: nothing here can say
        // which recording these bytes are, so there is no hour to name.
        let mut headless = CacheAudit::new(&index);
        headless
            .write_all(&cache[..CACHE_HEADER_LEN as usize - 1])
            .unwrap();
        let error = headless.finish().unwrap_err();
        assert!(error.message().contains("short of its own"), "{error}");

        // LONG: refused whole.
        let mut long = CacheAudit::new(&index);
        long.write_all(&cache).unwrap();
        long.write_all(b"\0").unwrap();
        let error = long.finish().unwrap_err();
        assert!(error.message().contains("belong to no record"), "{error}");

        // The honest stream, fed in awkward chunks: a record boundary must be
        // found from the index's offsets, never from where a write happened to
        // land.
        let mut split = CacheAudit::new(&index);
        for chunk in cache.chunks(7) {
            split.write_all(chunk).unwrap();
        }
        let verdict = split.finish().unwrap();
        assert!(verdict.is_intact());
        assert!(verdict.truncation().is_none());
        assert_eq!(verdict.whole_cache_sha256(), index.cache_sha256());
        assert_eq!(verdict.header().len(), CACHE_HEADER_LEN as usize);
    }

    /// The per-record digest covers EXACTLY the window the index publishes for
    /// that record — its 21-byte header and its payload, and nothing of its
    /// neighbours.
    ///
    /// kills: hashing the payload alone, hashing a fixed length from the record
    /// offset, or including the following record's header — each makes the row
    /// publish a digest of bytes that are not the record, and the assertions
    /// below fail.
    ///
    /// What payload-only hashing costs is the CONFINEMENT, not an admission
    /// (Codex review round 5, finding 2): a rewritten record header still moves
    /// the whole-cache digest, so `admit_cache`'s `(false, true)` arm refuses
    /// the artifact outright — one flipped header byte becomes a whole-artifact
    /// refusal again instead of one named window.
    #[test]
    fn the_per_record_digest_is_over_exactly_the_indexed_window() {
        let (cache, index, _) = build(
            3,
            &[
                (0, r#"{"v":0}"#),
                (1, r#"{"v":1}"#),
                (2, r#"{"v":2}"#),
                (3, r#"{"v":3}"#),
            ],
        );
        for row in index.hours() {
            let start = row.cache_record_offset() as usize;
            let end = start + row.cache_record_len() as usize;
            assert_eq!(
                row.cache_record_sha256(),
                Sha256::hex(&cache[start..end]),
                "hour {} row publishes a digest of some other bytes",
                row.hour_index()
            );
            assert_ne!(
                row.cache_record_sha256(),
                Sha256::hex(&cache[start + RECORD_HEADER_LEN as usize..end]),
                "the digest must cover the record HEADER too"
            );
        }
    }

    /// Scratch file that removes itself, so a file-backed test leaves nothing.
    struct TempCacheFile(std::path::PathBuf);

    impl TempCacheFile {
        fn write(tag: &str, bytes: &[u8]) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "h4cp-{tag}-{}-{}.bin",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, bytes).expect("write scratch cache");
            Self(path)
        }

        fn open(&self) -> File {
            File::open(&self.0).expect("open scratch cache")
        }
    }

    impl Drop for TempCacheFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// The whole point of the file backing: identical answers, no residency.
    ///
    /// Production change that would make this fail: any divergence between the
    /// two backings — a different admission rule, a different record window, a
    /// different reconstruction — including the easy mistake of having the file
    /// backing hash something other than the complete cache.
    #[test]
    fn a_file_backed_cache_answers_exactly_as_a_resident_one() {
        let hours: Vec<(i64, &str)> = vec![
            (0, r#"{"keep":true,"rows":[1,2]}"#),
            (1, r#"{"keep":true,"rows":[1,3]}"#),
            (2, r#"{"extra":"x","rows":[1,3]}"#),
            (3, r#"{"extra":"x","rows":[9]}"#),
            (4, r#"{"extra":"y","rows":[9]}"#),
        ];
        let (cache, index, source_sha) = build(3, &hours);
        let index_bytes = index.to_bytes().unwrap();
        let source_len = 500;

        let scratch = TempCacheFile::write("parity", &cache);
        let resident =
            CertifiedReplayCache::open(&index_bytes, cache.clone(), source_len, &source_sha)
                .unwrap();
        let on_disk =
            CertifiedReplayCache::open_file(&index_bytes, scratch.open(), source_len, &source_sha)
                .unwrap();

        for (hour, body) in &hours {
            let (resident_json, resident_receipt) = resident.reconstruct_json(*hour).unwrap();
            let (disk_json, disk_receipt) = on_disk.reconstruct_json(*hour).unwrap();
            assert_eq!(resident_json, world_json(*hour, body));
            assert_eq!(disk_json, resident_json, "hour {hour} differs by backing");
            assert_eq!(disk_receipt.applied_deltas, resident_receipt.applied_deltas);
            assert_eq!(disk_receipt.checkpoint_hour, resident_receipt.checkpoint_hour);
        }
        assert_eq!(
            on_disk.verify_every_hour().unwrap(),
            resident.verify_every_hour().unwrap()
        );
    }

    /// A file whose bytes are not the cache the index is bound to is judged by
    /// the same admission gate the resident reader uses, and reaches the same
    /// verdict: the hours a moved record reaches are refused by name, and so
    /// are the hours a TRUNCATED file's lost tail reaches.
    ///
    /// kills: this test pins that a FINAL-BYTE mutation of a file-backed cache
    /// is CONFINED rather than refused whole, and confined exactly. The flipped
    /// byte is the very last one in the file — the position a pass that stopped
    /// early would miss — and the two assertions divide the work (Codex review
    /// round 8, finding 4): the `expect` fails if the per-record account misses
    /// that byte while the whole-cache account still covers it, because
    /// `admit_cache`'s `(false, true)` arm then refuses the artifact at open
    /// and there is nothing to confine; the `confined_hours()` comparison fails
    /// if the confinement is too wide (hour 0) or too narrow (empty).
    ///
    /// It does NOT pin the admission side of that failure: hashing a record
    /// PREFIX still produces a digest that is not the sealed one, so the record
    /// is still classified as damaged (Codex review round 5, finding 3).
    ///
    /// The truncation half additionally kills restoring `admit_cache`'s
    /// `index.cache_len != cache.cache_len()` refusal, which is the whole-cache
    /// door task #192 removed: with it back, the short file is refused before a
    /// single per-record digest is read and hour 0 is lost with hour 1.
    #[test]
    fn a_file_backed_cache_that_is_not_the_indexed_one_is_refused() {
        let hours: Vec<(i64, &str)> = vec![(0, r#"{"v":1}"#), (1, r#"{"v":2}"#)];
        let (cache, index, source_sha) = build(2, &hours);
        let index_bytes = index.to_bytes().unwrap();

        let mut flipped = cache.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        let scratch = TempCacheFile::write("flipped", &flipped);
        let confined =
            CertifiedReplayCache::open_file(&index_bytes, scratch.open(), 200, &source_sha)
                .expect("post-seal rot in one record costs that record's window, not the file");
        assert_eq!(
            confined
                .confined_hours()
                .iter()
                .map(ConfinedHour::hour_index)
                .collect::<Vec<i64>>(),
            vec![1],
            "the flipped byte is in hour 1's record"
        );
        confined.reconstruct_json(0).expect("hour 0 still serves");
        assert!(confined.reconstruct_json(1).is_err());

        // A TRUNCATED file goes through the same door and keeps its verified
        // prefix (task #192). One byte short costs hour 1's record; hour 0's
        // record closed against the sealed digest before the bytes ran out, so
        // hour 0 still serves and hour 1 is refused by a name that says the
        // record is not wholly in the artifact.
        let mut truncated = cache.clone();
        truncated.pop();
        let scratch = TempCacheFile::write("truncated", &truncated);
        let short = CertifiedReplayCache::open_file(&index_bytes, scratch.open(), 200, &source_sha)
            .expect("a truncated cache serves its verified prefix");
        assert_eq!(
            short
                .confined_hours()
                .iter()
                .map(ConfinedHour::hour_index)
                .collect::<Vec<i64>>(),
            vec![1],
            "the lost byte is in hour 1's record and costs hour 1 alone"
        );
        short
            .reconstruct_json(0)
            .expect("hour 0's record arrived whole and hashes to the sealed digest");
        let error = short.reconstruct_json(1).unwrap_err();
        assert!(
            error.message().contains("ends at")
                && error.message().contains("not wholly in the artifact"),
            "a truncated tail is named as truncation, not as a rotted record: {error}"
        );
        assert!(
            short.verify_every_hour().is_err(),
            "the producer's total proof must not hold over a cache that lost a record"
        );
    }

    /// The per-record bound is what replaced the whole-entry one, so it has to
    /// refuse BEFORE the length is used to size anything, and it has to refuse
    /// only what is genuinely over it.
    ///
    /// Production change that would make this fail: deleting the bound (both
    /// halves then report the out-of-range error), or applying it to the wrong
    /// value so that a legal record at the edge is refused too.
    #[test]
    fn a_record_length_past_the_per_record_bound_is_refused_by_name() {
        let hours: Vec<(i64, &str)> = vec![(0, r#"{"v":1}"#), (1, r#"{"v":2}"#)];
        let (cache, index, source_sha) = build(2, &hours);
        let index_bytes = index.to_bytes().unwrap();

        assert!(ReplayCacheReader::open(&index_bytes, &cache, 200, &source_sha).is_ok());

        let mut over = index.hours[1].clone();
        over.cache_record_len = MAX_CACHE_RECORD_BYTES + 1;
        let error = cache_record(&SliceCache(&cache), &over)
            .expect_err("a record past the bound must not be read");
        assert!(
            error.message().contains("per-record reader bound"),
            "{error}"
        );

        // Exactly at the bound passes the bound. This fixture is still a bad
        // record for a different reason — the index now claims 2 GiB while the
        // record's own header declares a 47-byte payload — and the refusal must
        // name THAT, which it can only do if the bound let it through first.
        let mut at = index.hours[1].clone();
        at.cache_record_len = MAX_CACHE_RECORD_BYTES;
        let error = cache_record(&SliceCache(&cache), &at)
            .expect_err("this fixture is not a valid record either way");
        assert!(
            error.message().contains("but the index published a"),
            "a record exactly at the bound must reach the header check: {error}"
        );
    }

    /// A record whose own header disagrees with the length the index published
    /// is refused, and refused before the payload is requested.
    ///
    /// Production change that would make this fail: dropping the header/index
    /// agreement, or reading the payload first and only noticing afterwards.
    #[test]
    fn a_record_that_disagrees_with_its_indexed_length_is_refused() {
        let hours: Vec<(i64, &str)> = vec![(0, r#"{"v":1}"#), (1, r#"{"v":2}"#)];
        let (cache, index, source_sha) = build(2, &hours);
        let index_bytes = index.to_bytes().unwrap();
        assert!(ReplayCacheReader::open(&index_bytes, &cache, 200, &source_sha).is_ok());

        let mut shortened = index.hours[1].clone();
        shortened.cache_record_len -= 1;
        let error = cache_record(&SliceCache(&cache), &shortened)
            .expect_err("index and record must agree on the record's length");
        assert!(
            error.message().contains("but the index published a"),
            "{error}"
        );

        let mut lengthened = index.hours[0].clone();
        lengthened.cache_record_len += 1;
        let error = cache_record(&SliceCache(&cache), &lengthened)
            .expect_err("index and record must agree on the record's length");
        assert!(
            error.message().contains("but the index published a"),
            "{error}"
        );

        // "Refused before the payload is requested" is an ordering claim, so it
        // is proved by counting reads rather than by reading the code: exactly
        // one read — the 21-byte header — reaches the backing before the
        // disagreement is named. A reader that fetched the whole record first
        // and checked afterwards would still return the same error and would
        // still be a defect, because the length it trusted is the forged one.
        let counting = CountingCache {
            inner: SliceCache(&cache),
            reads: std::cell::RefCell::new(Vec::new()),
        };
        let mut forged = index.hours[1].clone();
        forged.cache_record_len -= 1;
        cache_record(&counting, &forged).expect_err("the forged record is refused");
        assert_eq!(
            counting.reads.borrow().as_slice(),
            &[(index.hours[1].cache_record_offset, RECORD_HEADER_LEN)],
            "only the record header may be read before the length agrees"
        );

        // The honest record reads twice: header, then payload.
        let counting = CountingCache {
            inner: SliceCache(&cache),
            reads: std::cell::RefCell::new(Vec::new()),
        };
        cache_record(&counting, &index.hours[1]).expect("a well-formed record reads");
        assert_eq!(counting.reads.borrow().len(), 2);
    }

    /// Records every `(offset, len)` its wrapped backing is asked for.
    struct CountingCache<'a> {
        inner: SliceCache<'a>,
        reads: std::cell::RefCell<Vec<(u64, u64)>>,
    }

    impl CacheRecords for CountingCache<'_> {
        fn read_all(&self, sink: &mut dyn Write) -> Result<(), ReplayCacheError> {
            self.inner.read_all(sink)
        }

        fn record(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>, ReplayCacheError> {
            self.reads.borrow_mut().push((offset, len));
            self.inner
                .record(offset, len)
                .map(|bytes| Cow::Owned(bytes.into_owned()))
        }
    }

    #[test]
    fn sparse_checkpoints_reconstruct_every_hour_with_a_hard_seek_bound() {
        let hours: Vec<(i64, &str)> = vec![
            (0, r#"{"keep":true,"rows":[1,2]}"#),
            (1, r#"{"keep":true,"rows":[1,3]}"#),
            (2, r#"{"extra":"x","rows":[1,3]}"#),
            (3, r#"{"extra":"x","rows":[9]}"#),
            (4, r#"{"extra":"y","rows":[9]}"#),
        ];
        let (cache, index, source_sha) = build(3, &hours);
        assert_eq!(
            index
                .hours()
                .iter()
                .filter(|hour| hour.delta_steps() == 0)
                .map(ReplayHourEntry::hour_index)
                .collect::<Vec<_>>(),
            [0, 3]
        );
        let index_bytes = index.to_bytes().unwrap();
        let parsed = ReplayHourIndex::from_bytes(&index_bytes).unwrap();
        assert_eq!(parsed, index);
        let reader = ReplayCacheReader::open(&index_bytes, &cache, 500, &source_sha).unwrap();
        for (hour, body) in &hours {
            let (json, receipt) = reader.reconstruct_json(*hour).unwrap();
            assert_eq!(json, world_json(*hour, body));
            assert!(receipt.applied_deltas <= 2);
            assert_eq!(receipt.configured_bound, 2);
        }
    }

    /// The defect this format version exists to close.
    ///
    /// Each token here is one `serde_json` cannot give back: the first five are
    /// real capture-t26 hour-17032314 spellings that reparse one ULP low, the
    /// rest are spellings `serde_json`'s serializer never emits or integers its
    /// `Number` cannot hold at all. The test asserts BOTH halves — that a
    /// `Value` round trip really does move the token (so the fixture cannot rot
    /// into a tautology) and that the cache reconstructs the byte exactly.
    #[test]
    fn number_spellings_serde_json_cannot_reproduce_survive_a_seek() {
        for token in NON_IDENTITY_TOKENS {
            let source = format!(r#"{{"v":{token}}}"#);
            let value: Value = serde_json::from_str(&source).unwrap();
            assert_ne!(
                serde_json::to_string(&value).unwrap(),
                source,
                "fixture token {token} round-trips through serde_json unchanged, \
                 so it no longer witnesses the defect"
            );

            let body = format!(r#"{{"v":{token}}}"#);
            let hours: Vec<(i64, &str)> = vec![(0, r#"{"v":0}"#), (1, body.as_str())];
            let (cache, index, source_sha) = build(2, &hours);
            let index_bytes = index.to_bytes().unwrap();
            let reader = ReplayCacheReader::open(&index_bytes, &cache, 200, &source_sha).unwrap();
            let (json, _) = reader.reconstruct_json(1).unwrap();
            assert_eq!(
                json,
                world_json(1, &body),
                "token {token} did not come back byte-exact"
            );
        }
    }

    #[test]
    fn string_escape_spellings_survive_a_seek() {
        for body in [
            r#"{"v":"café"}"#,
            r#"{"v":"café"}"#,
            r#"{"v":"😀"}"#,
            r#"{"v":"A\/\b\f\n\r\t\"\\"}"#,
            r#"{"v":"é中文😀"}"#,
        ] {
            let hours: Vec<(i64, &str)> = vec![(0, r#"{"v":""}"#), (1, body)];
            let (cache, index, source_sha) = build(2, &hours);
            let index_bytes = index.to_bytes().unwrap();
            let reader = ReplayCacheReader::open(&index_bytes, &cache, 200, &source_sha).unwrap();
            let (json, _) = reader.reconstruct_json(1).unwrap();
            assert_eq!(json, world_json(1, body), "string body {body} moved");
        }
    }

    /// A world body the cache model cannot spell back is a BUILD failure, never
    /// a silently different seek.
    ///
    /// `push` no longer re-serializes each hour to prove that inline -- it is
    /// the SEAL that proves it, over the finished artifact, and this test holds
    /// that seal to the same defect. The recorded `world_json_sha256` is taken
    /// from the source bytes; the record carries the parse; so a body whose
    /// spelling the value model does not preserve makes the two disagree and
    /// `verify_every_hour` refuses the artifact. Delete the digest comparison
    /// in `verify_every_hour` and this test fails.
    #[test]
    fn a_body_that_does_not_round_trip_is_refused_when_the_artifact_is_sealed() {
        let source_sha = "cd".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 2).unwrap();
        let mut hour = world(0, r#"{"v":1}"#);
        // Whitespace is legal JSON and parses, but it is not what the value
        // model re-emits, so the source is not reproducible from the cache.
        hour.world_json = String::from_utf8(hour.world_json)
            .unwrap()
            .replace(r#""world":"#, r#""world": "#)
            .into_bytes();
        let source_len = hour.h4cr_end_offset;
        encoder
            .push(hour)
            .expect("the body parses; only its spelling is unreproducible");
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::IdentityV1, source_len)
            .unwrap();
        let index_bytes = index.to_bytes().unwrap();
        let reader =
            ReplayCacheReader::open(&index_bytes, &cache, source_len, &source_sha).unwrap();
        let error = reader.verify_every_hour().unwrap_err().to_string();
        assert!(
            error.contains("does not byte-match replay_decoder parity hash"),
            "{error}"
        );
    }

    #[test]
    fn member_names_out_of_decoded_order_are_refused_at_push() {
        let source_sha = "cd".repeat(32);
        for (body, needle) in [
            (r#"{"b":1,"a":2}"#, "strictly increasing"),
            (r#"{"a":1,"a":2}"#, "strictly increasing"),
        ] {
            let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 2).unwrap();
            let error = encoder.push(world(0, body)).unwrap_err().to_string();
            assert!(error.contains(needle), "{body}: {error}");
        }
    }

    /// An object member name carrying a JSON escape is reachable from a real
    /// recording, not a hypothetical: domain 51 admits every printable ASCII
    /// byte in a scripted flag/variable name — `"` and `\` included — and the
    /// world fold inserts those names directly as JSON object members. Whatever
    /// the cache does with them, it must not be "refuse the capture".
    ///
    /// It must also order them the way `serde_json` does, which is by the
    /// DECODED name: `mod"flag` sorts before `modXflag`, while the stored body
    /// `mod\"flag` does not.
    #[test]
    fn escaped_member_names_from_a_scripted_variable_seek_byte_exactly() {
        let bodies = [
            r#"{"_r2_script_variables":{"a":1}}"#,
            r#"{"_r2_script_variables":{"mod\"flag":2,"modXflag":4,"mod\\path":3}}"#,
            r#"{"_r2_script_variables":{"mod\"flag":5,"modXflag":4,"mod\\path":3}}"#,
        ];
        let hours: Vec<(i64, &str)> = bodies
            .iter()
            .enumerate()
            .map(|(position, body)| (position as i64, *body))
            .collect();
        let (cache, index, source_sha) = build(2, &hours);
        let index_bytes = index.to_bytes().unwrap();
        let reader = ReplayCacheReader::open(&index_bytes, &cache, 300, &source_sha).unwrap();
        for (hour, body) in &hours {
            let (json, _) = reader.reconstruct_json(*hour).unwrap();
            assert_eq!(json, world_json(*hour, body), "hour {hour} moved");
        }
        // And this really is the order serde_json would have written.
        let value: Value =
            serde_json::from_str(r#"{"mod\"flag":2,"modXflag":4,"mod\\path":3}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"mod\"flag":2,"modXflag":4,"mod\\path":3}"#
        );
    }

    /// A NUMBER THAT MOVES A LITTLE COSTS A LITTLE. The measured lever: seven
    /// or eight bytes of fresh decimal digits become a one- or two-byte varint,
    /// and the hour still reconstructs byte for byte.
    ///
    /// kills: dropping the `NumberDelta` arm from `diff_value` (the delta hour
    /// grows back to the replacement size and the first assertion fails);
    /// spelling the reconstruction from the WIDER fraction width instead of the
    /// new token's (`47.4` comes back as `47.40000` and the seek assertion
    /// fails); adding the delta to the wrong side (the reconstruction is
    /// `47.33102` and the seek assertion fails).
    #[test]
    fn a_number_that_moves_a_little_costs_a_little() {
        let hours = [
            (0, r#"{"d":{"exp":944.34752,"org":47.36551,"str":227.2}}"#),
            (1, r#"{"d":{"exp":944.35001,"org":47.4,"str":227.19}}"#),
        ];
        let (cache, index, source_sha) = build(16, &hours);

        // THE ENCODER REALLY CHOSE THE OP, read back out of the record it
        // wrote: all three members are spelled as differences.
        fn count_number_deltas(patch: &Patch) -> usize {
            match patch {
                Patch::NumberDelta { .. } => 1,
                Patch::Revive(inner) => count_number_deltas(inner),
                Patch::Object { changes, .. } => {
                    changes.iter().map(|(_, p)| count_number_deltas(p)).sum()
                }
                Patch::Array(changes) => {
                    changes.iter().map(|(_, p)| count_number_deltas(p)).sum()
                }
                _ => 0,
            }
        }
        let row = &index.hours()[1];
        let start = (row.cache_record_offset() + RECORD_HEADER_LEN) as usize;
        let payload = &cache[start..start + (row.cache_record_len() - RECORD_HEADER_LEN) as usize];
        assert_eq!(count_number_deltas(&decode_patch_exact(payload).unwrap()), 3);

        // And each one is shorter than the replacement it stands in for.
        for (from, to) in [
            ("47.36551", "47.4"),
            ("227.2", "227.19"),
            ("944.34752", "944.35001"),
        ] {
            let (fraction_digits, delta) = number_delta_between(
                &CanonValue::Number(from.into()),
                &CanonValue::Number(to.into()),
            )
            .expect("eligible");
            let as_delta = encode_patch(&Patch::NumberDelta {
                fraction_digits,
                delta,
            })
            .unwrap();
            let as_replace =
                encode_patch(&Patch::Replace(CanonValue::Number(to.into()))).unwrap();
            assert!(
                as_delta.len() < as_replace.len(),
                "{from} -> {to}: {} vs {}",
                as_delta.len(),
                as_replace.len()
            );
        }

        let index_bytes = index.to_bytes().unwrap();
        let reader =
            ReplayCacheReader::open(&index_bytes, cache.as_slice(), 200, &source_sha).unwrap();
        for (hour, body) in &hours {
            let (json, _) = reader.reconstruct_json(*hour).unwrap();
            assert_eq!(json, world_json(*hour, body), "hour {hour} moved");
        }
    }

    /// The spellings the op must REFUSE, each because the arithmetic cannot put
    /// the recording's own bytes back.
    ///
    /// kills: dropping the exact-respell check in `number_delta_between` (the
    /// exponent and negative-zero cases start returning `Some` and their
    /// assertions fail); widening `plain_decimal` to accept an exponent form.
    #[test]
    fn a_number_whose_bytes_the_arithmetic_cannot_restore_stays_a_replace() {
        let number = |token: &str| CanonValue::Number(token.to_string());
        // An exponent form on either side.
        assert_eq!(number_delta_between(&number("3e-05"), &number("0.5")), None);
        assert_eq!(number_delta_between(&number("0.5"), &number("3e-05")), None);
        // Negative zero: the value is zero and the spelling is not.
        assert_eq!(number_delta_between(&number("1.0"), &number("-0.0")), None);
        // A token too wide to rescale without losing it.
        let wide = "1.".to_string() + &"1".repeat(40);
        assert_eq!(number_delta_between(&number("1.0"), &number(&wide)), None);
        // A change to a value that is not a number at all.
        assert_eq!(
            number_delta_between(&number("1.0"), &CanonValue::Null),
            None
        );
        // And the ordinary case still works, narrowing the fraction width.
        assert_eq!(
            number_delta_between(&number("47.36551"), &number("47.4")),
            Some((1, 3449))
        );
        assert_eq!(spell_decimal(474, 1), "47.4");
        assert_eq!(spell_decimal(-5, 3), "-0.005");
        assert_eq!(spell_decimal(0, 1), "0.0");
        assert_eq!(plain_decimal("-0.005"), Some((-5, 3)));
        assert_eq!(plain_decimal("108"), Some((108, 0)));
        assert_eq!(plain_decimal("1e5"), None);

        // The allocation-free price of a replacement is the price
        // `encode_value_into` actually charges. Drift between them would let
        // the encoder choose a spelling the applier prices differently, and
        // then refuse its own output.
        // The shapes chosen are the branch boundaries: the inline ceiling and
        // the first value above it, both magnitude branches, an integer-looking
        // token too wide for `u64` (which the 30-digit delta ceiling DOES
        // admit), the 127/128-byte token-length varint boundary, and ordinary
        // fallbacks.
        let long_127 = "0.".to_string() + &"1".repeat(125);
        let long_128 = "0.".to_string() + &"1".repeat(126);
        for token in [
            "0",
            "127",
            "128",
            "-1",
            "-300",
            "47.4",
            "944.35001",
            "3e-05",
            "18446744073709551616",
            long_127.as_str(),
            long_128.as_str(),
        ] {
            let value = CanonValue::Number(token.to_string());
            assert_eq!(
                encoded_number_len(token),
                Some(encode_value(&value).unwrap().len()),
                "{token}"
            );
        }
        assert_eq!(long_127.len(), 127);
        assert_eq!(long_128.len(), 128);
        // And where the real encoder REFUSES, the twin reports no length at all
        // rather than a length nothing would ever write.
        let past_bound = "0.".to_string() + &"1".repeat(MAX_STRING_BYTES);
        assert_eq!(encoded_number_len(&past_bound), None);
        assert!(encode_value(&CanonValue::Number(past_bound)).is_err());
    }

    /// One delta, one encoding — for numbers too. A `Replace` where a delta was
    /// available, and a delta that is not the one the encoder would have
    /// written, are both second spellings of one change and both refused.
    ///
    /// kills: removing the `number_delta_between` refusal from the `Replace`
    /// arm (the first assertion fails); removing the re-derivation check from
    /// the `NumberDelta` arm (the not-shorter delta applies and its assertion
    /// fails); accepting a number delta at the world root; widening the decoder
    /// fraction ceiling. The padded varint below is refused by `ByteReader`
    /// itself, not by anything this op added — it is here to pin that the new
    /// tag inherits that refusal rather than parsing its own varints.
    #[test]
    fn a_number_change_has_exactly_one_spelling() {
        let mut target = CanonValue::parse(br#"{"a":47.36551}"#).unwrap();
        let replace = Patch::Object {
            removals: Vec::new(),
            changes: vec![(
                "a".to_string(),
                Patch::Replace(CanonValue::Number("47.4".into())),
            )],
        };
        assert!(apply_patch(
            &mut target,
            &replace,
            &mut Shadow::default(),
            &mut ShadowBudget::default(),
            0
        )
        .unwrap_err()
        .message()
        .contains("shorter spelling"));

        // A delta the encoder would NOT have chosen because it is not shorter:
        // 1 -> 2 is one inline byte as a replacement and three as a delta, so
        // the delta spelling of it is a second encoding and must not decode.
        let not_shorter = Patch::Object {
            removals: Vec::new(),
            changes: vec![(
                "a".to_string(),
                Patch::NumberDelta {
                    fraction_digits: 0,
                    delta: 1,
                },
            )],
        };
        let mut target = CanonValue::parse(br#"{"a":1}"#).unwrap();
        assert!(apply_patch(
            &mut target,
            &not_shorter,
            &mut Shadow::default(),
            &mut ShadowBudget::default(),
            0
        )
        .unwrap_err()
        .message()
        .contains("not the encoder's spelling"));

        // A number delta at the world root, which is always an object.
        assert!(decode_patch_exact(&[PATCH_NUMBER_DELTA, 1, 2])
            .unwrap_err()
            .message()
            .contains("world root"));

        // A fraction width no number may carry.
        let mut absurd = vec![PATCH_OBJECT, 0, 1];
        put_text(&mut absurd, "a").unwrap();
        absurd.push(PATCH_NUMBER_DELTA);
        put_varint(&mut absurd, 31);
        put_varint(&mut absurd, 2);
        assert!(decode_patch_exact(&absurd)
            .unwrap_err()
            .message()
            .contains("more fraction digits"));

        // A padded varint is a second spelling of the same delta and the reader
        // must not admit it.
        let mut padded = vec![PATCH_OBJECT, 0, 1];
        put_text(&mut padded, "a").unwrap();
        padded.extend_from_slice(&[PATCH_NUMBER_DELTA, 0x81, 0x00, 2]);
        assert!(decode_patch_exact(&padded).is_err());
    }

    /// The edges of the op, each named by Codex round 1 as behaviour the first
    /// three tests left uncovered.
    ///
    /// kills: canonicalising a fraction width (the trailing-zero pair stops
    /// being expressible and its assertions fail); dropping the `i64` bound on
    /// the difference; widening the digit ceiling without widening the
    /// arithmetic (the rescale-overflow refusal turns into a panic under this
    /// crate's release overflow checks); accepting a number delta where a
    /// member is being ADDED rather than changed; accepting one as a revive's
    /// inner patch.
    #[test]
    fn the_edges_of_the_number_delta() {
        let number = |token: &str| CanonValue::Number(token.to_string());

        // A CHANGE OF FRACTION WIDTH ALONE is a real change of bytes, and it
        // goes both ways: the delta is zero and the width carries the content.
        assert_eq!(number_delta_between(&number("1.0"), &number("1.00")), Some((2, 0)));
        assert_eq!(number_delta_between(&number("1.00"), &number("1.0")), Some((1, 0)));
        // The same width with a zero difference is `Keep`'s job.
        assert_eq!(number_delta_between(&number("1.0"), &number("1.0")), None);

        // An old `-0.0` may still be joined: only the NEW token has to come
        // back, and it does.
        assert_eq!(number_delta_between(&number("-0.0"), &number("0.5")), Some((1, 5)));

        // The digit ceiling is admissible right up to its edge.
        let wide = "9".repeat(30);
        let wider = "9".repeat(29) + "8";
        assert_eq!(number_delta_between(&number(&wide), &number(&wider)), Some((0, -1)));
        assert_eq!(plain_decimal(&"9".repeat(31)), None);

        // Spellings `plain_decimal` must not admit at all.
        for token in ["-", "+1.0", "1.", ".5", "", "1.2.3", "0x1"] {
            assert_eq!(plain_decimal(token), None, "{token}");
        }
        // ...and one it admits arithmetically but the respell check refuses.
        assert_eq!(plain_decimal("01.0"), Some((10, 1)));
        assert_eq!(number_delta_between(&number("1.0"), &number("01.0")), None);

        // A HOSTILE WIDTH AGAINST A WIDE TARGET is refused, not panicked on,
        // even though this crate keeps overflow checks on in release.
        let mut target = number(&wide);
        assert!(apply_patch(
            &mut target,
            &Patch::NumberDelta { fraction_digits: 30, delta: 0 },
            &mut Shadow::default(),
            &mut ShadowBudget::default(),
            1
        )
        .unwrap_err()
        .message()
        .contains("overflows its target"));
        // And the widest zigzag varint decodes to `i64::MIN` rather than
        // wrapping on the way in.
        let mut widest = vec![PATCH_OBJECT, 0, 1];
        put_text(&mut widest, "a").unwrap();
        widest.push(PATCH_NUMBER_DELTA);
        put_varint(&mut widest, 0);
        put_varint(&mut widest, u64::MAX);
        let Patch::Object { changes, .. } = decode_patch_exact(&widest).unwrap() else {
            panic!("object patch");
        };
        assert!(matches!(
            changes[0].1,
            Patch::NumberDelta { fraction_digits: 0, delta: i64::MIN }
        ));

        // A NUMBER DELTA WHERE A MEMBER IS BEING ADDED has no old value to
        // stand on, and the object arm spells an addition as a bare replace.
        let mut absent = CanonValue::parse(br#"{"b":1}"#).unwrap();
        assert!(apply_patch(
            &mut absent,
            &Patch::Object {
                removals: Vec::new(),
                changes: vec![(
                    "a".to_string(),
                    Patch::NumberDelta { fraction_digits: 1, delta: 5 },
                )],
            },
            &mut Shadow::default(),
            &mut ShadowBudget::default(),
            0
        )
        .is_err());

        // A revive's inner patch is never a number delta: a revivable scalar
        // memory EQUALS the new value, which is `PATCH_REVIVE_SAME`.
        let mut revive = vec![PATCH_OBJECT, 0, 1];
        put_text(&mut revive, "a").unwrap();
        revive.push(PATCH_REVIVE_PATCH);
        revive.push(PATCH_NUMBER_DELTA);
        put_varint(&mut revive, 1);
        put_varint(&mut revive, 4);
        assert!(decode_patch_exact(&revive)
            .unwrap_err()
            .message()
            .contains("shorter spelling"));
    }

    /// One delta series, one set of cache bytes. `diff_value` never emits a
    /// patch that changes nothing, so the decoder must not accept one either —
    /// otherwise the same reconstructed hours could be carried by two caches
    /// with different lengths and different SHAs.
    #[test]
    fn redundant_patch_spellings_are_rejected() {
        // An object patch with no removals and no changes.
        let empty_object = [PATCH_OBJECT, 0, 0];
        assert!(decode_patch_exact(&empty_object)
            .unwrap_err()
            .message()
            .contains("touches no member"));

        // An array patch with no touched elements.
        assert!(decode_patch_exact(&[PATCH_ARRAY, 0])
            .unwrap_err()
            .message()
            .contains("touches no element"));

        // A keep nested inside an object patch, instead of omitting the member.
        let mut nested_keep = vec![PATCH_OBJECT, 0, 1];
        put_text(&mut nested_keep, "a").unwrap();
        nested_keep.push(PATCH_KEEP);
        assert!(decode_patch_exact(&nested_keep)
            .unwrap_err()
            .message()
            .contains("nested keep"));

        // A name that is both removed and changed, which is a replacement.
        let mut remove_and_change = vec![PATCH_OBJECT, 1];
        put_text(&mut remove_and_change, "a").unwrap();
        put_varint(&mut remove_and_change, 1);
        put_text(&mut remove_and_change, "a").unwrap();
        remove_and_change.push(PATCH_REPLACE);
        encode_value_into(&CanonValue::Null, &mut remove_and_change, 0).unwrap();
        assert!(decode_patch_exact(&remove_and_change)
            .unwrap_err()
            .message()
            .contains("removes and changes the same member name"));

        // A whole hour that changed nothing still keeps, at the top level.
        assert!(matches!(
            decode_patch_exact(&[PATCH_KEEP]).unwrap(),
            Patch::Keep
        ));
    }

    /// The rest of the aliasing closes at APPLY time, where the target is in
    /// hand: a replacement that restates a value the encoder would have diffed,
    /// a replacement that changes nothing, and a member name spelled some other
    /// decodable way than the target spells it. Each is a second encoding of a
    /// delta the encoder emits exactly one way.
    #[test]
    fn redundant_patch_spellings_are_rejected_against_their_target() {
        let object = |body: &str| CanonValue::parse(body.as_bytes()).unwrap();
        let replace = |body: &str| Patch::Replace(object(body));

        let mut target = object(r#"{"a":1}"#);
        assert!(apply_patch(&mut target, &replace(r#"{"a":2}"#), &mut Shadow::default(), &mut ShadowBudget::default(), 0)
            .unwrap_err()
            .message()
            .contains("would have diffed"));
        assert!(apply_patch(&mut target, &replace(r#"{"a":1}"#), &mut Shadow::default(), &mut ShadowBudget::default(), 0)
            .unwrap_err()
            .message()
            .contains("does not change its target"));

        let mut array = object(r#"{"a":[1,2]}"#);
        let same_length = Patch::Object {
            removals: Vec::new(),
            changes: vec![("a".to_string(), replace("[3,4]"))],
        };
        assert!(apply_patch(&mut array, &same_length, &mut Shadow::default(), &mut ShadowBudget::default(), 0)
            .unwrap_err()
            .message()
            .contains("would have diffed"));
        // A DIFFERENT length is exactly when the encoder does replace, so it
        // must still apply.
        let different_length = Patch::Object {
            removals: Vec::new(),
            changes: vec![("a".to_string(), replace("[3]"))],
        };
        apply_patch(&mut array, &different_length, &mut Shadow::default(), &mut ShadowBudget::default(), 0).unwrap();
        assert_eq!(array.to_json_bytes(), br#"{"a":[3]}"#);

        // `a` decodes to `a`, so it names the same member — and is not how
        // the target spells it.
        let alias = "\\u0061".to_string();
        let mut removal_target = object(r#"{"a":1,"b":2}"#);
        let aliased_removal = Patch::Object {
            removals: vec![alias.clone()],
            changes: vec![("b".to_string(), Patch::Replace(CanonValue::Null))],
        };
        assert!(apply_patch(&mut removal_target, &aliased_removal, &mut Shadow::default(), &mut ShadowBudget::default(), 0)
            .unwrap_err()
            .message()
            .contains("removal spells its member name differently"));
        // A refused patch leaves its target half-applied; every caller discards
        // the value on error, and this test has to as well.
        let mut removal_target = object(r#"{"a":1,"b":2}"#);
        let aliased_change = Patch::Object {
            removals: Vec::new(),
            changes: vec![(alias, Patch::Replace(CanonValue::Null))],
        };
        assert!(apply_patch(&mut removal_target, &aliased_change, &mut Shadow::default(), &mut ShadowBudget::default(), 0)
            .unwrap_err()
            .message()
            .contains("change spells its member name differently"));
    }

    /// Corruption of either artifact fails closed, each at the resolution it
    /// can be attributed to: a damaged INDEX is refused outright (nothing else
    /// can be believed once the authority itself moved), a damaged CACHE RECORD
    /// costs the hours that read it, and an index whose rows do not reach the
    /// end of the source cannot even be written.
    ///
    /// kills: dropping the `cache_record_sha256` comparison from `CacheAudit`.
    /// Note what that costs, precisely (Codex review round 4, finding 2): NOT a
    /// served bad hour — the whole-cache digest would still disagree while the
    /// record verdict said "intact", and `admit_cache`'s `(false, true)` arm
    /// refuses that as an account that does not add up. What it costs is the
    /// CONFINEMENT: the `open` below stops returning a reader at all, and one
    /// flipped byte is a whole-artifact refusal again, which is GY-2 restored.
    /// Also kills: dropping the tiling rule's `next_offset != source_len` arm
    /// (an index that accounts for only part of the source becomes encodable
    /// again).
    #[test]
    fn cache_and_index_corruption_fail_closed() {
        let hours: Vec<(i64, &str)> = vec![(0, r#"{"x":1}"#), (1, r#"{"x":2}"#)];
        let (cache, index, source_sha) = build(2, &hours);
        let mut index_bytes = index.to_bytes().unwrap();
        index_bytes[0] ^= 0x40;
        assert!(ReplayHourIndex::from_bytes(&index_bytes).is_err());

        // A flipped cache byte fails closed AT HOUR RESOLUTION since the
        // per-record digests (owner ruling 2026-08-12, task #191): the reader
        // admits the artifact and refuses, by name, exactly the hours that
        // replay through the record the byte landed in. The last byte is in the
        // last record, and with a checkpoint every two hours that is hour 1's
        // delta — so hour 0 still serves and hour 1 does not.
        let index_bytes = index.to_bytes().unwrap();
        let mut corrupt_cache = cache;
        let last = corrupt_cache.len() - 1;
        corrupt_cache[last] ^= 1;
        let reader =
            ReplayCacheReader::open(&index_bytes, &corrupt_cache, 200, &source_sha).unwrap();
        assert_eq!(
            reader
                .confined_hours()
                .iter()
                .map(ConfinedHour::hour_index)
                .collect::<Vec<i64>>(),
            vec![1]
        );
        reader
            .reconstruct_json(0)
            .expect("an hour whose window the damage never reaches still serves");
        let refused = reader.reconstruct_json(1).unwrap_err();
        assert!(refused.message().contains("game hour 1"), "{refused}");
        let total = reader.verify_every_hour().expect_err(
            "a proof over EVERY indexed hour is not available once one is confined",
        );
        assert!(
            total.message().contains("cannot serve 1 of the hours it indexes")
                && total.message().contains("game hour 1"),
            "the total proof must refuse by naming what cannot be served: {total}"
        );

        let mut incomplete_source_index = index;
        incomplete_source_index
            .hours
            .last_mut()
            .unwrap()
            .h4cr_end_offset = 199;
        assert!(
            incomplete_source_index.to_bytes().is_err(),
            "the hourly offset index must cover the canonical H4CR source through EOF"
        );
    }

    /// One value, one encoding. A decoder that accepted a second spelling would
    /// let two different caches carry the same content under different SHAs.
    #[test]
    fn non_canonical_binary_spellings_are_rejected() {
        // 5 encoded the long way instead of inline.
        let long_form = [TAG_UINT, 5];
        assert!(decode_value_exact(&long_form)
            .unwrap_err()
            .message()
            .contains("must use the inline encoding"));

        // 200 as a token instead of TAG_UINT.
        let mut token_form = vec![TAG_NUMBER_TOKEN];
        put_text(&mut token_form, "200").unwrap();
        assert!(decode_value_exact(&token_form)
            .unwrap_err()
            .message()
            .contains("must use the compact integer encoding"));

        // A non-minimal varint length prefix.
        let non_minimal = [TAG_STRING, 0x80, 0x00];
        assert!(decode_value_exact(&non_minimal)
            .unwrap_err()
            .message()
            .contains("not minimal"));

        // `-0` has no negative encoding.
        assert!(decode_value_exact(&[TAG_NEGINT, 0])
            .unwrap_err()
            .message()
            .contains("must not spell zero"));

        // Arbitrary bytes cannot ride in as a "number".
        let mut junk = vec![TAG_NUMBER_TOKEN];
        put_text(&mut junk, "1,\"x\":2").unwrap();
        assert!(decode_value_exact(&junk)
            .unwrap_err()
            .message()
            .contains("is not a JSON number"));

        // Nor as a string body.
        let mut bad_string = vec![TAG_STRING];
        put_text(&mut bad_string, "a\"b").unwrap();
        assert!(decode_value_exact(&bad_string)
            .unwrap_err()
            .message()
            .contains("is not a JSON string body"));
    }

    #[test]
    fn every_encodable_integer_shape_round_trips_through_the_binary_form() {
        for token in [
            "0",
            "1",
            "127",
            "128",
            "255",
            "65535",
            "9223372036854775807",
            "18446744073709551615",
            "-1",
            "-9223372036854775808",
            "-18446744073709551615",
            "18446744073709551616",
            "-18446744073709551616",
            "-0",
            "1.0",
            "1e3",
        ] {
            let value = CanonValue::Number(token.to_string());
            let encoded = encode_value(&value).unwrap();
            assert_eq!(
                decode_value_exact(&encoded).unwrap(),
                value,
                "token {token} did not survive the binary round trip"
            );
        }
        // The compact forms really are compact.
        assert_eq!(encode_value(&CanonValue::Number("0".into())).unwrap().len(), 1);
        assert_eq!(encode_value(&CanonValue::Number("127".into())).unwrap().len(), 1);
        assert_eq!(encode_value(&CanonValue::Number("128".into())).unwrap().len(), 3);
    }

    /// The linear whole-artifact pass and a per-hour seek sweep must certify
    /// exactly the same thing: every hour's bytes, and the deepest seek.
    #[test]
    fn whole_artifact_verification_agrees_with_a_per_hour_seek_sweep() {
        let hours: Vec<(i64, String)> = (0..40)
            .map(|hour| {
                (
                    hour,
                    format!(r#"{{"churn":{hour},"stable":"x","wide":[1,2,{hour}]}}"#),
                )
            })
            .collect();
        let borrowed: Vec<(i64, &str)> = hours
            .iter()
            .map(|(hour, body)| (*hour, body.as_str()))
            .collect();
        let (cache, index, source_sha) = build(7, &borrowed);
        let index_bytes = index.to_bytes().unwrap();
        let reader = ReplayCacheReader::open(&index_bytes, &cache, 4000, &source_sha).unwrap();

        let mut swept = 0u32;
        for (hour, body) in &borrowed {
            let (json, receipt) = reader.reconstruct_json(*hour).unwrap();
            assert_eq!(json, world_json(*hour, body));
            swept = swept.max(receipt.applied_deltas);
        }
        assert_eq!(reader.verify_every_hour().unwrap(), swept);
        assert_eq!(swept, 6, "the fixture must actually reach the window's far end");

        // The linear pass resets its running value at every checkpoint record,
        // so it is only equivalent to the sweep if a delta cannot name a
        // checkpoint that a LATER checkpoint has already superseded. Such an
        // index is self-consistent by distance and unseekable in fact, and
        // nothing but this rule notices.
        let (_, mut stale, _) = build(
            2,
            &[
                (0, r#"{"v":0}"#),
                (1, r#"{"v":1}"#),
                (2, r#"{"v":2}"#),
                (3, r#"{"v":3}"#),
            ],
        );
        assert_eq!(stale.hours[2].delta_steps, 0, "position 2 is a checkpoint");
        stale.checkpoint_interval_hours = 4;
        stale.hours[3].checkpoint_position = 0;
        stale.hours[3].delta_steps = 3;
        let error = stale.to_bytes().unwrap_err();
        assert!(
            error.message().contains("with checkpoint 2 between them"),
            "{error}"
        );

        // And it fails on the same corruption a seek would: one flipped payload
        // byte inside a delta record, in an index REBOUND to the corrupt cache
        // so the admission gate has nothing to object to. Both of the index's
        // accounts of the cache have to be rebound — the whole-artifact digest
        // and that record's own — because an index that carried one of each
        // would be refused as self-contradictory before any hour was decoded,
        // which is a different (and weaker) statement than the one this test
        // makes: the CONTENT is wrong, and only replaying it can tell.
        let mut corrupt = cache.clone();
        let delta = index.hours()[3].cache_record_offset() as usize + 21;
        corrupt[delta] ^= 0x20;
        let mut rebound = index.clone();
        rebound.cache_sha256 = to_hex(&Sha256::digest(&corrupt));
        let window = index.hours()[3].cache_record_offset() as usize
            ..(index.hours()[3].cache_record_offset() + index.hours()[3].cache_record_len())
                as usize;
        rebound.hours[3].cache_record_sha256 = to_hex(&Sha256::digest(&corrupt[window]));
        let rebound_bytes = rebound.to_bytes().unwrap();
        let reader =
            ReplayCacheReader::open(&rebound_bytes, &corrupt, 4000, &source_sha).unwrap();
        assert!(reader.confined_hours().is_empty(), "nothing is confined here");
        assert!(reader.verify_every_hour().is_err());
    }

    /// The delta encoding must not cost more than the compact JSON it replaces.
    /// The old format spent a fixed 4-byte length prefix on every number token
    /// and every collection count, which made a per-hour patch LARGER than
    /// simply writing the changed JSON out again.
    #[test]
    fn a_delta_is_smaller_than_the_compact_json_of_the_same_change() {
        let mut previous = serde_json::Map::new();
        let mut next = serde_json::Map::new();
        for country in 0..64u32 {
            let key = format!("c{country:03}");
            previous.insert(
                key.clone(),
                serde_json::json!({"fuel": 1000 + country, "manpower": 50000, "name": "x"}),
            );
            next.insert(
                key,
                serde_json::json!({"fuel": 1001 + country, "manpower": 50000, "name": "x"}),
            );
        }
        let previous_body = serde_json::to_string(&Value::Object(previous)).unwrap();
        let next_body = serde_json::to_string(&Value::Object(next)).unwrap();
        let hours: Vec<(i64, &str)> =
            vec![(0, previous_body.as_str()), (1, next_body.as_str())];
        let (cache, index, _) = build(2, &hours);
        let delta = index.hours()[1].cache_record_len() as usize - 21;

        // What the same change costs as JSON: the changed members, spelled out.
        let json_equivalent: usize = (0..64u32)
            .map(|country| format!(r#"{{"c{country:03}":{{"fuel":{}}}}}"#, 1001 + country).len())
            .sum();
        assert!(
            delta < json_equivalent,
            "delta record is {delta} bytes for a change worth {json_equivalent} bytes of JSON"
        );
        assert!(cache.len() > delta, "sanity: the checkpoint dominates");
    }

    // -----------------------------------------------------------------------
    // Lane GZ — interior holes (owner ruling 2026-08-12, task #189)
    // -----------------------------------------------------------------------

    // Four world bodies that differ, so a delta record is not empty and a
    // reconstruction proof is not vacuous.
    const OBJ_A: &str = r#"{"n":1}"#;
    const OBJ_B: &str = r#"{"n":2}"#;
    const OBJ_C: &str = r#"{"n":3}"#;
    const OBJ_D: &str = r#"{"n":4}"#;

    /// Build a cache with one hour SKIPPED BY NAME — the shape a recording with
    /// an injured hour now has.
    fn build_with_hole(
        interval: u32,
        hole_hour: i64,
        hours: &[(i64, &str)],
    ) -> (Vec<u8>, ReplayHourIndex, String) {
        let source_sha = "ab".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, interval).unwrap();
        let mut last = 0i64;
        for (hour, body) in hours {
            if *hour == hole_hour {
                encoder
                    .exclude_hour(ReplayExcludedHour::new(
                        *hour,
                        (*hour as u64) * 100,
                        (*hour as u64 + 1) * 100,
                        "sequence discontinuity at byte 421",
                    ))
                    .unwrap();
            } else {
                encoder.push(world(*hour, body)).unwrap();
            }
            last = *hour;
        }
        let source_len = (last as u64 + 1) * 100;
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::IdentityV1, source_len)
            .unwrap();
        (cache, index, source_sha)
    }

    /// kills: dropping the excluded-hour section from `to_bytes`/`from_bytes`,
    /// or letting `validate` accept an index whose served rows leave a gap no
    /// hole row claims. Either turns a recording that names its injured hour
    /// into one that silently stretches a good hour's byte range over the
    /// injured hour's records — hour misattribution, the owner's stop
    /// condition.
    #[test]
    fn an_interior_hole_is_carried_by_the_format_and_an_unnamed_gap_is_not() {
        let (cache, index, sha) =
            build_with_hole(168, 1, &[(0, OBJ_A), (1, OBJ_B), (2, OBJ_C), (3, OBJ_D)]);
        assert_eq!(
            index
                .hours()
                .iter()
                .map(ReplayHourEntry::hour_index)
                .collect::<Vec<_>>(),
            vec![0, 2, 3],
            "the excluded hour must not be served"
        );
        assert_eq!(index.excluded_hours().len(), 1);
        assert_eq!(index.excluded_hours()[0].hour_index(), 1);
        assert_eq!(index.entry(1), None, "an excluded hour is not seekable");
        assert_eq!(
            index.exclusion_reason(1),
            Some("sequence discontinuity at byte 421"),
            "the refusal is BY NAME"
        );

        // The whole point: the artifact still serves everything it vouches for.
        let bytes = index.to_bytes().unwrap();
        let reader = ReplayCacheReader::open(&bytes, &cache, index.source_len(), &sha)
            .expect("a holed index still binds to its cache");
        reader
            .verify_every_hour()
            .expect("every served hour reconstructs");
        assert_eq!(
            ReplayHourIndex::from_bytes(&bytes).unwrap(),
            index,
            "the hole survives the binary encoding"
        );

        // Take the hole row away and leave the served rows as they were: the
        // bytes hour 1 occupied are then claimed by nobody.
        let mut unnamed = index;
        unnamed.excluded.clear();
        let error = unnamed.validate().expect_err("an unclaimed gap must refuse");
        assert!(
            error.message().contains("account for the source"),
            "unexpected: {error}"
        );
    }

    /// kills: removing the tie arm from the merge in `validate_h4cr_tiling`.
    ///
    /// Stated precisely, because the obvious stronger claim would be wrong: an
    /// index that both serves and refuses one hour is refused EITHER WAY, since
    /// the general merged-order check downstream sees the hour twice. What the
    /// tie arm buys is the DIAGNOSIS — "hour index both serves and excludes
    /// game hour N" instead of "not in strictly increasing hour order" — and
    /// that is what this test pins, by asserting the message rather than only
    /// the refusal. A producer that has published a contradictory index needs
    /// to be told which contradiction it published (Codex review round 3,
    /// finding 2).
    #[test]
    fn an_hour_cannot_be_both_served_and_excluded() {
        let (_, index, _) = build(168, &[(0, OBJ_A), (1, OBJ_B)]);
        let mut forged = index;
        forged.excluded.push(ReplayExcludedHour::new(
            1,
            100,
            200,
            "claimed injured while also being served",
        ));
        let error = forged
            .validate()
            .expect_err("a doubly-claimed hour must refuse");
        assert!(
            error.message().contains("both serves and excludes"),
            "unexpected: {error}"
        );
    }

    /// kills: dropping the empty-reason refusal from `exclude_hour` or from
    /// `validate_h4cr_tiling`. An hour skipped with nothing said about why is
    /// silent loss wearing the exclusion label — exactly the guessing the
    /// per-hour ruling forbids.
    #[test]
    fn a_hole_is_never_admitted_without_a_named_reason() {
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &"ab".repeat(32), 168).unwrap();
        encoder.push(world(0, OBJ_A)).unwrap();
        let error = encoder
            .exclude_hour(ReplayExcludedHour::new(1, 100, 200, "   "))
            .expect_err("an unnamed skip must refuse at the encoder");
        assert!(
            error.message().contains("without a named reason"),
            "unexpected: {error}"
        );

        // ...and a hand-built index carrying one is refused by the READER too,
        // so the encoder is not the only thing holding the line.
        let (_, index, _) = build(168, &[(0, OBJ_A), (1, OBJ_B)]);
        let mut forged = index;
        forged
            .excluded
            .push(ReplayExcludedHour::new(2, 200, 200, ""));
        assert!(forged
            .validate()
            .expect_err("an unnamed hole must refuse")
            .message()
            .contains("names no reason"));
    }

    /// kills: giving `push` and `exclude_hour` separate placement cursors, or
    /// restoring `check_ordered_and_contiguous`'s old "compare against the last
    /// SERVED row" rule. Either lets a caller skip an hour WITHOUT declaring
    /// it — the misattribution the hole rows exist to prevent — and it would
    /// pass silently, because the resulting index is internally consistent.
    #[test]
    fn the_encoder_refuses_a_skip_that_was_never_declared() {
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &"ab".repeat(32), 168).unwrap();
        encoder.push(world(0, OBJ_A)).unwrap();
        let error = encoder
            .push(world(2, OBJ_C))
            .expect_err("jumping over hour 1 without declaring it must refuse");
        assert!(
            error.message().contains("ordered and contiguous"),
            "unexpected: {error}"
        );

        // Declaring it is the whole difference.
        encoder
            .exclude_hour(ReplayExcludedHour::new(1, 100, 200, "records never arrived"))
            .expect("a declared skip is admitted");
        encoder
            .push(world(2, OBJ_C))
            .expect("and the next hour then fits");
    }

    /// kills: restoring the old "first served row at offset zero, last served
    /// row at `source_len`" pair as the only placement law. Those are the two
    /// positions an injury actually lands on in production — a crashed recorder
    /// tears the tail, and a stream whose opening hour never arrived starts
    /// late — and both were whole-recording refusals.
    #[test]
    fn a_leading_hole_and_a_trailing_hole_both_still_tile_the_source() {
        // Leading: the first SERVED hour no longer starts at offset zero.
        let (_, leading, _) = build_with_hole(168, 0, &[(0, OBJ_A), (1, OBJ_B), (2, OBJ_C)]);
        assert_eq!(leading.hours()[0].h4cr_start_offset(), 100);
        assert_eq!(leading.excluded_hours()[0].h4cr_start_offset(), 0);
        leading.validate().expect("a leading hole tiles the source");

        // Trailing: the last SERVED hour no longer reaches the source end.
        let (_, trailing, _) = build_with_hole(168, 2, &[(0, OBJ_A), (1, OBJ_B), (2, OBJ_C)]);
        assert_eq!(trailing.hours().last().unwrap().h4cr_end_offset(), 200);
        assert_eq!(trailing.excluded_hours()[0].h4cr_end_offset(), 300);
        trailing.validate().expect("a trailing hole tiles the source");

        // Slide a hole off its place and the account stops adding up.
        let mut slipped = trailing;
        slipped.excluded[0].h4cr_start_offset = 250;
        assert!(slipped
            .validate()
            .expect_err("a hole that does not abut its neighbour must refuse")
            .message()
            .contains("account for the source"));
    }

    /// kills: decoding a hole's reason through the GENERIC `index_string`
    /// instead of `index_string_bounded(MAX_EXCLUSION_REASON_BYTES)`. The
    /// structural `validate` check would still refuse the index — but only
    /// after every declared reason has been copied out of the input, so an
    /// authenticated H4IX could make a replay consumer materialize hundreds of
    /// megabytes before producing the refusal the bound exists to produce. The
    /// refusal has to happen at the LENGTH, not at the value.
    #[test]
    fn an_oversized_reason_is_refused_at_its_length_prefix_not_after_the_copy() {
        let (_, index, _) = build(168, &[(0, OBJ_A), (1, OBJ_B)]);
        let mut bytes = index.to_bytes().unwrap();
        // The encoded excluded-hour section is the tail: a zero count. Replace
        // it with one row whose reason DECLARES a length no honest index has,
        // and supply none of those bytes. A reader that trusts the declared
        // length would try to take them; a reader that bounds first refuses
        // without reading past the prefix.
        assert_eq!(
            &bytes[bytes.len() - 8..],
            &0u64.to_le_bytes(),
            "an uninjured index ends with a zero excluded-hour count"
        );
        bytes.truncate(bytes.len() - 8);
        bytes.extend_from_slice(&1u64.to_le_bytes()); // one excluded row
        bytes.extend_from_slice(&2i64.to_le_bytes()); // hour_index
        bytes.extend_from_slice(&200u64.to_le_bytes()); // start
        bytes.extend_from_slice(&200u64.to_le_bytes()); // end
        bytes.extend_from_slice(&(64u32 * 1024 * 1024).to_le_bytes()); // 64 MiB reason
        let error = ReplayHourIndex::from_bytes(&bytes)
            .expect_err("a 64 MiB exclusion reason must be refused");
        assert!(
            error.message().contains("above the")
                && error.message().contains(&MAX_EXCLUSION_REASON_BYTES.to_string()),
            "the refusal must name the reason bound rather than the generic string bound or a \
             short-input error: {error}"
        );
    }

    /// kills: removing `MAX_EXCLUSION_REASON_BYTES` from either the encoder or
    /// the reader's structural validation. A reason is prose naming one hour;
    /// without a bound of its own an index could carry two million rows of
    /// quarter-gigabyte strings under the generic string bound.
    #[test]
    fn an_exclusion_reason_is_bounded_at_both_ends() {
        let huge = "x".repeat(MAX_EXCLUSION_REASON_BYTES + 1);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &"ab".repeat(32), 168).unwrap();
        encoder.push(world(0, OBJ_A)).unwrap();
        assert!(encoder
            .exclude_hour(ReplayExcludedHour::new(1, 100, 200, huge.clone()))
            .expect_err("an oversized reason must refuse at the encoder")
            .message()
            .contains("above the"));

        let (_, index, _) = build(168, &[(0, OBJ_A), (1, OBJ_B)]);
        let mut forged = index;
        forged
            .excluded
            .push(ReplayExcludedHour::new(2, 200, 200, huge));
        assert!(forged
            .validate()
            .expect_err("an oversized reason must refuse at the reader")
            .message()
            .contains("above the"));
    }
}
