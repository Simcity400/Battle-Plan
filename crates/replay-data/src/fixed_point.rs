//! The engine's conversions between its 15-bit fixed point (Q15) and the
//! five-decimal fixed point (fixed5) the save writer and the displayed values
//! use. Shared by the analytics formula and the fresh-save oracle, so both read
//! a recorded Q15 word exactly as the game does.

/// Convert Q15 words to fixed5, preserving saturation and i32 truncations.
pub fn q15_to_fixed5(value: i64) -> i64 {
    let whole = i64::from((value >> 15) as i32);
    if whole > 0x36f9_bfb1 {
        return 0x53e2_d61f_d320;
    }
    if whole < -0x36f9_bfb1 {
        return -0x53e2_d61f_d321;
    }
    let fraction = value.wrapping_sub(whole << 15);
    let rounded = ((fraction.wrapping_mul(0xc350_0000) >> 15) + 0x4000) >> 15;
    i64::from(rounded as i32).wrapping_add(whole.wrapping_mul(100_000))
}

/// Convert fixed5 to Q15, truncating in both parts.
pub fn fixed5_to_q15(value: i64) -> i64 {
    let whole = value / 100_000;
    let rest = value - whole * 100_000;
    ((rest << 30) / 3_276_800_000).wrapping_add(whole << 15)
}

/// The save writer's rendering of a Q15 word as five decimals: truncated toward
/// zero, unlike [`q15_to_fixed5`].
pub fn q15_to_saved_fixed5(value: i64) -> i64 {
    (i128::from(value) * 100_000 / 32_768) as i64
}
