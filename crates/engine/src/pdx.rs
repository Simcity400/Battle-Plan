use crate::{Memory, read, read_slice};

/// CPdxArray header, confirmed by old country_tag_table_authenticated and engine accessors.
pub const ARRAY_DATA: usize = 0;
/// CPdxArray allocated element capacity, u32 at +8.
pub const ARRAY_CAPACITY: usize = 8;
/// CPdxArray live element count, u32 at +12; not the allocated capacity.
pub const ARRAY_COUNT: usize = 12;
/// MSVC std::string character length, confirmed by old read_msvc_string_with_policy_in.
pub const STRING_LENGTH: usize = 16;
/// MSVC std::string capacity, confirmed by old read_msvc_string_with_policy_in.
pub const STRING_CAPACITY: usize = 24;
/// MSVC std::string inline storage capacity (excludes its terminator).
pub const STRING_INLINE_CAPACITY: u64 = 15;
/// MSVC std::string size, confirmed by the old country tag database's element stride.
pub const STRING_STRIDE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Array {
    pub data: usize,
    pub count: u32,
}

impl Array {
    pub fn read(memory: &impl Memory, address: usize, stride: usize) -> Option<Self> {
        let data = usize::try_from(read::<u64>(memory, address.checked_add(ARRAY_DATA)?)?).ok()?;
        let capacity = read::<u32>(memory, address.checked_add(ARRAY_CAPACITY)?)?;
        let count = read::<u32>(memory, address.checked_add(ARRAY_COUNT)?)?;
        if count > capacity
            || capacity > i32::MAX as u32
            || (count != 0 && data == 0)
            || stride == 0
        {
            return None;
        }
        data.checked_add((count as usize).checked_mul(stride)?)?;
        Some(Self { data, count })
    }

    pub fn element(self, index: u32, stride: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        self.data.checked_add((index as usize).checked_mul(stride)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<const N: usize> {
    pub bytes: [u8; N],
    pub len: usize,
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> Text<N> {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }
}

/// Copies the engine's bytes without rounding, truncating, or lossy text conversion.
pub fn read_text<const N: usize>(memory: &impl Memory, address: usize) -> Option<Text<N>> {
    let len = read::<u64>(memory, address.checked_add(STRING_LENGTH)?)?;
    let capacity = read::<u64>(memory, address.checked_add(STRING_CAPACITY)?)?;
    if len > capacity || len > N as u64 || capacity < STRING_INLINE_CAPACITY {
        return None;
    }
    let source = if capacity == STRING_INLINE_CAPACITY {
        address
    } else {
        usize::try_from(read::<u64>(memory, address)?).ok()?
    };
    let mut value = Text {
        len: len as usize,
        ..Text::default()
    };
    if read::<u8>(memory, source.checked_add(value.len)?)? != 0 {
        return None;
    }
    read_slice(memory, source, &mut value.bytes[..value.len]).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn inline_and_heap_strings_are_exact_and_never_silently_shortened() {
        let mut bytes = [0_u8; 96];
        bytes[..3].copy_from_slice(b"GER");
        bytes[16..24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&0x1040_u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&4_u64.to_le_bytes());
        bytes[56..64].copy_from_slice(&31_u64.to_le_bytes());
        bytes[64..68].copy_from_slice(&[65, 0, 255, 66]);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            read_text::<3>(&memory, 0x1000).unwrap().as_str(),
            Some("GER")
        );
        assert!(read_text::<2>(&memory, 0x1000).is_none());
        assert_eq!(
            read_text::<8>(&memory, 0x1020).unwrap().as_bytes(),
            &[65, 0, 255, 66]
        );
    }

    #[test]
    fn corrupt_array_header_is_not_a_successful_empty_table() {
        let mut bytes = [0_u8; 16];
        bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        assert!(
            Array::read(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                8
            )
            .is_none()
        );
        bytes[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[12..16].copy_from_slice(&3_u32.to_le_bytes());
        assert!(
            Array::read(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                8
            )
            .is_none()
        );
    }
}
