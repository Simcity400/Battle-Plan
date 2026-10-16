//! Reverse lexer dictionary, copied once between mutation witnesses.
//!
//! Dynamic insertion holds the writer flag and increments the dynamic count
//! before touching reverse strings. Lookup rebuild additionally grows the
//! table extent. Reset and runtime destruction follow the application update
//! loop. Matching writer-free witnesses permit hourly publication; an
//! overlapping change discards the table without retry or locking. The hourly
//! update holds the simulation thread rather than the lexer lock. These
//! checks cover insertion and rebuild, but cannot exclude arbitrary writes
//! through escaped string aliases.

use crate::{Memory, msvc_string, read, read_slice, walk::Outcome};
use std::sync::atomic::{Ordering, compiler_fence};

/// Empty research-slot token authenticated by the shared fixed-token contract.
pub const IDLE_RESEARCH_TOKEN: u32 = 0x2a4e;

/// Maximum-speed token authenticated by the shared fixed-token contract.
pub const MAXIMUM_SPEED_TOKEN: u32 = 0x2eb2;

/// Lexer initialized byte required to be one.
pub const INITIALIZED_RVA: usize = 0x35c8b50;
/// Maximum admitted token ID, signed i32, used as an inclusive bound.
pub const MAX_TOKEN_ID_RVA: usize = 0x35c8b54;
/// Static maximum calculated from loaded static token definitions.
pub const STATIC_MAX_TOKEN_ID_RVA: usize = 0x35c8b5c;
/// Dynamic-record pointer to its separate 0x48-byte-record array.
pub const DYNAMIC_TOKENS_RVA: usize = 0x35c8b60;
/// Dynamic-array signed capacity, compared against count.
pub const DYNAMIC_CAPACITY_RVA: usize = 0x35c8b68;
/// Dynamic count, incremented before reverse-name assignment or rebuild.
pub const DYNAMIC_COUNT_RVA: usize = 0x35c8b6c;
/// Token lookup read lock.
pub const READ_LOCK_RVA: usize = 0x35c8b78;
/// Exclusive flag set atomically and decremented on release.
pub const WRITER_FLAG_RVA: usize = 0x35c8b7c;
/// Token-name table pointer.
pub const TOKEN_NAMES_RVA: usize = 0x35c8b80;
/// Reverse-string allocation capacity used during lookup rebuilding.
pub const TOKEN_ALLOCATION_RVA: usize = 0x35c8b88;
/// Token-name capacity, signed i32, validated against the maximum token ID.
pub const TOKEN_CAPACITY_RVA: usize = 0x35c8b8c;
/// MSVC string entry stride for token-name lookup.
pub const TOKEN_NAME_STRIDE: usize = 0x20;
/// Token-string lookup using the inclusive ID bound and string address.
pub const GET_TOKEN_STRING_RVA: usize = 0x24a7300;
/// Dynamic-token wrapper used during strategic-resource loading.
pub const ADD_DYNAMIC_TOKEN_RVA: usize = 0x24a6500;
/// Dynamic-token insertion implementation.
pub const ADD_DYNAMIC_TOKEN_IMPL_RVA: usize = 0x24a65a0;
/// Static long-fixed token formatted as fixed15 numeric text instead of its
/// spelling; saves never serialize the name.
pub const FIXED15_TOKEN: u32 = 0x167;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub token: u32,
    pub text_start: usize,
    pub text_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub text_len: usize,
    pub text_required: usize,
}

impl Walk {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            outcome: Outcome::Unavailable { reason },
            text_len: 0,
            text_required: 0,
        }
    }
}

/// Process-lifetime storage identity. Dynamic ids only increase during valid
/// play; reset/destruction occur outside the application's hourly update loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedWitness {
    maximum: u32,
    static_maximum: u32,
    dynamic_pointer: u64,
    dynamic_capacity: u32,
    dynamic_count: u32,
    table: usize,
    allocation: u32,
    length: u32,
}

