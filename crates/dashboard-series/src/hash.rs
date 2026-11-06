//! The hash of the keys an emit looks up for every value it carries: places and handles made of
//! small integers, none of them from outside, so a multiply-rotate per word does where the
//! standard library's keyed hash costs several times as much.

use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
};

/// A map keyed by [`Words`].
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<Words>>;

/// Hashes word by word (the FxHash mix).
#[derive(Clone, Copy, Default)]
#[derive(replay_spool::Spooled)]
pub struct Words(u64);

const SEED: u64 = 0xf135_7aea_2e62_a9c5;

impl Words {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(26) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for Words {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("an 8-byte chunk")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word) ^ ((rest.len() as u64) << 59));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn write_isize(&mut self, value: isize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // The table reads its control bits from the top: fold the low bits up.
        self.0.rotate_left(26)
    }
}
