use crate::{Memory, cpdx_array::Array, read};

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

/// Both game arrays reserve slot zero. Null holes contain no row. A fault never
/// turns a prefix into a successful table, and capacity never causes allocation.
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
    for index in 1..array.count {
        let Some(address) = array.element(index, 8) else {
            return Outcome::Unavailable {
                reason: "table address overflow",
            };
        };
        let Some(pointer) = read::<u64>(memory, address).and_then(|p| usize::try_from(p).ok())
        else {
            return Outcome::Unavailable {
                reason: "table pointer unreadable",
            };
        };
        if pointer == 0 {
            continue;
        }
        if len == output.len() {
            unread_rows += 1;
            continue;
        }
        match read_row(index, pointer) {
            Ok(row) => {
                output[len] = row;
                len += 1;
            }
            Err(reason) => return Outcome::Unavailable { reason },
        }
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
