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

use crate::sha256::{to_hex, Sha256};
use serde_json::{Map, Number, Value};
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
const FORMAT_VERSION: u16 = 1;
const RECORD_CHECKPOINT: u8 = 1;
const RECORD_DELTA: u8 = 2;
const CACHE_HEADER_LEN: u64 = 4 + 2 + 2 + 32;
const MAX_RECURSION_DEPTH: usize = 256;
const MAX_STRING_BYTES: usize = 256 * 1024 * 1024;
const MAX_HOURS: usize = 2_000_000;

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
        put_string(&mut out, &self.game_id)?;
        put_string(&mut out, &self.schema_version)?;
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
            put_string(&mut out, &hour.date)?;
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
        let game_id = reader.string()?;
        let schema_version = reader.string()?;
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
                date: reader.string()?,
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
    previous: Option<Value>,
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
            || self
                .entries
                .last()
                .is_some_and(|previous| {
                    hour.hour_index <= previous.hour_index
                        || hour.h4cr_start_offset != previous.h4cr_end_offset
                })
            || (self.entries.is_empty() && hour.h4cr_start_offset != 0)
        {
            return Err(ReplayCacheError::new(
                "cache input hours are not strictly ordered and contiguous",
            ));
        }
        let value = crate::strict_json::from_slice(&hour.world_json).map_err(|error| {
            ReplayCacheError::new(format!(
                "hour {} is not strict canonical world JSON: {error}",
                hour.hour_index
            ))
        })?;
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
        let json = serde_json::to_vec(&value).map_err(|error| {
            ReplayCacheError::new(format!("serialize reconstructed world state: {error}"))
        })?;
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
    Replace(Value),
    Object {
        removals: Vec<String>,
        changes: Vec<(String, Patch)>,
    },
    Array(Vec<(u32, Patch)>),
}

