
#[inline]
pub fn slot(key: u64, slots: usize) -> usize {
    debug_assert!(slots.is_power_of_two());
    let mut value = key;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
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
