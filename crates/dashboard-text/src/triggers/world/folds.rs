
use crate::triggers::{Failure, refusal::refused};
use replay_data::rows::LexicalTokenRow;
use std::collections::BTreeMap;

const EMPTY: u32 = u32::MAX;

#[cfg(test)]
thread_local! {
    pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy)]
struct Slot {
    hash: u32,
    row: u32,
}

pub(super) struct Folds {
    slots: Box<[Slot]>,
    shift: u32,
    ambiguous: BTreeMap<u32, u32>,
}

fn several(literal: &[u8]) -> Failure {
    refused(format!(
        "{} names several lexer tokens",
        String::from_utf8_lossy(literal)
    ))
}

fn hash(literal: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in literal {
        hash = (hash ^ u64::from(byte.to_ascii_lowercase())).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl Folds {
    pub(super) fn read(rows: &[LexicalTokenRow]) -> Self {
        #[cfg(test)]
        BUILDS.with(|builds| builds.set(builds.get() + 1));
        let slots = (rows.len() * 2).max(2).next_power_of_two();
        let mut folds = Self {
            slots: vec![Slot { hash: 0, row: EMPTY }; slots].into_boxed_slice(),
            shift: 64 - slots.trailing_zeros(),
            ambiguous: BTreeMap::new(),
        };
        for (at, row) in rows.iter().enumerate() {
            let hash = hash(&row.literal);
            match folds.find(rows, hash, &row.literal) {
                Ok(first) => {
                    folds.ambiguous.insert(first, at as u32);
                }
                Err(slot) => {
                    folds.slots[slot] = Slot { hash: hash as u32, row: at as u32 };
                }
            }
        }
        folds
    }

    pub(super) fn get(&self, rows: &[LexicalTokenRow], name: &[u8]) -> Result<Option<u32>, Failure> {
        let Ok(at) = self.find(rows, hash(name), name) else {
            return Ok(None);
        };
        match self.ambiguous.get(&at) {
            Some(&last) => Err(several(&rows[last as usize].literal)),
            None => Ok(Some(rows[at as usize].token)),
        }
    }

    fn find(&self, rows: &[LexicalTokenRow], hash: u64, literal: &[u8]) -> Result<u32, usize> {
        let mask = self.slots.len() - 1;
        let mut slot = (hash.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize;
        loop {
            let entry = self.slots[slot];
            if entry.row == EMPTY {
                return Err(slot);
            }
            if entry.hash == hash as u32
                && rows[entry.row as usize].literal.eq_ignore_ascii_case(literal)
            {
                return Ok(entry.row);
            }
            slot = (slot + 1) & mask;
        }
    }
}
