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

pub fn fixed5_to_q15(value: i64) -> i64 {
    let whole = value / 100_000;
    let rest = value - whole * 100_000;
    ((rest << 30) / 3_276_800_000).wrapping_add(whole << 15)
}

pub fn q15_to_saved_fixed5(value: i64) -> i64 {
    (i128::from(value) * 100_000 / 32_768) as i64
}
