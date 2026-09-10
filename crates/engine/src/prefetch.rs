//! Cache hints for reads the walker has already decided to make.
//!
//! A hint is not a read. `PREFETCHh` observes no value, writes nothing, faults
//! on no address and raises no exception, so an unmapped, misaligned or wholly
//! invalid pointer is architecturally a no-op. Hints therefore cannot change a
//! walk's rows, validators, error precedence or failure semantics; they only
//! let the guarded copies that follow overlap their cache misses instead of
//! serialising them. Nothing here allocates, calls the engine, retries, or
//! keeps state between calls.

/// Hints the cache lines covering `len` bytes at `address`. The last step
/// lands on the final byte, which can sit in a later line than the stride
/// reached. A span longer than `LINE * LIMIT` is hinted at its first `LIMIT`
/// lines and its final byte rather than line by line; hinting is advisory, and
/// this keeps the walk bounded and overflow-free for every length.
#[inline(always)]
pub fn span(address: usize, len: usize) {
    let Some(last) = len.checked_sub(1) else {
        return;
    };
    let mut offset = 0;
    for _ in 0..LIMIT {
        line(address.wrapping_add(offset));
        if offset >= last {
            return;
        }
        offset = offset.saturating_add(LINE).min(last);
    }
    line(address.wrapping_add(last));
}

/// Hints the lines covering each `(offset, len)` field of one object.
#[inline(always)]
pub fn fields(pointer: usize, fields: &[crate::memory::RecordField]) {
    for field in fields {
        span(pointer.wrapping_add(field.source_offset), field.len);
    }
}

/// The pinned target's cache line. Only the hint granularity depends on it.
const LINE: usize = 64;
/// Line budget for one span. Every field this crate hints is far smaller.
const LIMIT: usize = 16;

#[inline(always)]
fn line(address: usize) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: `_mm_prefetch` never dereferences, never faults and never
    // observes a value, so an arbitrary integer address is sound and inert.
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(address as *const i8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = address;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::RecordField;

    #[test]
    fn hints_are_inert_for_null_unaligned_and_overflowing_addresses() {
        // Every one of these would fault a guarded copy; a hint must not.
        span(0, 0);
        span(0, 64);
        span(1, 4096);
        span(usize::MAX, 512);
        span(usize::MAX - 3, 8);
        // A length that would overflow a naive stride walk still terminates.
        span(0x1000, usize::MAX);
        fields(
            usize::MAX - 7,
            &[
                RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: 0x1c0,
                    destination_offset: 0,
                    len: 8,
                },
            ],
        );
        let bytes = [7_u8; 256];
        span(bytes.as_ptr() as usize, bytes.len());
        assert_eq!(bytes[255], 7);
    }
}
