
use crate::triggers::{Failure, refusal::refused};
use std::collections::BTreeMap;

const EMPTY: u32 = u32::MAX;

struct Fold {
    hash: u64,
    start: usize,
    len: usize,
    token: u32,
}

pub(in crate::triggers) struct Folds {
    text: Vec<u8>,
    folds: Vec<Fold>,
    index: Vec<u32>,
    shift: u32,
    refused: BTreeMap<u32, Failure>,
}

pub(super) fn several(literal: &[u8]) -> Failure {
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
    pub(super) fn read<'a>(
        len: usize,
        rows: impl Iterator<Item = Result<(u32, &'a [u8]), Failure>>,
    ) -> Result<Self, Failure> {
        let slots = (len * 2).max(2).next_power_of_two();
        let mut folds = Self {
            text: Vec::new(),
            folds: Vec::with_capacity(len),
            index: vec![EMPTY; slots],
            shift: 64 - slots.trailing_zeros(),
            refused: BTreeMap::new(),
        };
        for row in rows {
            let (token, literal) = row?;
            let hash = hash(literal);
            match folds.find(hash, literal) {
                Ok(at) => {
                    folds.refused.insert(at, several(literal));
                }
                Err(slot) => {
                    folds.index[slot] = folds.folds.len() as u32;
                    folds.folds.push(Fold {
                        hash,
                        start: folds.text.len(),
                        len: literal.len(),
                        token,
                    });
                    folds.text.extend(literal.iter().map(u8::to_ascii_lowercase));
                }
            }
        }
        Ok(folds)
    }

    pub(super) fn get(&self, name: &[u8]) -> Option<Result<u32, &Failure>> {
        let at = self.find(hash(name), name).ok()?;
        Some(match self.refused.get(&at) {
            Some(reason) => Err(reason),
            None => Ok(self.folds[at as usize].token),
        })
    }

    fn find(&self, hash: u64, literal: &[u8]) -> Result<u32, usize> {
        let mask = self.index.len() - 1;
        let mut slot = (hash.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize;
        loop {
            let at = self.index[slot];
            if at == EMPTY {
                return Err(slot);
            }
            let fold = &self.folds[at as usize];
            if fold.hash == hash
                && self.text[fold.start..fold.start + fold.len].eq_ignore_ascii_case(literal)
            {
                return Ok(at);
            }
            slot = (slot + 1) & mask;
        }
    }
}