impl PreparedWitness {
    fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let at = |offset| base.checked_add(offset).ok_or("lexer address overflow");
        let writer = at(WRITER_FLAG_RVA)?;
        // The pinned x86-64 scalar loads are aligned and load/load ordered.
        // Compiler fences also keep the foreign copies inside their witnesses.
        compiler_fence(Ordering::SeqCst);
        if read::<u32>(memory, writer) != Some(0) {
            return Err("lexer writer active or unreadable");
        }
        compiler_fence(Ordering::SeqCst);
        let header =
            read::<[u8; 64]>(memory, at(INITIALIZED_RVA)?).ok_or("lexer witness unreadable")?;
        compiler_fence(Ordering::SeqCst);
        if read::<u32>(memory, writer) != Some(0) {
            return Err("lexer writer active or unreadable");
        }
        compiler_fence(Ordering::SeqCst);
        Self::from_header(header)
    }

    /// Validate a captured native witness header without reading live memory.
    /// Callers must separately retain the two surrounding writer-flag checks.
    pub fn from_header(header: [u8; 64]) -> Result<Self, &'static str> {
        if header[0] != 1 {
            return Err("lexer is not initialized");
        }
        let signed = |rva: usize| -> Result<u32, &'static str> {
            let offset = rva - INITIALIZED_RVA;
            u32::try_from(i32::from_le_bytes(
                header[offset..offset + 4].try_into().unwrap(),
            ))
            .map_err(|_| "lexer witness has negative count")
        };
        let pointer = |rva: usize| {
            let offset = rva - INITIALIZED_RVA;
            u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap())
        };
        if signed(WRITER_FLAG_RVA)? != 0 {
            return Err("lexer writer active during witness");
        }
        let witness = Self {
            maximum: signed(MAX_TOKEN_ID_RVA)?,
            static_maximum: signed(STATIC_MAX_TOKEN_ID_RVA)?,
            dynamic_pointer: pointer(DYNAMIC_TOKENS_RVA),
            dynamic_capacity: signed(DYNAMIC_CAPACITY_RVA)?,
            dynamic_count: signed(DYNAMIC_COUNT_RVA)?,
            table: usize::try_from(pointer(TOKEN_NAMES_RVA))
                .map_err(|_| "lexer pointer overflow")?,
            allocation: signed(TOKEN_ALLOCATION_RVA)?,
            length: signed(TOKEN_CAPACITY_RVA)?,
        };
        if witness.static_maximum.checked_add(witness.dynamic_count) != Some(witness.maximum)
            || witness.dynamic_count > witness.dynamic_capacity
            || (witness.dynamic_count != 0 && witness.dynamic_pointer == 0)
            || witness.table == 0
            || witness.maximum >= witness.length
            || witness.length > witness.allocation
        {
            return Err("lexer witness has inconsistent extents");
        }
        witness
            .table
            .checked_add(
                (witness.length as usize)
                    .checked_mul(TOKEN_NAME_STRIDE)
                    .ok_or("lexer token table extent overflow")?,
            )
            .ok_or("lexer token table extent overflow")?;
        Ok(witness)
    }

    pub fn maximum(self) -> u32 {
        self.maximum
    }

    pub fn static_maximum(self) -> u32 {
        self.static_maximum
    }

    pub fn table(self) -> usize {
        self.table
    }
}

/// Attach/worker sizing follows the inclusive token maximum and reverse
/// strings between the same writer witnesses as acquisition. Only readable
/// payload bytes become text demand.
pub fn demand(memory: &impl Memory, base: usize) -> Result<(usize, usize), &'static str> {
    let before = PreparedWitness::read(memory, base)?;
    let dictionary = TokenNames {
        table: before.table,
        maximum: before.maximum,
    };
    let rows = before.maximum as usize + 1;
    let mut bytes = 0usize;
    for token in 0..rows {
        let name = dictionary.text(memory, token as u32)?;
        let end = bytes
            .checked_add(name.len())
            .ok_or("lexer text arena extent overflow")?;
        probe_growth_prefix(memory, name.source(), name.len(), bytes, end)?;
        bytes = end;
    }
    if PreparedWitness::read(memory, base)? != before {
        return Err("lexer changed during sizing");
    }
    Ok((rows, bytes))
}

