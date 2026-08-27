//! The canonical id-set block: one sorted set of `u32` ids, one legal byte
//! string.
//!
//! # What it is for
//!
//! Several places in this product have to put "which things did I look at" on
//! the wire, next to "here is what I found". Domain 48's walk visits state and
//! province roots; each read returns rows (possibly none), rows with some
//! skipped, or nothing. If only the rows reach the wire, a consumer cannot tell
//! "scanned and genuinely empty" from "scanned and failed", from "scanned and
//! partly skipped", from "never in the root list". Treating absence as zero
//! silently under-counts every country; treating it as unavailable makes most
//! countries unusable. Id sets are what separate them, and this is their codec.
//!
//! # Why three encodings
//!
//! Measured on lane NA's t88 capture (60 hours, one d48 frame each):
//!
//! * the scanned STATE set is 1,081 contiguous ids - ONE run, about 4 bytes,
//!   where a bitmap is 136 and a delta list is ~1,081;
//! * the scanned PROVINCE set is ~10,272 ids scattered over a 13,412-wide span,
//!   because the walk drops every province with no state and sea provinces
//!   punch holes through the id space. Its ids with rows form 2,371 runs: runs
//!   ~9,484 bytes, delta ~7,892, BITMAP 1,677.
//!
//! No encoding dominates, so each block carries its own tag and the encoder
//! picks the smallest. It PRICES all three without building any of them - the
//! three `*_len` functions are streaming passes that allocate nothing - and
//! then builds only the winner. That matters twice over: a set of
//! `{1, u32::MAX}` has a 512 MiB bitmap candidate, and this code runs inside
//! the running game.
//!
//! # Canonical, so that equal sets are equal bytes
//!
//! Runs are MAXIMAL (two adjacent runs are illegal), LEB128 is canonical and
//! bounded, the bitmap's first and last bits are set and its padding is zero,
//! and the empty set has exactly one spelling. Two passes over the same world
//! must produce the same bytes, and a decoder must never accept two spellings
//! of one set.
//!
//! # Why a decoder needs a resource ceiling it did not read off the wire
//!
//! The encodings COMPRESS, so a declared cardinality is not paid for in bytes:
//! a canonical fifteen-byte runs block can legally declare one maximal run
//! covering the whole `u32` space. Every endpoint, maximality, cardinality and
//! varint rule passes, and the decoder is asked for two billion pushes.
//!
//! So `decode_set_block` takes `max_ids` and refuses a larger declaration
//! BEFORE it builds anything. The caller must not derive that bound from the
//! same untrusted payload - see [`MAX_DECODABLE_IDS`], which is the ceiling
//! callers use when the only other candidate would be a wire word.

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// The most ids any one set block may name.
///
/// This is a RESOURCE ceiling, not a claim about game content, and it is
/// derived from the producer rather than invented: the recorder refuses to walk
/// a province array larger than 65,536 slots
/// (`MAX_MAP_CONTROL_PROVINCE_COUNT`, enforced at
/// `attach_log.rs::validate_map_control_province_bridge` and again before the
/// topology walk). Every state is discovered THROUGH a province - the state
/// roster is built by following each province's state pointer - so neither a
/// province set nor a state set this product can produce may exceed that
/// number. A map bigger than the recorder will walk is refused at CAPTURE
/// time, loudly, rather than mis-decoded quietly here.
///
/// Mod safety is therefore unchanged: a mod may define as many states as the
/// recorder is willing to capture, and no fewer than it could before. What
/// this forbids is a corrupt or hostile payload asking a replay to materialise
/// billions of ids, which caps a decoded set at 256 KiB instead of 8 GiB.
pub const MAX_DECODABLE_IDS: usize = 65_536;

/// How one id set is spelled. The tag is per BLOCK, not per trailer: the state
/// sets and the province sets have opposite best encodings in the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetEncoding {
    /// Maximal contiguous intervals. Best for the scanned state set.
    Runs = 1,
    /// One varint gap per id. Best for a scattered set of few ids.
    Delta = 2,
    /// One bit per id in `[min_id, min_id + bit_len)`. Best for the scanned
    /// province set.
    Bitmap = 3,
}

