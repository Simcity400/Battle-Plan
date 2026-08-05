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
//! [`ReplayCacheEncoder::push`] does not assume the representation is lossless:
//! it re-serializes each parse and requires the original bytes before the hour
//! is admitted, so a token shape nobody anticipated fails the build loudly
//! instead of corrupting a seek quietly.

use crate::canon_value::{
    is_json_number_token, is_json_string_body, is_valid_object_key, key_cmp, CanonValue,
    MAX_RECURSION_DEPTH,
};
use crate::sha256::{to_hex, Sha256};
use std::cmp::Ordering;
use std::io::Write;
use std::sync::Arc;

pub const HOUR_INDEX_ROLE: &str = "replay_decoder_hour_index";
pub const CHECKPOINT_CACHE_ROLE: &str = "replay_decoder_checkpoint_cache";
pub const HOUR_INDEX_FILE: &str = "replay_decoder/hour_index.h4ix";
pub const CHECKPOINT_CACHE_FILE: &str = "replay_decoder/checkpoints.h4cp";
pub const DEFAULT_CHECKPOINT_INTERVAL_HOURS: u32 = 168;
pub const MAX_CHECKPOINT_INTERVAL_HOURS: u32 = 24 * 31;

const INDEX_MAGIC: &[u8; 4] = b"H4IX";
const CACHE_MAGIC: &[u8; 4] = b"H4CP";
/// Version 2 carries scalar SPELLINGS and encodes lengths, counts and small
/// integers compactly. Version 1 stored `serde_json`-normalised numbers behind
/// fixed 32-bit length prefixes; it is not readable here and never will be,
/// because its records cannot answer what the fold actually wrote.
const FORMAT_VERSION: u16 = 2;
const RECORD_CHECKPOINT: u8 = 1;
const RECORD_DELTA: u8 = 2;
const CACHE_HEADER_LEN: u64 = 4 + 2 + 2 + 32;
const MAX_STRING_BYTES: usize = 256 * 1024 * 1024;
const MAX_HOURS: usize = 2_000_000;

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
            put_index_string(&mut out, &hour.date)?;
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
        let mut hours = Vec::with_capacity(hour_count);
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
                date: reader.index_string()?,
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
        let mut previous_h4cr_end = None;
        let mut previous_cache_end = CACHE_HEADER_LEN;
        let mut active_checkpoint = usize::MAX;
        for (position, hour) in self.hours.iter().enumerate() {
            decode_hex32(&hour.world_json_sha256)?;
            if hour.date.trim().is_empty()
                || previous_hour.is_some_and(|value| hour.hour_index <= value)
                || hour.h4cr_start_offset >= hour.h4cr_end_offset
                || hour.h4cr_end_offset > self.source_len
                || previous_h4cr_end.is_some_and(|value| hour.h4cr_start_offset != value)
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
            previous_h4cr_end = Some(hour.h4cr_end_offset);
            previous_cache_end = hour.cache_record_offset + hour.cache_record_len;
        }
        if self.hours[0].h4cr_start_offset != 0
            || previous_h4cr_end != Some(self.source_len)
            || previous_cache_end != self.cache_len
        {
            return Err(ReplayCacheError::new(
                "hour index does not cover the cache/source from its first byte through its final record",
            ));
        }
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

