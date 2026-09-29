//! The engine's conversion of an authored script number to its fixed point with
//! five decimals: the text after the dot keeps only its first five digits, so
//! a value is truncated, never rounded.

use crate::crt::{Scanned, scan_i64};

/// `1424b0480`, the script fixed5 conversion: two `%lli` scans
/// ([`crate::crt::scan_i64`]), the whole part and then up to five fraction
/// digits after the first `.`, unlike save-file decimal parsing. It fails
/// only when the first scan returns 0; a scan that returns EOF leaves its
/// zero.
pub fn script_fixed5(bytes: &[u8]) -> Option<i64> {
    let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
    let whole = match scan_i64(bytes) {
        Scanned::Stored(value) => value,
        Scanned::End => 0,
        Scanned::NoMatch => return None,
    };
    let Some(dot) = bytes.iter().position(|byte| *byte == b'.') else {
        return Some(whole.wrapping_mul(100_000));
    };
    let mut fraction = [b'0'; 5];
    let suffix = &bytes[dot + 1..];
    let copied = suffix.len().min(5);
    fraction[..copied].copy_from_slice(&suffix[..copied]);
    let start = fraction.iter().position(|byte| *byte != b'0').unwrap_or(5);
    let mut part = match scan_i64(&fraction[start..]) {
        Scanned::Stored(value) => value,
        Scanned::End | Scanned::NoMatch => 0,
    };
    if whole < 0 {
        part = part.wrapping_neg();
    }
    let raw = whole.wrapping_mul(100_000).wrapping_add(part);
    Some(if whole == 0 && bytes.first() == Some(&b'-') {
        raw.wrapping_neg()
    } else {
        raw
    })
}

