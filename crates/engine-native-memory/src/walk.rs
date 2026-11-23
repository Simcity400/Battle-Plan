use crate::{Memory, cpdx_array::Array};

pub enum Buffer<'a, T> {
    Count,
    Exact(&'a mut [T]),
    Growing(&'a mut Vec<T>),
    GrowingFrom(&'a mut Vec<T>, usize),
}

impl<T: Default> Buffer<'_, T> {
    pub fn ensure(&mut self, required: usize) -> Result<bool, &'static str> {
        if required <= self.len() {
            return Ok(true);
        }
        let (values, start) = match self {
            Self::Count | Self::Exact(_) => return Ok(false),
            Self::Growing(values) => (values, 0),
            Self::GrowingFrom(values, start) => (values, *start),
        };
        let end = start
            .checked_add(required)
            .ok_or("capture buffer length overflow")?;
        grow(values, end)?;
        Ok(true)
    }
}

pub fn grow<T: Default>(values: &mut Vec<T>, end: usize) -> Result<(), &'static str> {
    if end <= values.len() {
        return Ok(());
    }
    let len = values.len().saturating_mul(2).max(end);
    engine_allocation::resize_default(values, len).map_err(|_| "capture buffer allocation failed")
}

impl<T> Buffer<'_, T> {
    pub fn is_growing(&self) -> bool {
        !matches!(self, Self::Count | Self::Exact(_))
    }

    pub fn suffix(&mut self, start: usize) -> Buffer<'_, T> {
        match self {
            Self::Count => Buffer::Count,
            Self::Exact(values) => Buffer::Exact(&mut values[start..]),
            Self::Growing(values) => Buffer::GrowingFrom(values, start),
            Self::GrowingFrom(values, offset) => Buffer::GrowingFrom(values, *offset + start),
        }
    }
}

impl<T> std::ops::Deref for Buffer<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Self::Count => &[],
            Self::Exact(values) => values,
            Self::Growing(values) => values,
            Self::GrowingFrom(values, start) => &values[*start..],
        }
    }
}

impl<T> std::ops::DerefMut for Buffer<'_, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Count => &mut [],
            Self::Exact(values) => values,
            Self::Growing(values) => values,
            Self::GrowingFrom(values, start) => &mut values[*start..],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Rows {
        len: usize,
    },
    Truncated {
        len: usize,
        unread_rows: u32,
    },
    Unavailable {
        reason: &'static str,
    },
}

