//! The engine's string object, read passively.
//!
//! (`out[3] = 0xf`, `out[2] = 0`,
//! `*(u8*)out = 0`) — MSVC `std::string`:
//!
//! ```text
//! +0x00  16 bytes  inline buffer, OR a pointer to heap storage
//! +0x10  u64       size
//! +0x18  u64       capacity   (<= 0xf  =>  the data is inline)
//! ```
//!
//! # What this reader refuses, and why each check exists
//!
//! The size and capacity words are attacker-shaped input from the recorder's
//! point of view: they are engine memory read without a lock, so a torn write
//! can present any 64-bit pair. Every one of these is therefore a refusal and
//! not a clamp — a clamped string is a DIFFERENT string, and publishing it
//! would fabricate a display name.
//!
//! * `capacity < size` — impossible in MSVC's invariant, so the pair is torn.
//! * `capacity <= 0xf` (inline) but `size > 0xf` — the size claims more bytes
//!   than the inline buffer has.
//! * `size > max_bytes` — the caller's fail-closed ceiling. The consumer's own
//!   ceiling is 65,536 bytes (`MAX_TEXT_BYTES` in `xmap17_player_labels.rs`),
//!   so a longer string could never be admitted anyway; refusing here names it
//!   at the producer instead of shipping bytes that will be rejected.
//! * an interior NUL — the consumer rejects it
//!   (`name.as_bytes().contains(&0)`), and a producer that shipped one would
//!   have made the whole hour unavailable rather than one row.
//! * invalid UTF-8 — refused rather than lossily transcoded, because a
//!   replacement character is a different name.

use crate::memory::{PassiveMemory, PassiveMemoryExt};
use crate::refusal::Refusal;

/// `+0x10` — the size word.
pub const STRING_SIZE_OFFSET: u64 = 0x10;
/// `+0x18` — the capacity word.
pub const STRING_CAPACITY_OFFSET: u64 = 0x18;
/// The inline buffer holds this many bytes plus the terminator.
pub const STRING_INLINE_CAPACITY: u64 = 0xf;

/// Allocate `length` zero bytes, or refuse by name.
///
/// `vec![0u8; n]` aborts the process on allocation failure, which is the one
/// outcome this crate is built to make impossible: a torn size word must cost
/// one named refusal, not the recording. `try_reserve_exact` is the fallible
/// form, and it is the ONLY bound this reader imposes beyond the caller's
/// ceiling — an invented internal cap would be an engine-structural claim with
/// no evidence behind it, and the callers already carry the consumer's proven
/// 65,536-byte limit.
fn try_zeroed(length: usize, what: &str) -> Result<Vec<u8>, Refusal> {
    let mut buf: Vec<u8> = Vec::new();
    buf.try_reserve_exact(length).map_err(|error| {
        Refusal::new(
            "engine_string.allocation_refused",
            format!("could not reserve {length} bytes for the {what} buffer: {error}"),
        )
    })?;
    buf.resize(length, 0);
    Ok(buf)
}

