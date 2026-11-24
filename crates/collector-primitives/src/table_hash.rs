
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