pub struct ReplayCacheEncoder<W: Write> {
    writer: W,
    hasher: Sha256,
    offset: u64,
    source_sha256: String,
    checkpoint_interval_hours: u32,
    previous: Option<CanonValue>,
    entries: Vec<ReplayHourEntry>,
    checkpoint_position: usize,
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
            writer,
            hasher: Sha256::new(),
            offset: 0,
            source_sha256,
            checkpoint_interval_hours,
            previous: None,
            entries: Vec::new(),
            checkpoint_position: 0,
        };
        encoder.write_hashed(CACHE_MAGIC)?;
        encoder.write_hashed(&FORMAT_VERSION.to_le_bytes())?;
        encoder.write_hashed(&0u16.to_le_bytes())?;
        encoder.write_hashed(&source_digest)?;
        Ok(encoder)
    }

    pub fn push(&mut self, hour: CacheBuildHour) -> Result<(), ReplayCacheError> {
        if hour.date.trim().is_empty()
            || hour.h4cr_start_offset >= hour.h4cr_end_offset
            || self.entries.last().is_some_and(|previous| {
                hour.hour_index <= previous.hour_index
                    || hour.h4cr_start_offset != previous.h4cr_end_offset
            })
            || (self.entries.is_empty() && hour.h4cr_start_offset != 0)
        {
            return Err(ReplayCacheError::new(
                "cache input hours are not strictly ordered and contiguous",
            ));
        }
        let value = CanonValue::parse(&hour.world_json).map_err(|error| {
            ReplayCacheError::new(format!(
                "hour {} is not strict canonical world JSON: {error}",
                hour.hour_index
            ))
        })?;
        // The representation claims to carry every spelling the fold emitted.
        // Prove it on THESE bytes rather than trusting the claim: if some token
        // shape survives parsing but not serialization, this is where the build
        // stops, with the byte offset, instead of a seek returning a document
        // that merely looks right.
        let round_trip = value.to_json_bytes();
        if round_trip != hour.world_json {
            let divergence = round_trip
                .iter()
                .zip(&hour.world_json)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| round_trip.len().min(hour.world_json.len()));
            return Err(ReplayCacheError::new(format!(
                "hour {} canonical JSON is not preserved by the cache value model: \
                 first divergence at byte {divergence} ({} source bytes, {} re-emitted)",
                hour.hour_index,
                hour.world_json.len(),
                round_trip.len()
            )));
        }
        validate_world_identity(&value, hour.hour_index, &hour.date)?;
        let position = self.entries.len();
        let is_checkpoint = position % self.checkpoint_interval_hours as usize == 0;
        let (record_kind, payload, delta_steps) = if is_checkpoint {
            self.checkpoint_position = position;
            (RECORD_CHECKPOINT, encode_value(&value)?, 0)
        } else {
            let previous = self
                .previous
                .as_ref()
                .expect("a non-checkpoint cache record has a previous hour");
            let patch = diff_value(previous, &value, 0)?;
            let payload = encode_patch(&patch)?;
            let mut reconstructed = previous.clone();
            apply_patch(&mut reconstructed, &patch, 0)?;
            if reconstructed != value {
                return Err(ReplayCacheError::new(format!(
                    "binary delta parity failed while encoding hour {}",
                    hour.hour_index
                )));
            }
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
            world_json_sha256: Sha256::hex(&hour.world_json),
        });
        self.previous = Some(value);
        Ok(())
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

pub struct ReplayCacheReader<'a> {
    index: ReplayHourIndex,
    cache: &'a [u8],
}

/// Owned, once-authenticated cache suitable for repeated product seeks.
///
/// Construction hashes and binds the cache exactly once. Individual seeks
/// still validate record structure, target identity, parity SHA, and the
/// configured delta bound.
#[derive(Clone)]
pub struct CertifiedReplayCache {
    index: ReplayHourIndex,
    cache: Arc<[u8]>,
}

impl CertifiedReplayCache {
    pub fn open(
        index_bytes: &[u8],
        cache: Vec<u8>,
        expected_source_len: u64,
        expected_source_sha256: &str,
    ) -> Result<Self, ReplayCacheError> {
        let reader = ReplayCacheReader::open(
            index_bytes,
            &cache,
            expected_source_len,
            expected_source_sha256,
        )?;
        let index = reader.index.clone();
        drop(reader);
        Ok(Self {
            index,
            cache: Arc::from(cache),
        })
    }

    pub fn index(&self) -> &ReplayHourIndex {
        &self.index
    }

    pub fn reconstruct_json(
        &self,
        hour_index: i64,
    ) -> Result<(Vec<u8>, ReplaySeekReceipt), ReplayCacheError> {
        reconstruct_json_from(&self.index, self.cache.as_ref(), hour_index)
    }
}

