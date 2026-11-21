
pub(super) const ONE: i64 = 100_000;
pub(super) const EXCEPTIONAL: i64 = 0xffff_ffff;

pub(super) fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

pub(super) const TRAP: &str =
    "the native fixed5 division traps on this quotient (a wrapped product of i64::MIN over -1)";

pub(super) fn exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}

pub(super) fn ratio(numerator: i64, denominator: i64, zero: i64) -> Option<i64> {
    if denominator == 0 {
        return Some(zero);
    }
    if exceptional(numerator) {
        return Some(EXCEPTIONAL);
    }
    numerator.wrapping_mul(ONE).checked_div(denominator)
}

pub(super) fn fraction(value: i64, maximum: i64) -> Option<i64> {
    ratio(value, maximum, EXCEPTIONAL)
}

pub(super) fn round(value: i64) -> i32 {
    (value.wrapping_add(if value < 0 { -50_000 } else { 50_000 }) / ONE) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_division_keeps_its_sentinel_and_wrapping() {
        for numerator in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(fraction(numerator, ONE), Some(EXCEPTIONAL));
        }
        assert!(!exceptional(i64::MIN + 2));
        assert_eq!(fraction(1, 0), Some(EXCEPTIONAL));
        assert_eq!(ratio(1, 0, 0), Some(0));
        assert_eq!(mul(i64::MAX, 2), 0);
        assert_eq!(ratio(-1, 3, EXCEPTIONAL), Some(-33_333));
        assert_eq!(round(-150_000), -2);
        assert_eq!(round(149_999), 1);
    }
}
