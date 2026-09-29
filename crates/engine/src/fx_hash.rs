//! A fixed, allocation-free 64-bit hash (FxHash's mix over 8-byte words) for
//! the sliced families' change fingerprints. Stable within a process: the
//! values are compared with earlier observations only, never stored.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxHash(pub u64);

impl FxHash {
    const SEED: u64 = 0x517c_c1b7_2722_0a95;

    /// Mix one word in.
    #[must_use]
    pub fn word(self, word: u64) -> Self {
        Self((self.0.rotate_left(5) ^ word).wrapping_mul(Self::SEED))
    }

    /// Mix `bytes` in, 8 bytes per word; the length separates a short tail
    /// from zero bytes.
    #[must_use]
    pub fn bytes(mut self, bytes: &[u8]) -> Self {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self = self.word(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let tail = chunks.remainder();
        let mut last = [0_u8; 8];
        last[..tail.len()].copy_from_slice(tail);
        self.word(u64::from_le_bytes(last) ^ ((bytes.len() as u64) << 56))
    }

    /// Mix an optional part in, its presence first.
    #[must_use]
    pub fn optional(self, part: Option<u64>) -> Self {
        self.word(u64::from(part.is_some())).word(part.unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_differs_from_its_zero_padding_and_presence_is_mixed() {
        assert_ne!(FxHash(0).bytes(&[1]), FxHash(0).bytes(&[1, 0]));
        assert_ne!(FxHash(0).bytes(&[]), FxHash(0).bytes(&[0]));
        assert_ne!(FxHash(0).optional(None), FxHash(0).optional(Some(0)));
        assert_eq!(FxHash(3).word(4), FxHash(3).word(4));
    }
}