/// Copy all admitted native ordinals, including empty/reserved entries. Only
/// stable capacity shortages publish a prefix; mutation discards even that prefix
/// and any observed growth requirement. The caller must use the accepted hourly
/// callback: process shutdown/CRT destruction is outside this lifetime protocol.
pub fn walk(memory: &impl Memory, base: usize, rows: &mut [Row], text: &mut [u8]) -> Walk {
    let before = match PreparedWitness::read(memory, base) {
        Ok(witness) => witness,
        Err(reason) => return Walk::unavailable(reason),
    };
    let dictionary = TokenNames {
        table: before.table,
        maximum: before.maximum,
    };
    let total = before.maximum as usize + 1;
    let mut result = Walk {
        outcome: Outcome::Rows { len: 0 },
        text_len: 0,
        text_required: 0,
    };
    let mut len = 0;
    for (token, destination) in rows.iter_mut().take(total).enumerate() {
        let name = match dictionary.text(memory, token as u32) {
            Ok(name) => name,
            Err(reason) => return Walk::unavailable(reason),
        };
        let Some(end) = result.text_len.checked_add(name.len()) else {
            return Walk::unavailable("lexer text arena extent overflow");
        };
        if end > text.len() {
            result.text_required = match probe_growth_prefix(
                memory,
                name.source(),
                name.len(),
                result.text_len,
                text.len(),
            ) {
                Ok(required) => required,
                Err(reason) => return Walk::unavailable(reason),
            };
            break;
        }
        if let Err(reason) = name.copy(memory, &mut text[result.text_len..end]) {
            return Walk::unavailable(reason);
        }
        *destination = Row {
            token: token as u32,
            text_start: result.text_len,
            text_len: name.len(),
        };
        result.text_len = end;
        result.text_required = end;
        len += 1;
    }
    let after = match PreparedWitness::read(memory, base) {
        Ok(witness) => witness,
        Err(reason) => return Walk::unavailable(reason),
    };
    if before != after {
        return Walk::unavailable("lexer changed during copy");
    }
    result.outcome = if len == total {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: (total - len) as u32,
        }
    };
    result
}

/// Certify only the next bounded growth step for an unacquired string payload.
/// A declared length is not allocation evidence. Probe in source order using
/// fixed scratch storage; the first failed chunk is terminal for this capture.
/// Successful bytes are not published or reused on the next scheduled capture.
pub fn probe_growth_prefix(
    memory: &impl Memory,
    address: usize,
    len: usize,
    used: usize,
    capacity: usize,
) -> Result<usize, &'static str> {
    const CHUNK: usize = 4096;
    let end = used
        .checked_add(len)
        .ok_or("lexer text arena extent overflow")?;
    let target = end.min(capacity.saturating_mul(2).max(CHUNK));
    let prefix = target
        .checked_sub(used)
        .ok_or("lexer text arena extent overflow")?;
    let mut scratch = [0; CHUNK];
    for offset in (0..prefix).step_by(CHUNK) {
        let count = (prefix - offset).min(CHUNK);
        let source = address
            .checked_add(offset)
            .ok_or("lexer token extent overflow")?;
        if !read_slice(memory, source, &mut scratch[..count]) {
            return Err("lexer token bytes unreadable");
        }
    }
    Ok(target)
}

/// Foreign addresses never escape the single witnessed walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenNames {
    table: usize,
    maximum: u32,
}

/// Fails unless the dictionary still matches the witness taken before a
/// caller's individual lookups, with no writer active at either edge.
pub fn unchanged(
    memory: &impl Memory,
    base: usize,
    before: PreparedWitness,
) -> Result<(), &'static str> {
    if PreparedWitness::read(memory, base)? == before {
        Ok(())
    } else {
        Err("lexer changed during lookup")
    }
}

impl TokenNames {
    /// The dictionary for individual lookups between two witnesses; the caller
    /// must pass the returned witness to [`unchanged`] after its last read.
    pub fn witnessed(
        memory: &impl Memory,
        base: usize,
    ) -> Result<(Self, PreparedWitness), &'static str> {
        let witness = PreparedWitness::read(memory, base)?;
        Ok((
            Self {
                table: witness.table,
                maximum: witness.maximum,
            },
            witness,
        ))
    }

    /// Resolves a native ordinal to its exact bytes without allocating or
    /// copying names that a caller already emitted into its frame dictionary.
    pub fn text(self, memory: &impl Memory, token: u32) -> Result<TokenText, &'static str> {
        if token > self.maximum {
            return Err("lexer token exceeds native maximum");
        }
        let address = self
            .table
            .checked_add(
                (token as usize)
                    .checked_mul(TOKEN_NAME_STRIDE)
                    .ok_or("lexer token address overflow")?,
            )
            .ok_or("lexer token address overflow")?;
        let header =
            read::<[u8; 32]>(memory, address).ok_or("lexer token string header unreadable")?;
        let text = TokenText::from_header(address, header)?;
        if read::<u8>(memory, text.source + text.len) != Some(0) {
            return Err("lexer token terminator unreadable or invalid");
        }
        Ok(text)
    }
}

/// A callback-local foreign byte source, not portable identity. Serialize the
/// native ordinal and copied bytes; never serialize or cache this pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenText {
    source: usize,
    len: usize,
}