/// The tag and its reverse, ONE declaration. The discriminant was the writer
/// and a literal `match` the reader, so exchanging two discriminants compiled
/// and read each block under the other encoding (lane TV). The writer is
/// generated from the same column, as an exhaustive match rather than an `as`
/// cast, so an encoding left out of the column fails to compile instead of
/// writing a tag its own reader refuses.
macro_rules! set_encodings {
    ($( $variant:ident = $tag:literal, )+) => {
        impl SetEncoding {
            pub const fn tag(self) -> u8 {
                match self {
                    $(Self::$variant => $tag,)+
                }
            }

            fn from_tag(tag: u8) -> Option<Self> {
                match tag {
                    $($tag => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        const _: () = {
            $(assert!(
                SetEncoding::$variant as u8 == $tag,
                "an encoding's declared tag is not its own discriminant",
            );)+
            let tags = [$($tag,)+];
            let mut i = 0;
            while i < tags.len() {
                let mut j = i + 1;
                while j < tags.len() {
                    assert!(tags[i] != tags[j], "two encodings share a tag");
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

set_encodings! {
    Runs = 1,
    Delta = 2,
    Bitmap = 3,
}

/// The empty set has ONE spelling: `Runs`, zero ids, zero bytes.
///
/// Not a stylistic choice. `Runs` and `Delta` both begin with a `first_id` and
/// `Bitmap` requires a nonzero `bit_len` with its first and last bits set, so
/// none of the three can express an empty set at all - and a HEALTHY frame has
/// three empty sets in it, because nothing failed. Without this rule the
/// encoder cannot emit the ordinary case.
pub const EMPTY_SET_ENCODING: SetEncoding = SetEncoding::Runs;

/// A `u32` LEB128 never needs more than five bytes, and a sixth is always an
/// overlong form.
const MAX_LEB128_LEN: usize = 5;

fn leb128_len(mut value: u32) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn write_leb128(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Read one canonical LEB128 `u32`, or say why not.
///
/// Refuses an overlong encoding - `81 00` for 1 - because two spellings of one
/// number are two spellings of one SET, and a census whose bytes are not
/// canonical cannot be compared for equality across two passes.
fn read_leb128(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut value: u64 = 0;
    let mut shift = 0;
    for index in 0..MAX_LEB128_LEN {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| "varint runs past the end of the block".to_string())?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            // Canonical: a multi-byte form whose last byte is zero encodes a
            // value a shorter form already encodes.
            if index > 0 && byte == 0 {
                return Err("overlong varint".to_string());
            }
            return u32::try_from(value).map_err(|_| "varint exceeds u32".to_string());
        }
        shift += 7;
    }
    Err("varint longer than five bytes".to_string())
}

/// Walk the maximal contiguous intervals of a sorted, deduplicated id list,
/// handing each to `emit` as `(first_id, len)`, ALLOCATING NOTHING.
///
/// Maximality is what makes the runs encoding canonical: `[1,2,3]` is one run
/// and never two. This is a streaming pass rather than a `Vec<(u32, u32)>`
/// because it is called to PRICE the runs encoding even when runs lose, and it
/// runs inside the game — building a run list only to throw it away was the
/// allocation the module header promised was not happening.
fn for_each_run(ids: &[u32], mut emit: impl FnMut(u32, u32)) {
    let mut current: Option<(u32, u32)> = None;
    for &id in ids {
        match current {
            // `first + len` cannot overflow here: it is the id after the run's
            // last, and the run's last is a `u32` that was strictly less than
            // `id`.
            Some((first, len)) if u64::from(first) + u64::from(len) == u64::from(id) => {
                current = Some((first, len + 1));
            }
            Some((first, len)) => {
                emit(first, len);
                current = Some((id, 1));
            }
            None => current = Some((id, 1)),
        }
    }
    if let Some((first, len)) = current {
        emit(first, len);
    }
}

/// How many bytes `Runs` would take. Arithmetic only - nothing is built.
fn runs_len(ids: &[u32]) -> usize {
    let mut len = 0usize;
    let mut previous_last: Option<u32> = None;
    for_each_run(ids, |first, run_len| {
        match previous_last {
            None => len += leb128_len(first),
            // The gap from the PREVIOUS RUN'S LAST id. Runs are maximal, so a
            // following run starts at least two past it, and `gap - 1 >= 1`.
            Some(last) => len += leb128_len(first - last - 1),
        }
        len += leb128_len(run_len - 1);
        // `first + (run_len - 1)`, NOT `first + run_len - 1`. A run of one at
        // `u32::MAX` overflows the second spelling before the subtraction can
        // bring it back, which panics in debug and wraps in release. The run's
        // last id always fits, because it is an id.
        previous_last = Some(first + (run_len - 1));
    });
    len
}

/// How many bytes `Delta` would take. Arithmetic only.
fn delta_len(ids: &[u32]) -> usize {
    let mut len = 0usize;
    let mut previous: Option<u32> = None;
    for &id in ids {
        match previous {
            None => len += leb128_len(id),
            Some(prev) => len += leb128_len(id - prev - 1),
        }
        previous = Some(id);
    }
    len
}

/// How many bytes `Bitmap` would take, or `None` when it cannot legally
/// represent this set.
///
/// `None` for the empty set (a bitmap needs a nonzero `bit_len`) and for a span
/// that will not fit a `u32` `bit_len`. THE ANSWER IS ARITHMETIC: a set of
/// `{1, u32::MAX}` reports about 512 MiB here and loses the comparison without
/// a single byte being allocated.
fn bitmap_len(ids: &[u32]) -> Option<usize> {
    let (&min, &max) = (ids.first()?, ids.last()?);
    let bit_len = u64::from(max) - u64::from(min) + 1;
    // `bit_len` is written as a u32, and `min + bit_len - 1` must stay inside
    // u32 - which it does by construction here, since `max` is a u32.
    u32::try_from(bit_len).ok()?;
    let bytes = usize::try_from(bit_len.div_ceil(8)).ok()?;
    Some(8 + bytes)
}

/// The encoding this set is smallest in, and how many bytes its payload takes.
///
/// Ties break to the lowest tag so two runs of the same recorder produce the
/// same bytes for the same world.
fn choose_encoding(ids: &[u32]) -> (SetEncoding, usize) {
    if ids.is_empty() {
        return (EMPTY_SET_ENCODING, 0);
    }
    let mut best = (SetEncoding::Runs, runs_len(ids));
    let delta = delta_len(ids);
    if delta < best.1 {
        best = (SetEncoding::Delta, delta);
    }
    if let Some(bitmap) = bitmap_len(ids) {
        if bitmap < best.1 {
            best = (SetEncoding::Bitmap, bitmap);
        }
    }
    best
}

/// Bytes of one block's payload. `ids` must be sorted and deduplicated.
fn write_set_payload(out: &mut Vec<u8>, ids: &[u32], encoding: SetEncoding) {
    match encoding {
        SetEncoding::Runs => {
            let mut previous_last: Option<u32> = None;
            for_each_run(ids, |first, len| {
                match previous_last {
                    None => write_leb128(out, first),
                    Some(last) => write_leb128(out, first - last - 1),
                }
                write_leb128(out, len - 1);
                // See `runs_len`: the parenthesisation is load-bearing at
                // `u32::MAX`.
                previous_last = Some(first + (len - 1));
            });
        }
        SetEncoding::Delta => {
            let mut previous: Option<u32> = None;
            for &id in ids {
                match previous {
                    None => write_leb128(out, id),
                    Some(prev) => write_leb128(out, id - prev - 1),
                }
                previous = Some(id);
            }
        }
        SetEncoding::Bitmap => {
            let (min, max) = (ids[0], ids[ids.len() - 1]);
            let bit_len = max - min + 1;
            out.extend_from_slice(&min.to_le_bytes());
            out.extend_from_slice(&bit_len.to_le_bytes());
            let mut bits = vec![0u8; (bit_len as usize).div_ceil(8)];
            for &id in ids {
                let offset = (id - min) as usize;
                bits[offset / 8] |= 1 << (offset % 8);
            }
            out.extend_from_slice(&bits);
        }
    }
}

/// One id set as it goes on the wire: a 9-byte block header and its payload.
///
/// `ids` MUST be strictly increasing (sorted, no duplicates) and no longer than
/// [`MAX_DECODABLE_IDS`].
///
/// FAIL-CLOSED, in release as well as debug. This used to be a `debug_assert!`,
/// which is exactly the guard that is not there in the shipping recorder: the
/// gap arithmetic below computes `id - previous - 1`, so a duplicate or a
/// descending pair underflows and the block that reaches the wire is nonsense
/// its own decoder would refuse. The collector encodes a frame BEFORE its gate
/// runs, so the gate cannot stand in for this check either. Refusing here costs
/// one comparison per id on a list we already walk three times.
pub fn encode_set_block(out: &mut Vec<u8>, ids: &[u32]) -> Result<(), String> {
    if !ids.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err("an id set must be strictly increasing before encoding".to_string());
    }
    if ids.len() > MAX_DECODABLE_IDS {
        return Err(format!(
            "an id set of {} ids exceeds the {MAX_DECODABLE_IDS} this codec will encode",
            ids.len()
        ));
    }
    let (encoding, payload_len) = choose_encoding(ids);
    out.push(encoding.tag());
    out.extend_from_slice(&(ids.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(payload_len.min(u32::MAX as usize) as u32).to_le_bytes());
    let before = out.len();
    write_set_payload(out, ids, encoding);
    debug_assert_eq!(
        out.len() - before,
        payload_len,
        "the measured length and the written length must agree, or the block \
         header lies about its own payload"
    );
    Ok(())
}

/// The block header's fixed size: tag, count, byte length.
pub const SET_BLOCK_HEADER_LEN: usize = 9;

/// Read one id set, refusing every shape that is not the one canonical
/// spelling.
///
/// Every id is reconstructed in CHECKED `u64` and required to land inside
/// `u32`. Without that, `first_id = u32::MAX` with `len - 1 = 1`, or
/// `min_id = u32::MAX` with `bit_len = 2`, would name an id that does not
/// exist and pass every other rule here.
/// `max_ids` is the largest cardinality this set could legitimately have — the
/// authenticated root count for a scanned set, or the scanned set's own size for
/// an incomplete one.
///
/// It exists because the encodings are COMPRESSED: a canonical `Runs` block of a
/// few bytes can legally declare `count = u32::MAX` and one maximal run covering
/// the whole id space, passing every endpoint, cardinality, maximality and
/// varint rule while asking the decoder for billions of pushes. A declared count
/// is untrusted input and nothing may be allocated or iterated from it alone.
///
/// **`max_ids` is clamped to [`MAX_DECODABLE_IDS`] here, and that clamp is the
/// load-bearing half.** Callers naturally reach for a bound that came off the
/// same payload — d48's `scanned_states` is bounded by the frame's own
/// `observed_state_roots` header word, and the session state census by the
/// structural id ceiling, which is `i32::MAX - 1`. Either lets a fifteen-byte
/// block ask for two billion pushes, so a caller-supplied bound can only ever
/// TIGHTEN this one, never raise it.
pub fn decode_set_block(bytes: &[u8], at: &mut usize, max_ids: usize) -> Result<Vec<u32>, String> {
    let tag = *bytes
        .get(*at)
        .ok_or_else(|| "census block header runs past the end".to_string())?;
    let encoding =
        SetEncoding::from_tag(tag).ok_or_else(|| format!("unknown census encoding {tag}"))?;
    let header_end = at
        .checked_add(SET_BLOCK_HEADER_LEN)
        .ok_or_else(|| "census block header overflows".to_string())?;
    if header_end > bytes.len() {
        return Err("census block header runs past the end".to_string());
    }
    let count = u32::from_le_bytes(bytes[*at + 1..*at + 5].try_into().expect("4 bytes")) as usize;
    let byte_len =
        u32::from_le_bytes(bytes[*at + 5..*at + 9].try_into().expect("4 bytes")) as usize;
    let payload_end = header_end
        .checked_add(byte_len)
        .ok_or_else(|| "census block payload overflows".to_string())?;
    if payload_end > bytes.len() {
        return Err("census block payload runs past the end".to_string());
    }
    let payload = &bytes[header_end..payload_end];
    *at = payload_end;

    if count == 0 {
        // ONE spelling for the empty set, and this is where the others are
        // refused. Accepting, say, a Delta block with zero ids would give the
        // empty set two encodings and break the canonicality every equality
        // check here depends on.
        if encoding != EMPTY_SET_ENCODING || byte_len != 0 {
            return Err(
                "an empty census set must be encoding 1 with a zero-length payload".to_string(),
            );
        }
        return Ok(Vec::new());
    }

    // Bounded BEFORE anything is built. See `max_ids`: the caller's bound may
    // tighten the codec's own resource ceiling but may never raise it, because
    // the caller's bound is usually another word off this same payload.
    let max_ids = max_ids.min(MAX_DECODABLE_IDS);
    if count > max_ids {
        return Err(format!(
            "id set declares {count} ids, more than the {max_ids} this set can hold"
        ));
    }

    let ids = match encoding {
        SetEncoding::Runs => decode_runs(payload, count)?,
        SetEncoding::Delta => decode_delta(payload, count)?,
        SetEncoding::Bitmap => decode_bitmap(payload, count)?,
    };
    debug_assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    // THE ENCODINGS ARE CANONICAL INDIVIDUALLY BUT NOT ACROSS TAGS. `{1}` is a
    // legal one-run block, a legal one-delta block AND a legal one-bit bitmap,
    // so without this the same set has three legal spellings and two recorders
    // reading one world can disagree byte for byte. Recomputing the winner and
    // demanding the received block match it makes the encoding injective:
    // exactly one legal byte string per set.
    let (expected_encoding, expected_len) = choose_encoding(&ids);
    if encoding != expected_encoding || byte_len != expected_len {
        return Err(format!(
            "census set is not in its canonical encoding: got tag {} with {byte_len} bytes, \
             the only legal spelling is tag {} with {expected_len}",
            encoding.tag(),
            expected_encoding.tag()
        ));
    }
    Ok(ids)
}

fn decode_runs(payload: &[u8], count: usize) -> Result<Vec<u32>, String> {
    let mut ids: Vec<u32> = Vec::with_capacity(count.min(1 << 16));
    let mut at = 0usize;
    let mut previous_last: Option<u32> = None;
    while ids.len() < count {
        let first = match previous_last {
            None => read_leb128(payload, &mut at)?,
            Some(last) => {
                let gap_minus_one = read_leb128(payload, &mut at)?;
                // Maximality: the next run starts at least TWO past the last id,
                // so `gap - 1 >= 1`. A zero here is an adjacent run, which is a
                // second spelling of one set.
                if gap_minus_one == 0 {
                    return Err("census runs are not maximal".to_string());
                }
                let first = u64::from(last) + u64::from(gap_minus_one) + 1;
                u32::try_from(first).map_err(|_| "census run start exceeds u32".to_string())?
            }
        };
        let len_minus_one = read_leb128(payload, &mut at)?;
        let last = u64::from(first) + u64::from(len_minus_one);
        let last = u32::try_from(last).map_err(|_| "census run end exceeds u32".to_string())?;
        let len = usize::try_from(u64::from(len_minus_one) + 1).expect("fits usize");
        if ids.len() + len > count {
            return Err("census runs name more ids than the block declares".to_string());
        }
        for id in first..=last {
            ids.push(id);
        }
        previous_last = Some(last);
    }
    if at != payload.len() {
        return Err("census block has trailing bytes".to_string());
    }
    Ok(ids)
}

fn decode_delta(payload: &[u8], count: usize) -> Result<Vec<u32>, String> {
    let mut ids: Vec<u32> = Vec::with_capacity(count.min(1 << 16));
    let mut at = 0usize;
    let mut previous: Option<u32> = None;
    for _ in 0..count {
        let id = match previous {
            None => read_leb128(payload, &mut at)?,
            Some(prev) => {
                let gap_minus_one = read_leb128(payload, &mut at)?;
                let id = u64::from(prev) + u64::from(gap_minus_one) + 1;
                u32::try_from(id).map_err(|_| "census delta exceeds u32".to_string())?
            }
        };
        ids.push(id);
        previous = Some(id);
    }
    if at != payload.len() {
        return Err("census block has trailing bytes".to_string());
    }
    Ok(ids)
}

fn decode_bitmap(payload: &[u8], count: usize) -> Result<Vec<u32>, String> {
    if payload.len() < 8 {
        return Err("census bitmap header runs past the end".to_string());
    }
    let min = u32::from_le_bytes(payload[0..4].try_into().expect("4 bytes"));
    let bit_len = u32::from_le_bytes(payload[4..8].try_into().expect("4 bytes"));
    if bit_len == 0 {
        return Err("census bitmap has a zero length".to_string());
    }
    // The highest id this bitmap can name must exist. `min = u32::MAX` with
    // `bit_len = 2` names u32::MAX + 1, and every other rule here would pass it.
    let max = u64::from(min) + u64::from(bit_len) - 1;
    u32::try_from(max).map_err(|_| "census bitmap end exceeds u32".to_string())?;
    let byte_len = (bit_len as usize).div_ceil(8);
    if payload.len() != 8 + byte_len {
        return Err("census bitmap length does not match its bit count".to_string());
    }
    let bits = &payload[8..];
    let mut ids: Vec<u32> = Vec::with_capacity(count.min(1 << 16));
    for (index, byte) in bits.iter().enumerate() {
        let mut byte = *byte;
        while byte != 0 {
            let bit = byte.trailing_zeros() as usize;
            byte &= byte - 1;
            let offset = index * 8 + bit;
            if offset >= bit_len as usize {
                // Padding bits past `bit_len` must be zero: a set bit there is a
                // second spelling, and it also names an id outside the span the
                // header declared.
                return Err("census bitmap has a nonzero padding bit".to_string());
            }
            ids.push(min + offset as u32);
        }
    }
    // POPCOUNT is the count. Without this a block could declare `count = 1`
    // while naming nothing, which turns a NAMED unavailability into silence -
    // exactly the failure this census exists to prevent.
    if ids.len() != count {
        return Err("census bitmap set bits do not match its declared count".to_string());
    }
    // Canonical: the span is exactly the set, so both ends are set.
    if ids.first() != Some(&min) {
        return Err("census bitmap does not start at its own minimum".to_string());
    }
    if u64::from(*ids.last().expect("nonempty")) != max {
        return Err("census bitmap does not end at its own maximum".to_string());
    }
    Ok(ids)
}
