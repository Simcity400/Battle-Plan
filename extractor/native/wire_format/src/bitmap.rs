//! Bitmaps: field `i` is bit `i & 7` of byte `i >> 3`,
//! LSB-first, unused trailing bits zero, zero bits occupying zero bytes.
//!
//! The zero-bit case is not a degenerate corner — it is what makes the four field
//! states cost nothing on records that need none of them, so it is tested rather
//! than assumed.

use crate::varint::{Result, WireError};

pub fn byte_len(bits: usize) -> usize {
    bits.div_ceil(8)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Bitmap {
    bits: Vec<bool>,
}

impl Bitmap {
    pub fn with_len(bits: usize) -> Self {
        Self { bits: vec![false; bits] }
    }

    pub fn len(&self) -> usize {
        self.bits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    pub fn get(&self, index: usize) -> bool {
        self.bits.get(index).copied().unwrap_or(false)
    }

    pub fn set(&mut self, index: usize, value: bool) {
        if index < self.bits.len() {
            self.bits[index] = value;
        }
    }

    pub fn count_set(&self) -> usize {
        self.bits.iter().filter(|b| **b).count()
    }

    pub fn iter_set(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(i, set)| set.then_some(i))
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        if self.bits.is_empty() {
            return;
        }
        let mut bytes = vec![0u8; byte_len(self.bits.len())];
        for (i, set) in self.bits.iter().enumerate() {
            if *set {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        out.extend_from_slice(&bytes);
    }

    /// Decode `bits` bits at `pos`. Rejects a non-zero trailing pad, because a
    /// writer that leaves junk there produces different bytes for the same state.
    pub fn decode(buf: &[u8], pos: usize, bits: usize) -> Result<(Self, usize)> {
        if bits == 0 {
            return Ok((Self::default(), pos));
        }
        let len = byte_len(bits);
        // Subtraction, not addition: a cursor already past the end must not
        // overflow `pos + len` before the comparison rejects it.
        if pos > buf.len() || len > buf.len() - pos {
            return Err(WireError::Truncated);
        }
        let bytes = &buf[pos..pos + len];
        let used_in_last = bits - 8 * (len - 1);
        if used_in_last < 8 {
            let pad_mask = !((1u16 << used_in_last) - 1) as u8;
            if bytes[len - 1] & pad_mask != 0 {
                return Err(WireError::Corrupt("bitmap trailing pad bits must be zero"));
            }
        }
        let mut map = Self::with_len(bits);
        for i in 0..bits {
            map.bits[i] = bytes[i >> 3] & (1 << (i & 7)) != 0;
        }
        Ok((map, pos + len))
    }
}

impl From<Vec<bool>> for Bitmap {
    fn from(bits: Vec<bool>) -> Self {
        Self { bits }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bits_occupy_zero_bytes() {
        let map = Bitmap::with_len(0);
        let mut out = Vec::new();
        map.encode(&mut out);
        assert!(out.is_empty(), "an empty bitmap costs nothing");
        let (back, pos) = Bitmap::decode(&out, 0, 0).unwrap();
        assert_eq!(back.len(), 0);
        assert_eq!(pos, 0, "and consumes nothing");
    }

    #[test]
    fn lsb_first_within_each_byte() {
        let mut map = Bitmap::with_len(9);
        map.set(0, true);
        map.set(7, true);
        map.set(8, true);
        let mut out = Vec::new();
        map.encode(&mut out);
        assert_eq!(out, vec![0b1000_0001, 0b0000_0001]);
    }

    #[test]
    fn roundtrips_at_every_width_up_to_three_bytes() {
        for bits in 0..=24 {
            let mut map = Bitmap::with_len(bits);
            for i in (0..bits).step_by(3) {
                map.set(i, true);
            }
            let mut out = Vec::new();
            map.encode(&mut out);
            assert_eq!(out.len(), byte_len(bits), "{bits} bits");
            let (back, pos) = Bitmap::decode(&out, 0, bits).unwrap();
            assert_eq!(back, map, "{bits} bits roundtrip");
            assert_eq!(pos, out.len());
        }
    }

    #[test]
    fn non_zero_trailing_pad_is_corrupt() {
        // Three bits declared, but bit 7 set in the stored byte.
        let stored = [0b1000_0001u8];
        assert_eq!(
            Bitmap::decode(&stored, 0, 3),
            Err(WireError::Corrupt("bitmap trailing pad bits must be zero"))
        );
        // The same byte is fine when all eight bits are declared.
        assert!(Bitmap::decode(&stored, 0, 8).is_ok());
    }

    #[test]
    fn truncation_is_detected() {
        assert_eq!(Bitmap::decode(&[], 0, 1), Err(WireError::Truncated));
        assert_eq!(Bitmap::decode(&[0u8], 0, 9), Err(WireError::Truncated));
    }
}
