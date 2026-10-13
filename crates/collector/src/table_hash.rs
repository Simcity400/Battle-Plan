//! The first slot of a native address (or a key built from one) in a
//! power-of-two hash table: the top bits of the SplitMix64 finalizer.
//!
//! Objects of one size allocated one after another sit at an arithmetic run
//! of addresses. A multiplicative hash maps such a run through a slice of
//! `key * K`, and for some strides every slice, the top bits included,
//! piles the run into a few slots: a watch table refuses registrations past
//! its probe bound, an open-addressed table probes long chains and a
//! direct-mapped memo keeps evicting its own entries. The finalizer mixes
//! every key bit into every slot bit, so a run of any stride spreads like
//! random keys. Pure arithmetic: safe on hook paths.

/// The first slot of `key` in a table of `slots` slots, a power of two.
#[inline]
pub(crate) fn slot(key: u64, slots: usize) -> usize {
    debug_assert!(slots.is_power_of_two());
    let mut value = key;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    // A one-slot table takes no bits: the whole-width shift is refused.
    value
        .checked_shr(u64::BITS - slots.trailing_zeros())
        .unwrap_or(0) as usize
}

#[cfg(test)]
mod tests {
    use super::slot;

    #[test]
    fn every_slot_is_in_its_table() {
        for bits in 0..=20 {
            let slots = 1_usize << bits;
            for key in [0, 1, 0x1000, u64::MAX, 0x9e37_79b9_7f4a_7c15] {
                assert!(slot(key, slots) < slots, "{key:#x} in {slots}");
            }
        }
        assert_eq!(slot(u64::MAX, 1), 0);
    }

    /// Runs of 64 objects at every stride from 16 bytes to 32 KiB fill at
    /// least 40 of 256 slots, as random keys do (about 57 on average); the
    /// multiplicative hashes this replaces put some runs in 5 or 6.
    #[test]
    fn strided_runs_spread_like_random_keys() {
        for base in [0x1_2345_6780_u64, 0x2_0000_0000, 0x7ff0_1234_5670] {
            for stride in (0x10..=0x8000).step_by(0x10) {
                let mut used = [false; 256];
                for index in 0..64 {
                    used[slot((base + index * stride) >> 4, 256)] = true;
                }
                let spread = used.iter().filter(|&&used| used).count();
                assert!(spread >= 40, "base {base:#x}, stride {stride:#x}: {spread}");
            }
        }
    }
}