/// Read the MSVC `std::string` at `addr`.
///
/// `max_bytes` is the caller's fail-closed ceiling on the size word.
pub fn read_engine_string<M: PassiveMemory + ?Sized>(
    memory: &M,
    addr: u64,
    max_bytes: u64,
) -> Result<String, Refusal> {
    // Checked, because `addr` is engine-supplied: a torn pointer near the top
    // of the address space would otherwise wrap and read a low address.
    let size_at = addr.checked_add(STRING_SIZE_OFFSET).ok_or_else(|| {
        Refusal::new(
            "engine_string.address_overflow",
            format!("{addr:#x} + the size offset overflowed"),
        )
    })?;
    let capacity_at = addr.checked_add(STRING_CAPACITY_OFFSET).ok_or_else(|| {
        Refusal::new(
            "engine_string.address_overflow",
            format!("{addr:#x} + the capacity offset overflowed"),
        )
    })?;
    let size = memory.read_u64(size_at).ok_or_else(|| {
        Refusal::new(
            "engine_string.size_unreadable",
            format!("the size word at {size_at:#x} could not be read"),
        )
    })?;
    let capacity = memory.read_u64(capacity_at).ok_or_else(|| {
        Refusal::new(
            "engine_string.capacity_unreadable",
            format!("the capacity word at {capacity_at:#x} could not be read"),
        )
    })?;

    if capacity < size {
        return Err(Refusal::new(
            "engine_string.torn_size_capacity",
            format!("capacity {capacity} is below size {size}; the pair is torn"),
        ));
    }
    // The caller's ceiling is the only size bound. A caller may pass
    // `u64::MAX`; a torn header under such a caller is caught by the fallible
    // allocation below rather than by a number invented here.
    if size > max_bytes {
        return Err(Refusal::new(
            "engine_string.size_exceeds_ceiling",
            format!("size {size} exceeds the fail-closed ceiling {max_bytes}"),
        ));
    }

    // No separate "inline size overruns the buffer" arm: `capacity >= size`
    // has already been established, so `capacity <= 0xf` implies `size <= 0xf`.
    // A defensive check here would be unreachable, and unreachable checks are
    // how a reader acquires arms nothing tests.
    let length = usize::try_from(size).map_err(|_| {
        Refusal::new(
            "engine_string.size_exceeds_host_word",
            format!("size {size} does not fit this host's usize"),
        )
    })?;

    // Two full copies, and the header re-read between them.
    //
    // Re-reading only size/capacity is not enough: a same-size, same-capacity
    // reallocation or an in-place rewrite satisfies that check while the bytes
    // came from two different strings. The heap POINTER is therefore part of
    // the snapshot, and the payload is copied twice and compared. Equal twice
    // with an unchanged header and an unchanged pointer is the strongest
    // statement a lock-free reader can make; anything weaker publishes a
    // hybrid.
    let read_snapshot = |pass: &str| -> Result<(Option<u64>, Vec<u8>), Refusal> {
        if capacity <= STRING_INLINE_CAPACITY {
            let mut buf = try_zeroed(length, "inline")?;
            if !memory.read(addr, &mut buf) {
                return Err(Refusal::new(
                    "engine_string.inline_buffer_unreadable",
                    format!("the inline buffer at {addr:#x} could not be read on the {pass} pass"),
                ));
            }
            Ok((None, buf))
        } else {
            let heap = memory.read_pointer(addr).ok_or_else(|| {
                Refusal::new(
                    "engine_string.heap_pointer_unavailable",
                    format!("the heap pointer at {addr:#x} is null, misaligned, or unreadable"),
                )
            })?;
            let mut buf = try_zeroed(length, "heap")?;
            if !memory.read(heap, &mut buf) {
                return Err(Refusal::new(
                    "engine_string.heap_buffer_unreadable",
                    format!("{size} bytes at {heap:#x} could not be read on the {pass} pass"),
                ));
            }
            Ok((Some(heap), buf))
        }
    };

    let (first_heap, bytes) = read_snapshot("first")?;

    if memory.read_u64(size_at) != Some(size) || memory.read_u64(capacity_at) != Some(capacity) {
        return Err(Refusal::new(
            "engine_string.header_changed_during_read",
            "the size/capacity pair changed while the buffer was being copied",
        ));
    }

    let (second_heap, second_bytes) = read_snapshot("second")?;
    if first_heap != second_heap {
        return Err(Refusal::new(
            "engine_string.storage_moved_during_read",
            "the string's heap pointer changed between the two copies",
        ));
    }
    if bytes != second_bytes {
        return Err(Refusal::new(
            "engine_string.payload_changed_during_read",
            "the string's bytes changed between the two copies",
        ));
    }

    if bytes.contains(&0) {
        return Err(Refusal::new(
            "engine_string.interior_nul",
            "the string carries an interior NUL, which the replay consumer rejects",
        ));
    }
    String::from_utf8(bytes).map_err(|error| {
        Refusal::new(
            "engine_string.not_utf8",
            format!("the string is not valid UTF-8: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: u64 = 0x3_0000_0000;
    const HEAP: u64 = 0x3_0001_0000;

    /// The header goes in at LITERAL `0x10`/`0x18`, not through the production
    /// constants: a fixture written through `STRING_SIZE_OFFSET` moves with the
    /// constant, so a consistent wrong edit would move reader and fixture
    /// together and every test here would still pass.
    fn string_memory(size: u64, capacity: u64) -> SliceMemory {
        let mut memory = SliceMemory::new(0x1_4000_0000);
        memory.map_object(BASE, 0x20);
        memory.map_object(HEAP, 0x100);
        memory.put_u64(BASE + 0x10, size);
        memory.put_u64(BASE + 0x18, capacity);
        memory
    }

    /// kills: moving the size or capacity word off `+0x10`/`+0x18`, or reading
    /// the inline buffer from anywhere but the object's own `+0x00`. The
    /// negative tests below all still pass under a consistently wrong layout;
    /// only the positive pair pins it.
    #[test]
    fn an_inline_string_reads_from_the_object_itself() {
        let mut memory = string_memory(5, 0xf);
        memory.put_bytes(BASE, b"Alice");
        assert_eq!(read_engine_string(&memory, BASE, 65_536).unwrap(), "Alice");
    }

    /// kills: taking the inline branch when `capacity > 0xf`, i.e. reading 20
    /// bytes of pointer-and-header as if they were the string.
    #[test]
    fn a_heap_string_follows_the_pointer() {
        let mut memory = string_memory(20, 31);
        memory.put_u64(BASE, HEAP);
        memory.put_bytes(HEAP, b"a-twenty-byte-string");
        assert_eq!(
            read_engine_string(&memory, BASE, 65_536).unwrap(),
            "a-twenty-byte-string"
        );
    }

    /// kills: clamping instead of refusing on a torn size/capacity pair;
    /// dropping the size ceiling (a torn header would then drive an enormous
    /// allocation and read); and following a null heap pointer.
    #[test]
    fn torn_oversized_and_unreachable_headers_refuse_by_name() {
        assert_eq!(
            read_engine_string(&string_memory(40, 8), BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.torn_size_capacity"
        );
        // The inline boundary itself is admitted, not refused.
        let mut boundary = string_memory(0xf, 0xf);
        boundary.put_bytes(BASE, b"123456789012345");
        assert_eq!(
            read_engine_string(&boundary, BASE, 65_536).unwrap(),
            "123456789012345"
        );
        // capacity 0x10 > 0xf selects the heap arm; its pointer is null here.
        assert_eq!(
            read_engine_string(&string_memory(0x10, 0x10), BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.heap_pointer_unavailable"
        );
        assert_eq!(
            read_engine_string(&string_memory(1 << 40, 1 << 40), BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.size_exceeds_ceiling"
        );
        // A size the ceiling admits but the mapping does not still refuses,
        // rather than returning the prefix it could read.
        let mut short = string_memory(64, 128);
        short.put_u64(BASE, HEAP + 0x100 - 8);
        assert_eq!(
            read_engine_string(&short, BASE, 65_536).unwrap_err().reason,
            "engine_string.heap_buffer_unreadable"
        );
        // A base near the top of the address space wraps rather than reading a
        // low address.
        assert_eq!(
            read_engine_string(&string_memory(1, 1), u64::MAX - 4, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.address_overflow"
        );
        // A caller that passes an unbounded ceiling gets a NAMED refusal from
        // the fallible allocation rather than a process abort.
        //
        // The heap pointer is deliberately VALID here: the heap arm reads the
        // pointer before it allocates, so a fixture that left it null would
        // refuse with `heap_pointer_unavailable` and this assertion would be
        // about the wrong arm — which is exactly how it failed on the first
        // gate run. 2^62 bytes is past the 128 TiB user address space of any
        // 64-bit host, so the reservation cannot succeed on a large machine
        // either; the refusal does not depend on this box's RAM or pagefile.
        let mut unbounded = string_memory(1 << 62, 1 << 62);
        unbounded.put_u64(BASE, HEAP);
        assert_eq!(
            read_engine_string(&unbounded, BASE, u64::MAX)
                .unwrap_err()
                .reason,
            "engine_string.allocation_refused"
        );
    }

    /// kills: dropping the second copy or the heap-pointer half of the
    /// snapshot. A same-size, same-capacity rewrite satisfies the header
    /// re-read on its own, so only the payload comparison catches it.
    #[test]
    fn a_payload_that_changes_under_an_unchanged_header_refuses() {
        use std::cell::Cell;
        struct RewritesPayload {
            inner: SliceMemory,
            reads: Cell<u32>,
        }
        impl PassiveMemory for RewritesPayload {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == BASE && out.len() == 5 {
                    let pass = self.reads.replace(self.reads.get() + 1);
                    out.copy_from_slice(if pass == 0 { b"Alice" } else { b"Alic3" });
                    return true;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let memory = RewritesPayload {
            inner: string_memory(5, 0xf),
            reads: Cell::new(0),
        };
        assert_eq!(
            read_engine_string(&memory, BASE, 65_536).unwrap_err().reason,
            "engine_string.payload_changed_during_read"
        );
    }

    /// kills: dropping the header re-read, which would publish `size` bytes of
    /// a buffer that was reallocated to a different length mid-copy.
    #[test]
    fn a_header_that_changes_during_the_copy_refuses() {
        use std::cell::Cell;
        struct ShrinkingHeader {
            inner: SliceMemory,
            reads: Cell<u32>,
        }
        impl PassiveMemory for ShrinkingHeader {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == BASE + 0x10 && out.len() == 8 {
                    let count = self.reads.get();
                    self.reads.set(count + 1);
                    let size: u64 = if count == 0 { 5 } else { 4 };
                    out.copy_from_slice(&size.to_le_bytes());
                    return true;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let mut inner = string_memory(5, 0xf);
        inner.put_bytes(BASE, b"Alice");
        let memory = ShrinkingHeader {
            inner,
            reads: Cell::new(0),
        };
        assert_eq!(
            read_engine_string(&memory, BASE, 65_536).unwrap_err().reason,
            "engine_string.header_changed_during_read"
        );
    }

    /// kills: shipping a string the replay consumer would reject, turning one
    /// bad row into an unavailable hour.
    #[test]
    fn interior_nul_and_invalid_utf8_refuse_at_the_producer() {
        let mut nul = string_memory(3, 0xf);
        nul.put_bytes(BASE, &[b'a', 0, b'b']);
        assert_eq!(
            read_engine_string(&nul, BASE, 65_536).unwrap_err().reason,
            "engine_string.interior_nul"
        );
        let mut bad = string_memory(2, 0xf);
        bad.put_bytes(BASE, &[0xff, 0xfe]);
        assert_eq!(
            read_engine_string(&bad, BASE, 65_536).unwrap_err().reason,
            "engine_string.not_utf8"
        );
    }
}
