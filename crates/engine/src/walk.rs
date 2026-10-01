use crate::{Memory, cpdx_array::Array};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Rows {
        len: usize,
    },
    /// Non-null rows whose fields were not read after capacity was reached.
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

/// These game arrays reserve slot zero. Null holes contain no row. A fault never
/// turns a prefix into a successful table, and capacity never causes allocation.
/// Exact pointer spans are copied in bounded batches. Earlier complete rows keep
/// their validation precedence over a later unreadable pointer in the batch.
pub(crate) fn pointers<R>(
    memory: &impl Memory,
    array: Array,
    output: &mut [R],
    mut read_row: impl FnMut(u32, usize) -> Result<R, &'static str>,
) -> Outcome {
    if array.count == 0 {
        return Outcome::Unavailable {
            reason: "table has no sentinel slot",
        };
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
                // No byte from a partial failed pointer may become a row.
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
        let mut rows = [0; 132];
        let expected: Vec<u32> = (1..132).filter(|slot| slot % 3 != 0).collect();
        assert_eq!(
            pointers(
                &memory,
                Array {
                    data: 0x1000,
                    count: 132,
                },
                &mut rows,
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
                    &mut [0; 132],
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
                    &mut [0; 132],
                    |_, _| Err("earlier row invalid"),
                ),
                Outcome::Unavailable {
                    reason: "earlier row invalid"
                }
            );
        }
    }

    #[test]
    fn capacity_counts_holes_across_batches_without_reading_more_rows() {
        let bytes = pointer_bytes(132);
        let memory = TracedMemory {
            bytes: &bytes,
            blocked: None,
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
                &mut [0; 2],
                |slot, _| {
                    visited.push(slot);
                    Ok(slot)
                },
            ),
            Outcome::Truncated {
                len: 2,
                unread_rows: 86,
            }
        );
        assert_eq!(visited, [1, 2]);
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
                &mut [0; 0],
                |_, _| panic!("a full output must not read row fields"),
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
            pointers(&EdgeMemory, array, &mut [0; 4], |id, _| Ok(id)),
            Outcome::Unavailable {
                reason: "table pointer unreadable"
            }
        );
        assert_eq!(
            pointers(&EdgeMemory, array, &mut [0; 4], |_, _| {
                Err("earlier row invalid")
            }),
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
                &mut [0; 2],
                |id, _| Ok(id),
            ),
            Outcome::Unavailable {
                reason: "table address overflow"
            }
        );
    }

    #[test]
    fn capacity_and_pointer_faults_have_distinct_outcomes() {
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
        let mut output = [0; 1];
        assert_eq!(
            pointers(&memory, array, &mut output, |id, _| Ok(id)),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        let mut output = [0; 4];
        assert_eq!(
            pointers(&memory, array, &mut output, |id, _| Ok(id)),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(&output[..2], &[1, 3]);
        let broken = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..24],
        };
        assert_eq!(
            pointers(&broken, array, &mut output, |id, _| Ok(id)),
            Outcome::Unavailable {
                reason: "table pointer unreadable"
            }
        );
    }
}