fn diff_value(old: &Value, new: &Value, depth: usize) -> Result<Patch, ReplayCacheError> {
    require_depth(depth)?;
    if old == new {
        return Ok(Patch::Keep);
    }
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            let removals = old
                .keys()
                .filter(|key| !new.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            let mut changes = Vec::new();
            for (key, new_value) in new {
                let patch = match old.get(key) {
                    Some(old_value) => diff_value(old_value, new_value, depth + 1)?,
                    None => Patch::Replace(new_value.clone()),
                };
                if !matches!(patch, Patch::Keep) {
                    changes.push((key.clone(), patch));
                }
            }
            Ok(Patch::Object { removals, changes })
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
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

fn apply_patch(target: &mut Value, patch: &Patch, depth: usize) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match patch {
        Patch::Keep => Ok(()),
        Patch::Replace(value) => {
            *target = value.clone();
            Ok(())
        }
        Patch::Object { removals, changes } => {
            let object = target.as_object_mut().ok_or_else(|| {
                ReplayCacheError::new("object patch targets a non-object value")
            })?;
            for key in removals {
                if object.remove(key).is_none() {
                    return Err(ReplayCacheError::new(
                        "object patch removes an absent member",
                    ));
                }
            }
            for (key, change) in changes {
                match change {
                    Patch::Replace(value) if !object.contains_key(key) => {
                        object.insert(key.clone(), value.clone());
                    }
                    _ => {
                        let value = object.get_mut(key).ok_or_else(|| {
                            ReplayCacheError::new(
                                "object patch changes an absent non-replacement member",
                            )
                        })?;
                        apply_patch(value, change, depth + 1)?;
                    }
                }
            }
            Ok(())
        }
        Patch::Array(changes) => {
            let array = target.as_array_mut().ok_or_else(|| {
                ReplayCacheError::new("array patch targets a non-array value")
            })?;
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
        Patch::Keep => out.push(0),
        Patch::Replace(value) => {
            out.push(1);
            encode_value_into(value, out, depth + 1)?;
        }
        Patch::Object { removals, changes } => {
            out.push(2);
            put_u32(out, count_u32(removals.len())?);
            for key in removals {
                put_string(out, key)?;
            }
            put_u32(out, count_u32(changes.len())?);
            for (key, change) in changes {
                put_string(out, key)?;
                encode_patch_into(change, out, depth + 1)?;
            }
        }
        Patch::Array(changes) => {
            out.push(3);
            put_u32(out, count_u32(changes.len())?);
            for (index, change) in changes {
                put_u32(out, *index);
                encode_patch_into(change, out, depth + 1)?;
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
        0 => Ok(Patch::Keep),
        1 => Ok(Patch::Replace(decode_value(reader, depth + 1)?)),
        2 => {
            let removal_count = reader.count()?;
            let mut removals = Vec::with_capacity(removal_count);
            for _ in 0..removal_count {
                removals.push(reader.string()?);
            }
            if !strictly_increasing_strings(&removals) {
                return Err(ReplayCacheError::new(
                    "object patch removals are not strictly ordered",
                ));
            }
            let change_count = reader.count()?;
            let mut changes = Vec::with_capacity(change_count);
            for _ in 0..change_count {
                changes.push((reader.string()?, decode_patch(reader, depth + 1)?));
            }
            if !changes
                .windows(2)
                .all(|pair| pair[0].0.as_str() < pair[1].0.as_str())
            {
                return Err(ReplayCacheError::new(
                    "object patch changes are not strictly ordered",
                ));
            }
            Ok(Patch::Object { removals, changes })
        }
        3 => {
            let count = reader.count()?;
            let mut changes = Vec::with_capacity(count);
            for _ in 0..count {
                changes.push((reader.u32()?, decode_patch(reader, depth + 1)?));
            }
            if !changes.windows(2).all(|pair| pair[0].0 < pair[1].0) {
                return Err(ReplayCacheError::new(
                    "array patch indices are not strictly ordered",
                ));
            }
            Ok(Patch::Array(changes))
        }
        tag => Err(ReplayCacheError::new(format!(
            "unknown binary patch tag {tag}"
        ))),
    }
}

fn encode_value(value: &Value) -> Result<Vec<u8>, ReplayCacheError> {
    let mut out = Vec::new();
    encode_value_into(value, &mut out, 0)?;
    Ok(out)
}

fn encode_value_into(
    value: &Value,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), ReplayCacheError> {
    require_depth(depth)?;
    match value {
        Value::Null => out.push(0),
        Value::Bool(false) => out.push(1),
        Value::Bool(true) => out.push(2),
        Value::Number(number) => {
            out.push(3);
            put_string(out, &number.to_string())?;
        }
        Value::String(value) => {
            out.push(4);
            put_string(out, value)?;
        }
        Value::Array(values) => {
            out.push(5);
            put_u32(out, count_u32(values.len())?);
            for value in values {
                encode_value_into(value, out, depth + 1)?;
            }
        }
        Value::Object(values) => {
            out.push(6);
            put_u32(out, count_u32(values.len())?);
            for (key, value) in values {
                put_string(out, key)?;
                encode_value_into(value, out, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn decode_value_exact(bytes: &[u8]) -> Result<Value, ReplayCacheError> {
    let mut reader = ByteReader::new(bytes);
    let value = decode_value(&mut reader, 0)?;
    reader.finish()?;
    Ok(value)
}

fn decode_value(reader: &mut ByteReader<'_>, depth: usize) -> Result<Value, ReplayCacheError> {
    require_depth(depth)?;
    match reader.u8()? {
        0 => Ok(Value::Null),
        1 => Ok(Value::Bool(false)),
        2 => Ok(Value::Bool(true)),
        3 => {
            let text = reader.string()?;
            let number: Number = serde_json::from_str(&text).map_err(|error| {
                ReplayCacheError::new(format!("invalid binary JSON number {text:?}: {error}"))
            })?;
            if number.to_string() != text {
                return Err(ReplayCacheError::new(
                    "binary JSON number is not in canonical serde_json form",
                ));
            }
            Ok(Value::Number(number))
        }
        4 => Ok(Value::String(reader.string()?)),
        5 => {
            let count = reader.count()?;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(decode_value(reader, depth + 1)?);
            }
            Ok(Value::Array(values))
        }
        6 => {
            let count = reader.count()?;
            let mut values = Map::new();
            let mut previous: Option<String> = None;
            for _ in 0..count {
                let key = reader.string()?;
                if previous.as_ref().is_some_and(|value| key <= *value) {
                    return Err(ReplayCacheError::new(
                        "binary JSON object keys are not strictly ordered",
                    ));
                }
                let value = decode_value(reader, depth + 1)?;
                previous = Some(key.clone());
                values.insert(key, value);
            }
            Ok(Value::Object(values))
        }
        tag => Err(ReplayCacheError::new(format!(
            "unknown binary JSON value tag {tag}"
        ))),
    }
}

fn validate_world_identity(
    value: &Value,
    hour_index: i64,
    date: &str,
) -> Result<(), ReplayCacheError> {
    if value.get("hour_index").and_then(Value::as_i64) != Some(hour_index)
        || value.get("date").and_then(Value::as_str) != Some(date)
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

fn strictly_increasing_strings(values: &[String]) -> bool {
    values
        .windows(2)
        .all(|pair| pair[0].as_str() < pair[1].as_str())
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
fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), ReplayCacheError> {
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
    fn count(&mut self) -> Result<usize, ReplayCacheError> {
        let value = self.u32()? as usize;
        if value > MAX_HOURS.max(MAX_STRING_BYTES) {
            return Err(ReplayCacheError::new(
                "binary collection count exceeds the global bound",
            ));
        }
        Ok(value)
    }
    fn string(&mut self) -> Result<String, ReplayCacheError> {
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

    fn world(hour: i64, value: Value) -> CacheBuildHour {
        CacheBuildHour {
            hour_index: hour,
            date: format!("1936.1.1.{}", hour + 1),
            h4cr_start_offset: (hour as u64) * 100,
            h4cr_end_offset: (hour as u64 + 1) * 100,
            world_json: serde_json::to_vec(&serde_json::json!({
                "game_id": "g",
                "schema_version": "1",
                "hour_index": hour,
                "date": format!("1936.1.1.{}", hour + 1),
                "world": value,
            }))
            .unwrap(),
        }
    }

    #[test]
    fn sparse_checkpoints_reconstruct_every_hour_with_a_hard_seek_bound() {
        let source_sha = "ab".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 3).unwrap();
        for (hour, value) in [
            (0, serde_json::json!({"rows": [1, 2], "keep": true})),
            (1, serde_json::json!({"rows": [1, 3], "keep": true})),
            (2, serde_json::json!({"rows": [1, 3], "extra": "x"})),
            (3, serde_json::json!({"rows": [9], "extra": "x"})),
            (4, serde_json::json!({"rows": [9], "extra": "y"})),
        ] {
            encoder.push(world(hour, value)).unwrap();
        }
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::IdentityV1, 500)
            .unwrap();
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
        for hour in 0..5 {
            let (json, receipt) = reader.reconstruct_json(hour).unwrap();
            let body: Value = serde_json::from_slice(&json).unwrap();
            assert_eq!(body["hour_index"], hour);
            assert!(receipt.applied_deltas <= 2);
            assert_eq!(receipt.configured_bound, 2);
        }
    }

    #[test]
    fn cache_and_index_corruption_fail_closed() {
        let source_sha = "01".repeat(32);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha, 2).unwrap();
        encoder.push(world(0, serde_json::json!({"x": 1}))).unwrap();
        encoder.push(world(1, serde_json::json!({"x": 2}))).unwrap();
        let (cache, index) = encoder
            .finish("g", "1", CacheIdentityMode::Legacy, 200)
            .unwrap();
        let mut index_bytes = index.to_bytes().unwrap();
        index_bytes[0] ^= 0x40;
        assert!(ReplayHourIndex::from_bytes(&index_bytes).is_err());

        let index_bytes = index.to_bytes().unwrap();
        let mut corrupt_cache = cache;
        let last = corrupt_cache.len() - 1;
        corrupt_cache[last] ^= 1;
        assert!(ReplayCacheReader::open(&index_bytes, &corrupt_cache, 200, &source_sha).is_err());

        let mut incomplete_source_index = index;
        incomplete_source_index.hours.last_mut().unwrap().h4cr_end_offset = 199;
        assert!(
            incomplete_source_index.to_bytes().is_err(),
            "the hourly offset index must cover the canonical H4CR source through EOF"
        );
    }
}
