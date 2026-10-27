/// FNV-1a over an owner's fields, for its fingerprint; stable within a
/// process, never persisted.
pub struct Digest(u64);

impl Default for Digest {
    fn default() -> Self {
        Self::new()
    }
}

impl Digest {
    #[inline]
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    #[inline]
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        self
    }

    #[inline]
    pub fn u64(&mut self, value: u64) -> &mut Self {
        const PRIME: u64 = 0x0100_0000_01b3;
        const fn power(mut count: usize) -> u64 {
            let mut value = 1_u64;
            while count != 0 {
                value = value.wrapping_mul(PRIME);
                count -= 1;
            }
            value
        }
        const ZERO_SUFFIX: [u64; 9] = [
            power(0),
            power(1),
            power(2),
            power(3),
            power(4),
            power(5),
            power(6),
            power(7),
            power(8),
        ];
        let count = (64 - value.leading_zeros()).div_ceil(8) as usize;
        // Zero suffix bytes only multiply: retain exactly the bytewise FNV hash.
        self.bytes(&value.to_le_bytes()[..count]);
        self.0 = self.0.wrapping_mul(ZERO_SUFFIX[8 - count]);
        self
    }

    #[inline]
    pub fn i64(&mut self, value: i64) -> &mut Self {
        self.u64(value as u64)
    }

    /// A length-prefixed byte string, so adjacent strings cannot alias.
    pub fn text(&mut self, text: &[u8]) -> &mut Self {
        self.u64(text.len() as u64).bytes(text)
    }

    #[inline]
    pub fn finish(&self) -> u64 {
        self.0
    }
}
