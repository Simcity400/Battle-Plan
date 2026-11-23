
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

#[inline(always)]
pub fn fields(pointer: usize, fields: &[crate::memory::RecordField]) {
    for field in fields {
        span(pointer.wrapping_add(field.source_offset), field.len);
    }
}

const LINE: usize = 64;
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
        span(0, 0);
        span(0, 64);
        span(1, 4096);
        span(usize::MAX, 512);
        span(usize::MAX - 3, 8);
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
