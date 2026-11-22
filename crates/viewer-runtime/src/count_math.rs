
fn invalid_fixed(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
}

pub fn ratio(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 || invalid_fixed(numerator) {
        0xffff_ffff
    } else {
        numerator.wrapping_mul(100_000).wrapping_div(denominator)
    }
}

fn fixed15(value: i64) -> i64 {
    (value / 100_000)
        .wrapping_mul(32_768)
        .wrapping_add((value % 100_000).wrapping_mul(1 << 30) / 3_276_800_000)
}

pub fn count_range_seeded(
    count: i32,
    intel: i64,
    seed: u32,
    hour: i32,
    [minimum, maximum, range]: [i64; 3],
) -> [i32; 2] {
    let value = i64::from(count) << 15;
    if value <= 0 {
        return [0; 2];
    }
    let delta = intel.wrapping_sub(minimum);
    let denominator = maximum.wrapping_sub(minimum);
    if denominator == 0 || invalid_fixed(delta) {
        return [count; 2];
    }
    let ratio = ratio(delta, denominator);
    if ratio >= 100_000 {
        return [count; 2];
    }
    let width = (100_000_i64.wrapping_sub(ratio.max(0))).wrapping_mul(range) / 100_000;
    let spread = value
        .wrapping_mul(fixed15(width))
        .wrapping_shr(15)
        .wrapping_abs();
    let date = hour.wrapping_sub(43_800_000);
    let mut day = (date / 24) % 365;
    let mut month = 0_i32;
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    let salt = seed
        .wrapping_add(count as u32)
        .wrapping_add(month as u32)
        .wrapping_add((date / 8760).wrapping_mul(12) as u32);
    let deviation = |next| {
        let mut hash = 0x5EA6BA9F_u32
            .wrapping_sub(salt)
            .wrapping_add(salt.wrapping_mul(0xB5297A4D))
            .wrapping_add(next);
        hash = (hash >> 8 ^ hash).wrapping_add(0x68E31DA4);
        hash = (hash.wrapping_mul(0x100) ^ hash).wrapping_mul(0x1B56C4E9);
        hash = (hash >> 8 ^ hash).wrapping_mul(0x92D68CA2);
        let random = i64::from(((hash >> 8 ^ hash) & 0x7fff_ffff) % 100_000);
        let root = sqrt_fixed5(random);
        fixed15((100_000 - root).wrapping_mul(20_000) / 100_000 + root)
    };
    let lower = value
        .wrapping_sub(spread.wrapping_mul(deviation(0)) >> 15)
        .min(value);
    let upper = value
        .wrapping_add(spread.wrapping_mul(deviation(0xB5297A4D)) >> 15)
        .max(value)
        .max(lower);
    [lower, upper].map(|bound| (bound.max(32_768).wrapping_add(0x4000) >> 15) as i32)
}

fn sqrt_fixed5(input: i64) -> i64 {
    if input < 2 {
        return 0;
    }
    let mut root = input.wrapping_mul(100_000) / 200_000;
    let tolerance = input.wrapping_mul(1_000) / 100_000;
    loop {
        let error = (root.wrapping_mul(root) / 100_000).wrapping_sub(input);
        let step = ratio(error, root.wrapping_mul(2));
        root = root.wrapping_sub(step);
        if (-tolerance..=tolerance).contains(&error) || step == 0 {
            return root;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_count_operands_keep_their_native_integer_steps() {
        assert_eq!(fixed15(-150_001), -49_152);
        assert_eq!(sqrt_fixed5(25_000), 50_001);
    }
}
