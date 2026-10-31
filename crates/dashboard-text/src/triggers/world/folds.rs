//! Lexer tokens by their literal under the lexer's fold, read from one LexicalTokens table:
//! each fold's literal once, lower-cased, in one buffer, found by its hash.

use crate::triggers::{Failure, refusal::refused};
use replay_data::rows::LexicalTokenRow;
use std::collections::BTreeMap;

/// Where `index` holds no fold.
const EMPTY: u32 = u32::MAX;

/// A fold: its hash, where its literal is in [`Folds::text`], and the token it names.
struct Fold {
    hash: u64,
    start: usize,
    len: usize,
    token: u32,
}

/// Lexer tokens by their literal under the lexer's fold; refused for a fold two recorded
/// literals share.
pub(in crate::triggers) struct Folds {
    /// The folded literals, end to end.
    text: Vec<u8>,
    folds: Vec<Fold>,
    /// Open addressing over `folds` by hash, a power of two long and never past half full.
    index: Vec<u32>,
    /// What a hash is shifted by to place it in `index`.
    shift: u32,
    /// Why each fold two literals share names no token, by fold.
    refused: BTreeMap<u32, Failure>,
}

/// `literal`'s hash under the fold.
fn hash(literal: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in literal {
        hash = (hash ^ u64::from(byte.to_ascii_lowercase())).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl Folds {
    /// The folds of `rows`, in their order.
    pub(super) fn read<'a>(rows: impl ExactSizeIterator<Item = &'a LexicalTokenRow>) -> Self {
        let slots = (rows.len() * 2).max(2).next_power_of_two();
        let mut folds = Self {
            text: Vec::new(),
            folds: Vec::with_capacity(rows.len()),
            index: vec![EMPTY; slots],
            shift: 64 - slots.trailing_zeros(),
            refused: BTreeMap::new(),
        };
        for row in rows {
            let hash = hash(&row.literal);
            match folds.find(hash, &row.literal) {
                Ok(at) => {
                    // One token per fold natively: two are refused.
                    folds.refused.insert(
                        at,
                        refused(format!(
                            "{} names several lexer tokens",
                            String::from_utf8_lossy(&row.literal)
                        )),
                    );
                }
                Err(slot) => {
                    folds.index[slot] = folds.folds.len() as u32;
                    folds.folds.push(Fold {
                        hash,
                        start: folds.text.len(),
                        len: row.literal.len(),
                        token: row.token,
                    });
                    folds
                        .text
                        .extend(row.literal.iter().map(u8::to_ascii_lowercase));
                }
            }
        }
        folds
    }

    /// The token `name` names under the fold, or why it names none; None where no literal
    /// folds to it.
    pub(super) fn get(&self, name: &[u8]) -> Option<Result<u32, &Failure>> {
        let at = self.find(hash(name), name).ok()?;
        Some(match self.refused.get(&at) {
            Some(reason) => Err(reason),
            None => Ok(self.folds[at as usize].token),
        })
    }

    /// The fold `literal` folds to, or the empty slot of `index` it would take.
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
