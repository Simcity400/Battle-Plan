//! Fixed-point naval dominance arithmetic shared by world fills and combat terrain text.
const ONE: i64 = 100_000;

/// Saturate on the signed fixed5 overflow guard, preserving wrapping
/// multiplication rather than limiting the unsigned quotient.
pub fn dominance_value(minimum: i64, ratio: i64) -> i64 {
    let (scaled, remaining) = if ratio < ONE {
        (minimum.wrapping_mul(ratio), ONE.wrapping_sub(ratio))
    } else {
        (minimum.wrapping_mul(99_000), 1_000)
    };
    if remaining == 0 {
        return u32::MAX as i64;
    }
    let quotient = scaled / ONE;
    if (quotient as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
        u32::MAX as i64
    } else {
        quotient.wrapping_mul(ONE) / remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dominance_saturates_only_on_the_native_fixed_overflow_guard() {
        let minimum = (u32::MAX as i64 + 1) * ONE;
        assert_eq!(dominance_value(minimum, 1), minimum / (ONE - 1));
        assert_ne!(dominance_value(minimum, 1), u32::MAX as i64);
    }
}