impl<'a> ReplayCacheReader<'a> {
    pub fn open(
        index_bytes: &[u8],
        cache: &'a [u8],
        expected_source_len: u64,
        expected_source_sha256: &str,
    ) -> Result<Self, ReplayCacheError> {
        let index = ReplayHourIndex::from_bytes(index_bytes)?;
        if index.source_len != expected_source_len
            || index.source_sha256 != expected_source_sha256
            || index.cache_len != cache.len() as u64
            || index.cache_sha256 != Sha256::hex(cache)
        {
            return Err(ReplayCacheError::new(
                "replay index/cache is not bound to the selected canonical H4CR source",
            ));
        }
        let mut header = ByteReader::new(cache);
        if header.take(4)? != CACHE_MAGIC
            || header.u16()? != FORMAT_VERSION
            || header.u16()? != 0
            || to_hex(header.array32()?) != expected_source_sha256
        {
            return Err(ReplayCacheError::new(
                "checkpoint cache header identity is invalid",
            ));
        }
        Ok(Self { index, cache })
    }

    pub fn index(&self) -> &ReplayHourIndex {
        &self.index
    }

    pub fn reconstruct_json(
        &self,
        hour_index: i64,
    ) -> Result<(Vec<u8>, ReplaySeekReceipt), ReplayCacheError> {
        reconstruct_json_from(&self.index, self.cache, hour_index)
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
        let mut value: Option<CanonValue> = None;
        let mut max_seek_deltas = 0u32;
        for (position, entry) in self.index.hours.iter().enumerate() {
            let (kind, record_hour, payload) = cache_record(self.cache, entry)?;
            if record_hour != entry.hour_index {
                return Err(ReplayCacheError::new(format!(
                    "cache record at position {position} is not the indexed hour {}",
                    entry.hour_index
                )));
            }
            match (kind, entry.delta_steps) {
                (RECORD_CHECKPOINT, 0) => value = Some(decode_value_exact(payload)?),
                (RECORD_DELTA, steps) if steps != 0 => {
                    let running = value.as_mut().ok_or_else(|| {
                        ReplayCacheError::new("a delta record precedes its checkpoint")
                    })?;
                    apply_patch(running, &decode_patch_exact(payload)?, 0)?;
                }
                _ => {
                    return Err(ReplayCacheError::new(format!(
                        "cache record kind at position {position} disagrees with its delta distance"
                    )))
                }
            }
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
        if max_seek_deltas >= self.index.checkpoint_interval_hours {
            return Err(ReplayCacheError::new(
                "reconstructed seek exceeded its certified checkpoint bound",
            ));
        }
        Ok(max_seek_deltas)
    }
}

fn reconstruct_json_from(
    index: &ReplayHourIndex,
    cache: &[u8],
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
    let mut value = decode_value_exact(payload)?;
    for entry in &index.hours[checkpoint_position + 1..=target_position] {
        let (kind, record_hour, payload) = cache_record(cache, entry)?;
        if kind != RECORD_DELTA || record_hour != entry.hour_index {
            return Err(ReplayCacheError::new(
                "delta index points at an invalid cache record",
            ));
        }
        let patch = decode_patch_exact(payload)?;
        apply_patch(&mut value, &patch, 0)?;
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

fn cache_record<'a>(
    cache: &'a [u8],
    entry: &ReplayHourEntry,
) -> Result<(u8, i64, &'a [u8]), ReplayCacheError> {
    let start = usize::try_from(entry.cache_record_offset)
        .map_err(|_| ReplayCacheError::new("cache record offset does not fit usize"))?;
    let len = usize::try_from(entry.cache_record_len)
        .map_err(|_| ReplayCacheError::new("cache record length does not fit usize"))?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= cache.len())
        .ok_or_else(|| ReplayCacheError::new("cache record range is out of bounds"))?;
    let mut reader = ByteReader::new(&cache[start..end]);
    let kind = reader.u8()?;
    if reader.take(3)? != [0, 0, 0] {
        return Err(ReplayCacheError::new(
            "cache record reserved bytes are nonzero",
        ));
    }
    let hour = reader.i64()?;
    let payload_len = usize::try_from(reader.u64()?)
        .map_err(|_| ReplayCacheError::new("cache payload length does not fit usize"))?;
    if reader.u8()? != 0 {
        return Err(ReplayCacheError::new(
            "cache record trailing reserved byte is nonzero",
        ));
    }
    let payload = reader.take(payload_len)?;
    reader.finish()?;
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
}

fn diff_value(old: &CanonValue, new: &CanonValue, depth: usize) -> Result<Patch, ReplayCacheError> {
    require_depth(depth)?;
    if old == new {
        return Ok(Patch::Keep);
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
                        let patch = diff_value(&old[left].1, &new[right].1, depth + 1)?;
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
                let patch = diff_value(old_value, new_value, depth + 1)?;
                if !matches!(patch, Patch::Keep) {
                    changes.push((
                        u32::try_from(index)
                            .map_err(|_| ReplayCacheError::new("array index does not fit u32"))?,
                        patch,
                    ));
                }
            }
            Ok(Patch::Array(changes))
        }
        _ => Ok(Patch::Replace(new.clone())),
    }
}

