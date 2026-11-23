
use crate::{Memory, read, read_slice};

pub const DATA: usize = 0;
pub const CAPACITY: usize = 8;
pub const COUNT: usize = 12;
pub const STRIDE: usize = 0x18;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Array {
    pub data: usize,
    pub count: u32,
}

impl Array {
    pub fn read(memory: &impl Memory, address: usize, stride: usize) -> Option<Self> {
        let header = read::<[u8; 16]>(memory, address)?;
        Self::from_header(header, stride)
    }

    pub fn from_header(header: [u8; 16], stride: usize) -> Option<Self> {
        let data =
            usize::try_from(u64::from_le_bytes(header[DATA..CAPACITY].try_into().ok()?)).ok()?;
        let capacity = u32::from_le_bytes(header[CAPACITY..COUNT].try_into().ok()?);
        let count = u32::from_le_bytes(header[COUNT..16].try_into().ok()?);
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

    pub fn slice(self, first: u32, count: u32, stride: usize) -> Result<Self, &'static str> {
        let end = first
            .checked_add(count)
            .filter(|end| *end <= self.count)
            .ok_or("array window outside its own count")?;
        Ok(Self {
            data: self
                .element(first, stride)
                .or_else(|| (first == end).then_some(self.data))
                .ok_or("array window address overflow")?,
            count,
        })
    }

    pub fn element(self, index: u32, stride: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        self.data.checked_add((index as usize).checked_mul(stride)?)
    }
}

pub fn copy_u32s(
    memory: &impl Memory,
    source: Array,
    output: &mut [u32],
    unreadable: &'static str,
) -> Result<(), &'static str> {
    if output.len() != source.count as usize {
        return Err(unreadable);
    }
    copy_u32_prefix(memory, source, output, unreadable)
}

pub fn copy_u32_prefix(
    memory: &impl Memory,
    source: Array,
    output: &mut [u32],
    unreadable: &'static str,
) -> Result<(), &'static str> {
    if output.len() > source.count as usize {
        return Err(unreadable);
    }
    const ELEMENTS: usize = 64;
    let mut bytes = [0; ELEMENTS * 4];
    for (batch, destination) in output.chunks_mut(ELEMENTS).enumerate() {
        let address = batch
            .checked_mul(bytes.len())
            .and_then(|offset| source.data.checked_add(offset))
            .ok_or(unreadable)?;
        if !read_slice(memory, address, &mut bytes[..destination.len() * 4]) {
            return Err(unreadable);
        }
        for (value, source) in destination.iter_mut().zip(bytes.chunks_exact(4)) {
            *value = u32::from_le_bytes(source.try_into().expect("four bytes"));
        }
    }
    Ok(())
}

pub fn pointers(
    memory: &impl Memory,
    array: Array,
    what: &'static str,
    mut each: impl FnMut(u32, usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    pointer_pages(memory, array, what, |first, page| {
        for (index, &pointer) in page.iter().enumerate() {
            each(first + index as u32, pointer)?;
        }
        Ok(())
    })
}

pub const POINTER_PAGE: usize = 64;

pub fn pointer_pages(
    memory: &impl Memory,
    array: Array,
    what: &'static str,
    mut each: impl FnMut(u32, &[usize]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    const BATCH: usize = POINTER_PAGE;
    let mut pointers = [[0; 8]; BATCH];
    let mut page = [0_usize; BATCH];
    for first in (0..array.count).step_by(BATCH) {
        let count = ((array.count - first) as usize).min(BATCH);
        let addresses: [usize; BATCH] = std::array::from_fn(|index| {
            if index < count {
                array.element(first + index as u32, 8).unwrap_or(0)
            } else {
                0
            }
        });
        let mut fields = pointers.iter_mut().enumerate();
        let copied = memory.copy_scattered(
            addresses,
            &mut std::array::from_fn(|_| {
                let (index, bytes) = fields.next().unwrap();
                &mut bytes[..if index < count { 8 } else { 0 }]
            }),
        );
        let copied = copied.min(count);
        for (pointer, bytes) in page.iter_mut().zip(&pointers[..copied]) {
            *pointer = usize::try_from(u64::from_le_bytes(*bytes)).map_err(|_| what)?;
        }
        each(first, &page[..copied])?;
        if copied < count {
            return Err(what);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn prepared_header_keeps_capacity_null_stride_and_overflow_checks() {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&2_u32.to_le_bytes());
        header[12..].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            Array::from_header(header, 16),
            Some(Array {
                data: 0x2000,
                count: 1
            })
        );
        assert!(Array::from_header(header, 0).is_none());
        header[12..].copy_from_slice(&3_u32.to_le_bytes());
        assert!(Array::from_header(header, 16).is_none());
        header[12..].copy_from_slice(&1_u32.to_le_bytes());
        header[..8].fill(0);
        assert!(Array::from_header(header, 16).is_none());
        header[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(Array::from_header(header, 16).is_none());
        header[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Array::from_header(header, 16).is_none());
    }

    #[test]
    fn complete_unaligned_headers_preserve_bounds_and_partial_headers_fail() {
        let mut bytes = [0; 17];
        bytes[1..9].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[9..13].copy_from_slice(&3_u32.to_le_bytes());
        bytes[13..17].copy_from_slice(&2_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            Array::read(&memory, 0x1001, 8),
            Some(Array {
                data: 0x2000,
                count: 2
            })
        );
        for end in 1..17 {
            let memory = SliceMemory {
                base: 0x1000,
                bytes: &bytes[..end],
            };
            assert!(Array::read(&memory, 0x1001, 8).is_none());
        }
        bytes.fill(0);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            Array::read(&memory, 0x1001, 8),
            Some(Array { data: 0, count: 0 })
        );
    }

    #[test]
    fn a_batch_window_stays_inside_its_own_count() {
        let array = Array {
            data: 0x2000,
            count: 5,
        };
        assert_eq!(
            array.slice(2, 3, 8),
            Ok(Array {
                data: 0x2010,
                count: 3
            })
        );
        assert_eq!(
            array.slice(5, 0, 8),
            Ok(Array {
                data: 0x2000,
                count: 0
            })
        );
        for (first, count) in [(3_u32, 3_u32), (6, 0), (0, 6), (u32::MAX, 1)] {
            assert!(array.slice(first, count, 8).is_err(), "{first} {count}");
        }
        assert!(
            Array {
                data: usize::MAX - 7,
                count: 2
            }
            .slice(1, 1, 8)
            .is_err()
        );
    }

    #[test]
    fn bulk_copies_cross_batch_boundaries_and_refuse_a_wrong_destination() {
        let values: Vec<u8> = (0..300_u32).flat_map(u32::to_le_bytes).collect();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &values,
        };
        let array = Array {
            data: 0x1000,
            count: 300,
        };
        let mut output = vec![0; 300];
        copy_u32s(&memory, array, &mut output, "unreadable").unwrap();
        assert_eq!(output, (0..300).collect::<Vec<u32>>());
        assert_eq!(output[63..66], [63, 64, 65]);
        assert_eq!(
            copy_u32s(&memory, array, &mut output[..299], "unreadable"),
            Err("unreadable")
        );
        let short = SliceMemory {
            base: 0x1000,
            bytes: &values[..values.len() - 1],
        };
        assert_eq!(
            copy_u32s(&short, array, &mut output, "unreadable"),
            Err("unreadable")
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
