//! Six shared political-hover globals, read once each per accepted hourly pass.
//! The caller authenticates the supported module and world/sample boundary.
//! These are simulation operands, independent of selection or hover caches.
use crate::{Memory, read};

pub const NAMES: [&str; 6] = [
    "river_small",
    "river_large",
    "night",
    "fort",
    "move_strength_threshold",
    "move_organisation_threshold",
];
// Loaded globals authenticated by their installed name bytes, with warning operands read as qwords.
pub const RVAS: [usize; 6] = [
    0x0333_62e0,
    0x0333_6390,
    0x0333_1178,
    0x0333_6760,
    0x0295_2fd0,
    0x0295_2fc8,
];
const UNREADABLE: [&str; 6] = [
    "river_small global unreadable",
    "river_large global unreadable",
    "night global unreadable",
    "fort global unreadable",
    "move_strength_threshold global unreadable",
    "move_organisation_threshold global unreadable",
];

#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub sample_hour: i32,
    pub values: [Result<i64, &'static str>; 6],
}
impl Default for Inputs {
    fn default() -> Self {
        Self {
            sample_hour: 0,
            values: std::array::from_fn(|index| Err(UNREADABLE[index])),
        }
    }
}

pub fn capture(memory: &impl Memory, base: usize, sample_hour: i32) -> Inputs {
    // Each independently named leaf is attempted once. A failed leaf does not
    // stop siblings; no retry, content default, persistent memo or game call.
    Inputs {
        sample_hour,
        values: std::array::from_fn(|index| {
            base.checked_add(RVAS[index])
                .and_then(|address| read(memory, address))
                .ok_or(UNREADABLE[index])
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Source {
        refuse: usize,
        calls: RefCell<Vec<usize>>,
    }
    impl Memory for Source {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(address);
            let Some(index) = RVAS.iter().position(|&rva| rva == address) else {
                return false;
            };
            if index == self.refuse || output.len() != 8 {
                return false;
            }
            output.copy_from_slice(&[0, i64::MIN, i64::MAX, -1, 1, 80_000][index].to_le_bytes());
            true
        }
    }

    #[test]
    fn independently_refused_globals_are_attempted_once_and_preserve_siblings() {
        for refuse in 0..6 {
            let source = Source {
                refuse,
                calls: RefCell::new(Vec::new()),
            };
            let result = capture(&source, 0, 17);
            assert_eq!(result.sample_hour, 17);
            assert_eq!(&*source.calls.borrow(), &RVAS);
            for (index, value) in result.values.iter().enumerate() {
                assert_eq!(value.is_err(), index == refuse);
            }
        }
    }

    #[test]
    fn address_overflow_refuses_each_leaf_without_memory_reads() {
        let source = Source {
            refuse: 6,
            calls: RefCell::new(Vec::new()),
        };
        let result = capture(&source, usize::MAX, 18);
        assert!(result.values.iter().all(Result::is_err));
        assert!(source.calls.borrow().is_empty());
    }
}