fn apply_patch(
    target: &mut CanonValue,
    patch: &Patch,
    depth: usize,
) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match patch {
        Patch::Keep => Ok(()),
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
            *target = value.clone();
            Ok(())
        }
        Patch::Object { removals, changes } => {
            let CanonValue::Object(members) = target else {
                return Err(ReplayCacheError::new(
                    "object patch targets a non-object value",
                ));
            };
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
                    apply_patch(&mut existing, change, depth + 1)?;
                    out.push((existing_key, existing));
                } else {
                    match change {
                        Patch::Replace(value) => out.push((key.clone(), value.clone())),
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
        Patch::Array(changes) => {
            let CanonValue::Array(array) = target else {
                return Err(ReplayCacheError::new(
                    "array patch targets a non-array value",
                ));
            };
            let mut previous = None;
            for (index, change) in changes {
                let index = *index as usize;
                if previous.is_some_and(|value| index <= value) || index >= array.len() {
                    return Err(ReplayCacheError::new(
                        "array patch indices are not strictly increasing and in bounds",
                    ));
                }
                apply_patch(&mut array[index], change, depth + 1)?;
                previous = Some(index);
            }
            Ok(())
        }
    }
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
        Patch::Replace(value) => {
            out.push(PATCH_REPLACE);
            encode_value_into(value, out, depth + 1)?;
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
        PATCH_OBJECT => {
            let removal_count = reader.count()?;
            let mut removals = Vec::with_capacity(removal_count.min(4096));
            for _ in 0..removal_count {
                removals.push(reader.text()?);
            }
            if !strictly_increasing_keys(&removals) {
                return Err(ReplayCacheError::new(
                    "object patch removals are not strictly ordered",
                ));
            }
            let change_count = reader.count()?;
            let mut changes = Vec::with_capacity(change_count.min(4096));
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
            let count = reader.count()?;
            let mut changes: Vec<(u32, Patch)> = Vec::with_capacity(count.min(4096));
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
            let count = reader.count()?;
            let mut values = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                values.push(decode_value(reader, depth + 1)?);
            }
            Ok(CanonValue::Array(values))
        }
        TAG_OBJECT => {
            let count = reader.count()?;
            let mut members: Vec<(String, CanonValue)> = Vec::with_capacity(count.min(4096));
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

    fn count(&mut self) -> Result<usize, ReplayCacheError> {
        let value = self.varint()?;
        // A collection cannot be longer than the bytes left to describe it.
        if value > (self.bytes.len() - self.position) as u64 {
            return Err(ReplayCacheError::new(
                "binary collection count exceeds the bytes that remain",
            ));
        }
        usize::try_from(value)
            .map_err(|_| ReplayCacheError::new("binary collection count does not fit usize"))
    }

    fn text(&mut self) -> Result<String, ReplayCacheError> {
        let len = usize::try_from(self.varint()?)
            .map_err(|_| ReplayCacheError::new("binary string length does not fit usize"))?;
        if len > MAX_STRING_BYTES {
            return Err(ReplayCacheError::new("binary string exceeds the size bound"));
        }
        String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| ReplayCacheError::new("binary replay string is not UTF-8"))
    }

    fn index_string(&mut self) -> Result<String, ReplayCacheError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING_BYTES {
            return Err(ReplayCacheError::new("binary string exceeds the size bound"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon_value::NON_IDENTITY_TOKENS;
    use serde_json::Value;

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
        let source_sha = "ab".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, interval).unwrap();
        for (hour, body) in hours {
            encoder.push(world(*hour, body)).unwrap();
        }
        let source_len = (hours.last().unwrap().0 as u64 + 1) * 100;
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::IdentityV1, source_len)
            .unwrap();
        (cache, index, source_sha)
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

    /// A world body the cache model cannot spell back is a BUILD failure with
    /// the byte offset, never a silently different seek.
    #[test]
    fn a_body_that_does_not_round_trip_is_refused_at_push_with_its_offset() {
        let source_sha = "cd".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 2).unwrap();
        let mut hour = world(0, r#"{"v":1}"#);
        // Whitespace is legal JSON and parses, but it is not what the value
        // model re-emits, so the source is not reproducible from the cache.
        hour.world_json = String::from_utf8(hour.world_json)
            .unwrap()
            .replace(r#""world":"#, r#""world": "#)
            .into_bytes();
        let error = encoder.push(hour).unwrap_err().to_string();
        assert!(
            error.contains("is not preserved by the cache value model")
                && error.contains("first divergence at byte"),
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
        assert!(apply_patch(&mut target, &replace(r#"{"a":2}"#), 0)
            .unwrap_err()
            .message()
            .contains("would have diffed"));
        assert!(apply_patch(&mut target, &replace(r#"{"a":1}"#), 0)
            .unwrap_err()
            .message()
            .contains("does not change its target"));

        let mut array = object(r#"{"a":[1,2]}"#);
        let same_length = Patch::Object {
            removals: Vec::new(),
            changes: vec![("a".to_string(), replace("[3,4]"))],
        };
        assert!(apply_patch(&mut array, &same_length, 0)
            .unwrap_err()
            .message()
            .contains("would have diffed"));
        // A DIFFERENT length is exactly when the encoder does replace, so it
        // must still apply.
        let different_length = Patch::Object {
            removals: Vec::new(),
            changes: vec![("a".to_string(), replace("[3]"))],
        };
        apply_patch(&mut array, &different_length, 0).unwrap();
        assert_eq!(array.to_json_bytes(), br#"{"a":[3]}"#);

        // `a` decodes to `a`, so it names the same member — and is not how
        // the target spells it.
        let alias = "\\u0061".to_string();
        let mut removal_target = object(r#"{"a":1,"b":2}"#);
        let aliased_removal = Patch::Object {
            removals: vec![alias.clone()],
            changes: vec![("b".to_string(), Patch::Replace(CanonValue::Null))],
        };
        assert!(apply_patch(&mut removal_target, &aliased_removal, 0)
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
        assert!(apply_patch(&mut removal_target, &aliased_change, 0)
            .unwrap_err()
            .message()
            .contains("change spells its member name differently"));
    }

    #[test]
    fn cache_and_index_corruption_fail_closed() {
        let hours: Vec<(i64, &str)> = vec![(0, r#"{"x":1}"#), (1, r#"{"x":2}"#)];
        let (cache, index, source_sha) = build(2, &hours);
        let mut index_bytes = index.to_bytes().unwrap();
        index_bytes[0] ^= 0x40;
        assert!(ReplayHourIndex::from_bytes(&index_bytes).is_err());

        let index_bytes = index.to_bytes().unwrap();
        let mut corrupt_cache = cache;
        let last = corrupt_cache.len() - 1;
        corrupt_cache[last] ^= 1;
        assert!(ReplayCacheReader::open(&index_bytes, &corrupt_cache, 200, &source_sha).is_err());

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
        // byte inside a delta record.
        let mut corrupt = cache.clone();
        let delta = index.hours()[3].cache_record_offset() as usize + 21;
        corrupt[delta] ^= 0x20;
        let mut rebound = index.clone();
        rebound.cache_sha256 = to_hex(&Sha256::digest(&corrupt));
        let rebound_bytes = rebound.to_bytes().unwrap();
        let reader =
            ReplayCacheReader::open(&rebound_bytes, &corrupt, 4000, &source_sha).unwrap();
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
}