impl Outcome {
    pub fn len(self) -> usize {
        match self {
            Self::Rows { len } | Self::Truncated { len, .. } => len,
            Self::Unavailable { .. } => 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

pub fn pointers<R: Default>(
    memory: &impl Memory,
    array: Array,
    mut output: Buffer<'_, R>,
    mut read_row: impl FnMut(u32, usize) -> Result<R, &'static str>,
) -> Outcome {
    if array.count == 0 {
        return Outcome::Unavailable {
            reason: "table has no sentinel slot",
        };
    }
    if let Err(reason) = output.ensure(array.count.saturating_sub(1) as usize) {
        return Outcome::Unavailable { reason };
    }
    let mut len = 0;
    let mut unread_rows = 0;
    let source = match array.slice(1, array.count - 1, 8) {
        Ok(source) => source,
        Err(_) => {
            return Outcome::Unavailable {
                reason: "table address overflow",
            };
        }
    };
    let result = crate::cpdx_array::pointers(
        memory,
        source,
        "table pointer unreadable",
        |index, pointer| {
            if pointer == 0 {
                return Ok(());
            }
            if len == output.len() {
                unread_rows += 1;
                return Ok(());
            }
            output[len] = read_row(index + 1, pointer)?;
            len += 1;
            Ok(())
        },
    );
    if let Err(reason) = result {
        return Outcome::Unavailable { reason };
    }
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_worker_suffixes_grow_first_and_later_demands_without_losing_prefix() {
        let mut values = Vec::<u32>::new();
        let mut buffer = Buffer::Growing(&mut values);
        for countries in [15, 50] {
            buffer.ensure(countries).unwrap();
            buffer[..countries].fill(7);
            let mut tail = buffer.suffix(countries);
            let mut nested = tail.suffix(0);
            nested.ensure(countries * 19).unwrap();
            nested[..countries * 19].fill(13);
            assert_eq!(&buffer[..countries], vec![7; countries]);
            assert_eq!(buffer[countries..countries * 20], vec![13; countries * 19]);
        }
        assert!(!Buffer::<u32>::Count.suffix(1).ensure(5000).unwrap());
    }

    #[test]
    fn growing_pointer_census_uses_original_array_once_at_mod_size() {
        let bytes = pointer_bytes(5001);
        let memory = TracedMemory {
            bytes: &bytes,
            blocked: None,
            reads: RefCell::new(Vec::new()),
        };
        let mut rows = Vec::new();
        let result = pointers(
            &memory,
            Array {
                data: 0x1000,
                count: 5001,
            },
            Buffer::Growing(&mut rows),
            |slot, _| Ok(slot),
        );
        let expected: Vec<_> = (1..5001).filter(|slot| slot % 3 != 0).collect();
        assert_eq!(
            result,
            Outcome::Rows {
                len: expected.len()
            }
        );
        assert_eq!(&rows[..result.len()], expected);
        assert_eq!(memory.reads.borrow().len(), 5000);
    }

    use crate::memory::SliceMemory;
    use std::cell::{Cell, RefCell};

    struct TracedMemory<'a> {
        bytes: &'a [u8],
        blocked: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for TracedMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .blocked
                .is_some_and(|blocked| (address..address + destination.len()).contains(&blocked))
            {
                destination.fill(0xff);
                return false;
            }
            SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    struct CountBatches<M> {
        memory: M,
        batches: Cell<usize>,
    }

    impl<M: Memory> Memory for CountBatches<M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.memory.copy(address, destination)
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.batches.set(self.batches.get() + 1);
            self.memory.copy_scattered(addresses, destinations)
        }
    }

    fn pointer_bytes(count: u32) -> Vec<u8> {
        (0..count)
            .flat_map(|slot| {
                if slot == 0 {
                    u64::MAX
                } else if slot % 3 == 0 {
                    0
                } else {
                    0x2000_u64 + u64::from(slot)
                }
                .to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn batches_read_each_pointer_once_and_skip_the_sentinel_and_short_tail() {
        let bytes = pointer_bytes(132);
        let memory = CountBatches {
            memory: TracedMemory {
                bytes: &bytes,
                blocked: None,
                reads: RefCell::new(Vec::new()),
            },
            batches: Cell::new(0),
        };
        let mut rows = Vec::new();
        let expected: Vec<u32> = (1..132).filter(|slot| slot % 3 != 0).collect();
        assert_eq!(
            pointers(
                &memory,
                Array {
                    data: 0x1000,
                    count: 132,
                },
                Buffer::Growing(&mut rows),
                |slot, pointer| {
                    assert_eq!(pointer, 0x2000 + slot as usize);
                    Ok(slot)
                },
            ),
            Outcome::Rows {
                len: expected.len()
            }
        );
        assert_eq!(&rows[..expected.len()], expected);
        assert_eq!(memory.batches.get(), 3);
        assert_eq!(
            *memory.memory.reads.borrow(),
            (1..132)
                .map(|slot| (0x1000 + slot * 8, 8))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pointer_faults_keep_the_complete_prefix_and_earlier_row_failure() {
        let bytes = pointer_bytes(132);
        for failed_slot in [2, 64, 65, 131] {
            let memory = TracedMemory {
                bytes: &bytes,
                blocked: Some(0x1000 + failed_slot * 8 + 3),
                reads: RefCell::new(Vec::new()),
            };
            let mut visited = Vec::new();
            assert_eq!(
                pointers(
                    &memory,
                    Array {
                        data: 0x1000,
                        count: 132,
                    },
                    Buffer::Growing(&mut Vec::<u32>::new()),
                    |slot, _| {
                        visited.push(slot);
                        Ok(slot)
                    },
                ),
                Outcome::Unavailable {
                    reason: "table pointer unreadable"
                }
            );
            assert_eq!(
                visited,
                (1..failed_slot as u32)
                    .filter(|slot| slot % 3 != 0)
                    .collect::<Vec<_>>()
            );
            assert!(
                memory
                    .reads
                    .borrow()
                    .iter()
                    .all(|&(address, _)| address <= 0x1000 + failed_slot * 8)
            );
            assert_eq!(
                pointers(
                    &memory,
                    Array {
                        data: 0x1000,
                        count: 132,
                    },
                    Buffer::Growing(&mut Vec::<u32>::new()),
                    |_, _| Err("earlier row invalid"),
                ),
                Outcome::Unavailable {
                    reason: "earlier row invalid"
                }
            );
        }
    }

    #[test]
    fn a_count_probe_counts_holes_across_batches_without_reading_rows() {
        let bytes = pointer_bytes(132);
        let memory = TracedMemory {
            bytes: &bytes,
            blocked: None,
            reads: RefCell::new(Vec::new()),
        };
        assert_eq!(
            pointers(
                &memory,
                Array {
                    data: 0x1000,
                    count: 132,
                },
                Buffer::<u32>::Count,
                |_, _| panic!("a count probe must not read row fields"),
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 88,
            }
        );
        assert_eq!(memory.reads.borrow().len(), 131);
        let memory = TracedMemory {
            bytes: &bytes,
            blocked: Some(0x1000 + 130 * 8),
            reads: RefCell::new(Vec::new()),
        };
        assert_eq!(
            pointers(
                &memory,
                Array {
                    data: 0x1000,
                    count: 132,
                },
                Buffer::<u32>::Count,
                |_, _| panic!("a count probe must not read row fields"),
            ),
            Outcome::Unavailable {
                reason: "table pointer unreadable"
            }
        );
    }

    #[test]
    fn later_address_overflow_does_not_hide_earlier_pointer_or_row_failure() {
        struct EdgeMemory;
        impl Memory for EdgeMemory {
            fn copy(&self, _: usize, destination: &mut [u8]) -> bool {
                destination.copy_from_slice(&0x2000_u64.to_le_bytes());
                true
            }
        }
        let array = Array {
            data: usize::MAX - 23,
            count: 4,
        };
        assert_eq!(
            pointers(
                &EdgeMemory,
                array,
                Buffer::Growing(&mut Vec::<u32>::new()),
                |id, _| Ok(id)
            ),
            Outcome::Unavailable {
                reason: "table pointer unreadable"
            }
        );
        assert_eq!(
            pointers(
                &EdgeMemory,
                array,
                Buffer::Growing(&mut Vec::<u32>::new()),
                |_, _| Err("earlier row invalid")
            ),
            Outcome::Unavailable {
                reason: "earlier row invalid"
            }
        );
        assert_eq!(
            pointers(
                &EdgeMemory,
                Array {
                    data: usize::MAX - 3,
                    count: 2,
                },
                Buffer::Growing(&mut Vec::<u32>::new()),
                |id, _| Ok(id),
            ),
            Outcome::Unavailable {
                reason: "table address overflow"
            }
        );
    }

    #[test]
    fn counts_holes_and_pointer_faults_have_distinct_outcomes() {
        let bytes: Vec<u8> = [0_u64, 0x2000, 0, 0x3000]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let array = Array {
            data: 0x1000,
            count: 4,
        };
        assert_eq!(
            pointers(&memory, array, Buffer::Count, |id: u32, _| Ok(id)),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        let mut output = Vec::new();
        assert_eq!(
            pointers(&memory, array, Buffer::Growing(&mut output), |id, _| Ok(id)),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(&output[..2], &[1, 3]);
        let broken = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..24],
        };
        assert_eq!(
            pointers(&broken, array, Buffer::Growing(&mut output), |id, _| Ok(id)),
            Outcome::Unavailable {
                reason: "table pointer unreadable"
            }
        );
    }
}
