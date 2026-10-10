//! Read-only completion witness for a retained graphics task. Construction
//! installs wrapper identity and retains the item. Completion requires a
//! nonnull item with signed state exactly two. Execution publishes state one
//! before running the body and two after return using locked publication.
//! The owning idler retains the wrapper until main-thread wait/release, so
//! require that original authenticated main-thread owner loan. This reader
//! neither invokes completion/wait nor returns a game pointer.

use crate::{Memory, read};

/// Primary task-wrapper vtable.
pub const VTABLE_RVA: usize = 0x02b7_b428;
/// Retained work-item pointer.
pub const ITEM: usize = 0x10;
/// Signed i32 task state tested for exactly two.
pub const ITEM_STATE: usize = 0x58;

pub const WRAPPER_UNAVAILABLE: &str = "transport graphics task wrapper unavailable";
pub const WRAPPER_UNSUPPORTED: &str = "transport graphics task wrapper unsupported";
pub const ITEM_UNAVAILABLE: &str = "transport graphics task item unavailable";
pub const ITEM_MISSING: &str = "transport graphics task item missing";
pub const STATE_UNAVAILABLE: &str = "transport graphics task state unavailable";
pub const OUTSTANDING: &str = "transport graphics task outstanding";
pub const STATE_UNSUPPORTED: &str = "transport graphics task state unsupported";

/// One original completion predicate under an authenticated idler-owner loan.
pub fn read_completed(memory: &impl Memory, base: usize, task: usize) -> Result<(), &'static str> {
    if task == 0 {
        return Err(WRAPPER_UNAVAILABLE);
    }
    let vtable: u64 = read(memory, task).ok_or(WRAPPER_UNAVAILABLE)?;
    if Some(vtable) != base.checked_add(VTABLE_RVA).map(|value| value as u64) {
        return Err(WRAPPER_UNSUPPORTED);
    }
    let item: u64 = task
        .checked_add(ITEM)
        .and_then(|address| read(memory, address))
        .ok_or(ITEM_UNAVAILABLE)?;
    let item = usize::try_from(item).map_err(|_| ITEM_UNAVAILABLE)?;
    if item == 0 {
        return Err(ITEM_MISSING);
    }
    let state: i32 = item
        .checked_add(ITEM_STATE)
        .and_then(|address| read(memory, address))
        .ok_or(STATE_UNAVAILABLE)?;
    match state {
        2 => Ok(()),
        0 | 1 => Err(OUTSTANDING),
        _ => Err(STATE_UNSUPPORTED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fixture {
        wrapper: [u8; 0x60],
        item: [u8; 0x60],
        fail: Option<usize>,
        reads: RefCell<Vec<usize>>,
    }
    impl Fixture {
        fn new(state: i32) -> Self {
            let mut value = Self {
                wrapper: [0; 0x60],
                item: [0; 0x60],
                fail: None,
                reads: RefCell::new(Vec::new()),
            };
            value.wrapper[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
            value.wrapper[ITEM..ITEM + 8].copy_from_slice(&0x4000_u64.to_le_bytes());
            // A decoy completed word on the wrapper cannot replace the item.
            value.wrapper[ITEM_STATE..ITEM_STATE + 4].copy_from_slice(&2_i32.to_le_bytes());
            value.item[ITEM_STATE..ITEM_STATE + 4].copy_from_slice(&state.to_le_bytes());
            value
        }
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if self.fail == Some(address) {
                return false;
            }
            let (start, bytes): (usize, &[u8]) = if address >= 0x4000 {
                (0x4000, &self.item)
            } else {
                (0x3000, &self.wrapper)
            };
            let Some(offset) = address.checked_sub(start) else {
                return false;
            };
            let Some(end) = offset.checked_add(output.len()) else {
                return false;
            };
            let Some(source) = bytes.get(offset..end) else {
                return false;
            };
            output.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn transport_graphics_completion_requires_original_item_state_exactly_two() {
        for (state, expected) in [
            (0, Err(OUTSTANDING)),
            (1, Err(OUTSTANDING)),
            (2, Ok(())),
            (3, Err(STATE_UNSUPPORTED)),
            (-1, Err(STATE_UNSUPPORTED)),
        ] {
            let memory = Fixture::new(state);
            assert_eq!(read_completed(&memory, 0, 0x3000), expected);
            assert_eq!(*memory.reads.borrow(), [0x3000, 0x3010, 0x4058]);
        }
    }

    #[test]
    fn transport_graphics_completion_preserves_original_fault_and_identity() {
        for (address, reason, reads) in [
            (0x3000, WRAPPER_UNAVAILABLE, 1),
            (0x3010, ITEM_UNAVAILABLE, 2),
            (0x4058, STATE_UNAVAILABLE, 3),
        ] {
            let mut memory = Fixture::new(2);
            memory.fail = Some(address);
            assert_eq!(read_completed(&memory, 0, 0x3000), Err(reason));
            assert_eq!(memory.reads.borrow().len(), reads, "no fault retries");
        }
        let mut memory = Fixture::new(2);
        memory.wrapper[0] ^= 1;
        assert_eq!(read_completed(&memory, 0, 0x3000), Err(WRAPPER_UNSUPPORTED));
        assert_eq!(*memory.reads.borrow(), [0x3000]);
        let mut memory = Fixture::new(2);
        memory.wrapper[ITEM..ITEM + 8].fill(0);
        assert_eq!(read_completed(&memory, 0, 0x3000), Err(ITEM_MISSING));
        assert_eq!(*memory.reads.borrow(), [0x3000, 0x3010]);
    }
}
