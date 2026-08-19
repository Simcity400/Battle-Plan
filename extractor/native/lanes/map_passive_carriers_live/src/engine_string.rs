use crate::memory::{PassiveMemory, PassiveMemoryExt};
use crate::refusal::Refusal;

pub const STRING_SIZE_OFFSET: u64 = 0x10;
pub const STRING_CAPACITY_OFFSET: u64 = 0x18;
pub const STRING_INLINE_CAPACITY: u64 = 0xf;

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

pub fn read_engine_string<M: PassiveMemory + ?Sized>(
    memory: &M,
    addr: u64,
    max_bytes: u64,
) -> Result<String, Refusal> {
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
    if size > max_bytes {
        return Err(Refusal::new(
            "engine_string.size_exceeds_ceiling",
            format!("size {size} exceeds the fail-closed ceiling {max_bytes}"),
        ));
    }

    // how a reader acquires arms nothing tests.
    let length = usize::try_from(size).map_err(|_| {
        Refusal::new(
            "engine_string.size_exceeds_host_word",
            format!("size {size} does not fit this host's usize"),
        )
    })?;

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

    fn string_memory(size: u64, capacity: u64) -> SliceMemory {
        let mut memory = SliceMemory::new(0x1_4000_0000);
        memory.map_object(BASE, 0x20);
        memory.map_object(HEAP, 0x100);
        memory.put_u64(BASE + 0x10, size);
        memory.put_u64(BASE + 0x18, capacity);
        memory
    }

    #[test]
    fn an_inline_string_reads_from_the_object_itself() {
        let mut memory = string_memory(5, 0xf);
        memory.put_bytes(BASE, b"Alice");
        assert_eq!(read_engine_string(&memory, BASE, 65_536).unwrap(), "Alice");
    }

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

    #[test]
    fn torn_oversized_and_unreachable_headers_refuse_by_name() {
        assert_eq!(
            read_engine_string(&string_memory(40, 8), BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.torn_size_capacity"
        );
        let mut boundary = string_memory(0xf, 0xf);
        boundary.put_bytes(BASE, b"123456789012345");
        assert_eq!(
            read_engine_string(&boundary, BASE, 65_536).unwrap(),
            "123456789012345"
        );
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
        let mut short = string_memory(64, 128);
        short.put_u64(BASE, HEAP + 0x100 - 8);
        assert_eq!(
            read_engine_string(&short, BASE, 65_536).unwrap_err().reason,
            "engine_string.heap_buffer_unreadable"
        );
        assert_eq!(
            read_engine_string(&string_memory(1, 1), u64::MAX - 4, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.address_overflow"
        );
        let mut unbounded = string_memory(1 << 62, 1 << 62);
        unbounded.put_u64(BASE, HEAP);
        assert_eq!(
            read_engine_string(&unbounded, BASE, u64::MAX)
                .unwrap_err()
                .reason,
            "engine_string.allocation_refused"
        );
    }

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
            read_engine_string(&memory, BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.payload_changed_during_read"
        );
    }

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
            read_engine_string(&memory, BASE, 65_536)
                .unwrap_err()
                .reason,
            "engine_string.header_changed_during_read"
        );
    }

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