/// ParseTokenInt32 (1424B0080) -> the UCRT's `atol` (parse_signed_decimal32
/// 14254AF28 -> parse_integer32_radix 14253C770, base 10, signed): white
/// space, a sign, decimal digits, clamped to the `i32` range; no digits give
/// 0. The text ends at its first NUL.
pub fn decimal_i32(bytes: &[u8]) -> i32 {
    let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
    atoll(bytes).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The UCRT's `atoll` (14254ADC4 -> parse_integer64_radix 14253D1FC with
/// base 10, signed): white space, a sign, decimal digits, clamped to the
/// `i64` range; no digits give 0.
fn atoll(bytes: &[u8]) -> i64 {
    let mut cursor = 0;
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'..=b'\r'))
    {
        cursor += 1;
    }
    let negative = bytes.get(cursor) == Some(&b'-');
    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    let mut magnitude = 0_u64;
    while let Some(digit) = bytes.get(cursor).filter(|byte| byte.is_ascii_digit()) {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'));
        cursor += 1;
    }
    let bound = i64::MAX as u64 + u64::from(negative);
    let signed = magnitude.min(bound);
    if negative {
        (signed as i64).wrapping_neg()
    } else {
        signed as i64
    }
}

/// format_fixed5 (1424B89F0): `trim` is its mode 2, otherwise mode 1. The
/// whole part is printed as a 32-bit integer (FormatInt32DecimalIntoBuffer
/// 1424B7A80 takes ECX), and the decimals are the remainder plus 100000
/// printed with its first digit turned into `.` and cut after `precision`
/// digits, truncating rather than rounding. Mode 1 keeps the cut; mode 2
/// trims trailing zeros back from it and drops a bare `.`.
pub fn format_fixed5(value: i64, trim: bool, precision: u8) -> String {
    let mut text = String::new();
    let mut magnitude = value;
    if magnitude < 0 {
        text.push('-');
        magnitude = magnitude.wrapping_neg();
    }
    text.push_str(&((magnitude / 100_000) as i32).to_string());
    let remainder = magnitude % 100_000;
    if precision != 0 && (remainder != 0 || !trim) {
        let mut decimals = ((remainder as i32) + 100_000).to_string().into_bytes();
        decimals[0] = b'.';
        decimals.truncate(usize::from(precision) + 1);
        // Mode 2 trims back from the cut; with precision 6 the cut lies past
        // the terminator, which stops the trim before any digit.
        if trim && precision <= 5 {
            while decimals.len() > 1 && decimals.last() == Some(&b'0') {
                decimals.pop();
            }
            if decimals.len() == 1 {
                decimals.clear();
            }
        }
        text.push_str(&String::from_utf8(decimals).expect("ASCII digits"));
    }
    text
}

/// `Fixed5ToString` (1424B6200 -> [`format_fixed5`]): it passes `precision`
/// on and `!precision >> 31` as the mode (NOT R8D at 1424B622B, SHR R8D, 0x1F
/// at 1424B622E). A nonnegative precision is mode 1: exactly that many
/// truncated digits whenever it is nonzero, and a cut past the fifth digit
/// leaves all five. A negative one is mode 0: never cut, every trailing zero
/// trimmed, which is mode 2 cut at the fifth digit.
pub fn fixed5_to_string(value: i64, precision: i32) -> String {
    if precision < 0 {
        format_fixed5(value, true, 5)
    } else {
        format_fixed5(value, false, precision.min(5) as u8)
    }
}

/// `FormatFixed5Percent` (142259590): the value, times 100 when `times_hundred`
/// (`R9B`), through [`fixed5_to_string`] at `precision` (`R8D`, passed on to
/// 1424B6200), then `%`.
pub fn fixed5_percent(value: i64, precision: i32, times_hundred: bool) -> String {
    let value = if times_hundred {
        value.wrapping_mul(100)
    } else {
        value
    };
    let mut text = fixed5_to_string(value, precision);
    text.push('%');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed5_text_truncates_or_trims_as_the_native_formatter() {
        assert_eq!(fixed5_to_string(12_345, 2), "0.12");
        assert_eq!(fixed5_to_string(-12_345, 2), "-0.12");
        assert_eq!(fixed5_to_string(-1, 2), "-0.00");
        assert_eq!(fixed5_to_string(100_000, 2), "1.00");
        assert_eq!(fixed5_to_string(150_000, 0), "1");
        assert_eq!(fixed5_to_string(150_000, -1), "1.5");
        assert_eq!(fixed5_to_string(100_000, -1), "1");
        assert_eq!(fixed5_to_string(1_234_567, -1), "12.34567");
        // Mode 1 cut past the fifth digit keeps all five.
        assert_eq!(fixed5_to_string(150_000, 9), "1.50000");
        // -(i64::MIN) wraps to itself: `-`, (MIN / 100000) as i32, then
        // (MIN % 100000) + 100000 = 24192 with its first digit made `.`.
        assert_eq!(
            fixed5_to_string(i64::MIN, -1),
            format!("-{}.4192", (i64::MIN / 100_000) as i32)
        );
    }

    #[test]
    fn format_fixed5_modes_one_and_two() {
        assert_eq!(format_fixed5(150_000, false, 3), "1.500");
        assert_eq!(format_fixed5(150_000, true, 3), "1.5");
        assert_eq!(format_fixed5(100_000, true, 3), "1");
        assert_eq!(format_fixed5(100_000, false, 0), "1");
        // Precision 6 in mode 2 cuts past the terminator: nothing is trimmed.
        assert_eq!(format_fixed5(150_000, true, 6), "1.50000");
    }

    #[test]
    fn percent_text_scales_before_formatting() {
        assert_eq!(fixed5_percent(0, 0, true), "0%");
        assert_eq!(fixed5_percent(75_000, 0, true), "75%");
        assert_eq!(fixed5_percent(12_345, 1, false), "0.1%");
        assert_eq!(fixed5_percent(-500, 0, true), "-0%");
    }

    #[test]
    fn script_fixed5_matches_the_native_scanner() {
        assert_eq!(script_fixed5(b"0.05"), Some(5_000));
        assert_eq!(script_fixed5(b"-0.5"), Some(-50_000));
        assert_eq!(script_fixed5(b"-1.5"), Some(-150_000));
        // Digits past the fifth are dropped, not rounded.
        assert_eq!(script_fixed5(b"0.123456"), Some(12_345));
        // %lli honours C radix prefixes in the whole part.
        assert_eq!(script_fixed5(b"010.5"), Some(850_000));
        assert_eq!(script_fixed5(b"0x10"), Some(1_600_000));
        // The fraction is scanned too: "5e3" reads 5.
        assert_eq!(script_fixed5(b"1.5e3"), Some(100_005));
        assert_eq!(script_fixed5(b"yes"), None);
        // 1424B0480 fails only when sscanf returns 0; an empty token is
        // EOF (-1), so the whole part stays 0.
        assert_eq!(script_fixed5(b""), Some(0));
    }

    #[test]
    fn decimal_i32_is_atol() {
        assert_eq!(decimal_i32(b" 010"), 10);
        assert_eq!(decimal_i32(b"0x10"), 0);
        assert_eq!(decimal_i32(b"2.5"), 2);
        assert_eq!(decimal_i32(b"1tail"), 1);
        assert_eq!(decimal_i32(b" -7x"), -7);
        assert_eq!(decimal_i32(b"+3"), 3);
        assert_eq!(decimal_i32(b"x"), 0);
        assert_eq!(decimal_i32(b"99999999999"), i32::MAX);
        assert_eq!(decimal_i32(b"-99999999999"), i32::MIN);
    }
}