impl TokenText {
    /// Parse the same exact header as `TokenNames::text`; the independent
    /// terminator observation still has to succeed and contain zero.
    pub fn from_header(address: usize, header: [u8; 32]) -> Result<Self, &'static str> {
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
        if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
            return Err("lexer token string header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "lexer token length overflow")?;
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            address
        } else {
            usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .ok()
                .filter(|&p| p != 0)
                .ok_or("lexer token pointer invalid")?
        };
        source
            .checked_add(len)
            .ok_or("lexer token extent overflow")?;
        Ok(TokenText { source, len })
    }

    pub fn source(self) -> usize {
        self.source
    }

    pub fn len(self) -> usize {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    /// Insufficient capacity leaves the output untouched; the caller reports a
    /// truncated whole dictionary row and grows its arena off the game thread.
    pub fn copy(self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        if output.len() < self.len {
            return Err("lexer token destination too small");
        }
        if !read_slice(memory, self.source, &mut output[..self.len]) {
            return Err("lexer token bytes unreadable");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::Cell;

    struct WitnessMemory {
        before: [u8; 64],
        after: [u8; 64],
        names: [u8; 64],
        copied: Cell<bool>,
        writer_reads: Cell<usize>,
        busy_on: Option<usize>,
        unreadable_text: bool,
    }

    impl Memory for WitnessMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == WRITER_FLAG_RVA {
                let count = self.writer_reads.get() + 1;
                self.writer_reads.set(count);
                destination.copy_from_slice(&u32::from(self.busy_on == Some(count)).to_le_bytes());
                return true;
            }
            if address == INITIALIZED_RVA {
                destination.copy_from_slice(if self.copied.get() {
                    &self.after
                } else {
                    &self.before
                });
                return true;
            }
            self.copied.set(true);
            if self.unreadable_text && address == 0x1000 && destination.len() == 1 {
                return false;
            }
            SliceMemory {
                base: 0x1000,
                bytes: &self.names,
            }
            .copy(address, destination)
        }
    }

    fn set_count(header: &mut [u8; 64], rva: usize, value: i32) {
        let offset = rva - INITIALIZED_RVA;
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn witness_memory() -> WitnessMemory {
        let mut header = [0; 64];
        header[0] = 1;
        set_count(&mut header, MAX_TOKEN_ID_RVA, 1);
        set_count(&mut header, STATIC_MAX_TOKEN_ID_RVA, 1);
        set_count(&mut header, TOKEN_ALLOCATION_RVA, 2);
        set_count(&mut header, TOKEN_CAPACITY_RVA, 2);
        header[48..56].copy_from_slice(&0x1000_u64.to_le_bytes());
        let mut names = [0; 64];
        names[0] = b'x';
        names[16..24].copy_from_slice(&1_u64.to_le_bytes());
        names[24..32].copy_from_slice(&15_u64.to_le_bytes());
        names[32..34].copy_from_slice(b"yz");
        names[48..56].copy_from_slice(&2_u64.to_le_bytes());
        names[56..64].copy_from_slice(&15_u64.to_le_bytes());
        WitnessMemory {
            before: header,
            after: header,
            names,
            copied: Cell::new(false),
            writer_reads: Cell::new(0),
            busy_on: None,
            unreadable_text: false,
        }
    }

    fn expect_unavailable(memory: &WitnessMemory) {
        let result = walk(memory, 0, &mut [Row::default(); 2], &mut [0; 8]);
        assert!(
            matches!(result.outcome, Outcome::Unavailable { .. }),
            "{result:?}"
        );
        assert_eq!((result.text_len, result.text_required), (0, 0));
        assert!(
            memory.writer_reads.get() <= 4,
            "the walker must never retry"
        );
    }

    #[test]
    fn lexical_copy_rejects_a_writer_at_each_witness_edge_without_retrying() {
        for edge in 1..=4 {
            let mut memory = witness_memory();
            memory.busy_on = Some(edge);
            expect_unavailable(&memory);
            assert_eq!(memory.copied.get(), edge > 2);
        }
        let mut memory = witness_memory();
        set_count(&mut memory.before, WRITER_FLAG_RVA, 1);
        expect_unavailable(&memory);
        assert!(!memory.copied.get());
    }

    #[test]
    fn lexical_copy_rejects_an_insert_completed_between_writer_checks() {
        let mut memory = witness_memory();
        set_count(&mut memory.after, MAX_TOKEN_ID_RVA, 2);
        set_count(&mut memory.after, DYNAMIC_COUNT_RVA, 1);
        set_count(&mut memory.after, DYNAMIC_CAPACITY_RVA, 1);
        set_count(&mut memory.after, TOKEN_ALLOCATION_RVA, 3);
        set_count(&mut memory.after, TOKEN_CAPACITY_RVA, 3);
        memory.after[16..24].copy_from_slice(&0x9000_u64.to_le_bytes());
        expect_unavailable(&memory);
        assert_eq!(memory.writer_reads.get(), 4);
    }

    #[test]
    fn lexical_copy_rejects_count_before_reverse_publication_and_rebuilt_storage() {
        let mut memory = witness_memory();
        set_count(&mut memory.after, DYNAMIC_COUNT_RVA, 1);
        set_count(&mut memory.after, DYNAMIC_CAPACITY_RVA, 1);
        memory.after[16..24].copy_from_slice(&0x9000_u64.to_le_bytes());
        expect_unavailable(&memory);

        let mut memory = witness_memory();
        set_count(&mut memory.after, TOKEN_ALLOCATION_RVA, 4002);
        set_count(&mut memory.after, TOKEN_CAPACITY_RVA, 4002);
        expect_unavailable(&memory);

        let mut memory = witness_memory();
        memory.after[48..56].copy_from_slice(&0x2000_u64.to_le_bytes());
        expect_unavailable(&memory);
    }

    #[test]
    fn lexical_copy_rejects_uninitialized_invalid_and_unreadable_storage() {
        let mut memory = witness_memory();
        memory.before[0] = 0;
        expect_unavailable(&memory);
        let mut memory = witness_memory();
        memory.after[0] = 0;
        expect_unavailable(&memory);
        for (field, value) in [
            (DYNAMIC_COUNT_RVA, -1),
            (MAX_TOKEN_ID_RVA, i32::MAX),
            (TOKEN_CAPACITY_RVA, 1),
            (TOKEN_ALLOCATION_RVA, 1),
        ] {
            let mut memory = witness_memory();
            set_count(&mut memory.before, field, value);
            expect_unavailable(&memory);
            assert!(!memory.copied.get());
        }
        let mut memory = witness_memory();
        memory.unreadable_text = true;
        expect_unavailable(&memory);
    }

    #[test]
    fn lexical_copy_reports_only_stable_prefixes_and_exact_growth_requirements() {
        let mut rows = [Row::default(); 2];
        let mut text = [0; 1];
        let result = walk(&witness_memory(), 0, &mut rows, &mut text);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!((result.text_len, result.text_required), (1, 3));
        assert_eq!(
            rows[0],
            Row {
                token: 0,
                text_start: 0,
                text_len: 1
            }
        );
        assert_eq!(text, *b"x");

        let result = walk(&witness_memory(), 0, &mut [], &mut text);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!((result.text_len, result.text_required), (0, 0));

        let mut text = [0; 3];
        let result = walk(&witness_memory(), 0, &mut rows, &mut text);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(text, *b"xyz");
    }

    #[test]
    fn lexical_dictionary_preserves_bytes_and_has_no_lossy_capacity_path() {
        let mut bytes = [0_u8; 128];
        bytes[..4].copy_from_slice(&[b'x', 0, 255, b'y']);
        bytes[16..24].copy_from_slice(&4_u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&0x1060_u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&17_u64.to_le_bytes());
        bytes[56..64].copy_from_slice(&31_u64.to_le_bytes());
        bytes[96..113].copy_from_slice(b"a_long_token_name");
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let dictionary = TokenNames {
            table: 0x1000,
            maximum: 1,
        };
        let first = dictionary.text(&memory, 0).unwrap();
        let mut output = [99_u8; 32];
        assert!(first.copy(&memory, &mut output[..3]).is_err());
        assert_eq!(output, [99; 32]);
        first.copy(&memory, &mut output).unwrap();
        assert_eq!(&output[..4], &[b'x', 0, 255, b'y']);
        let second = dictionary.text(&memory, 1).unwrap();
        assert_eq!(second.len(), 17);
        second.copy(&memory, &mut output).unwrap();
        assert_eq!(&output[..17], b"a_long_token_name");
        assert!(dictionary.text(&memory, 2).is_err());
    }

    #[test]
    fn lexical_dictionary_rejects_corrupt_headers_and_missing_terminators() {
        let mut bytes = [0_u8; 32];
        let dictionary = TokenNames {
            table: 0x1000,
            maximum: 0,
        };
        assert!(
            dictionary
                .text(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0
                )
                .is_err()
        );
        bytes[16..24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[..4].copy_from_slice(b"fail");
        assert!(
            dictionary
                .text(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0
                )
                .is_err()
        );
        bytes[3] = 0;
        assert_eq!(
            dictionary
                .text(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0
                )
                .unwrap()
                .len(),
            3
        );
    }
}
