//! The engine's conversion of an authored script number to its fixed point with
//! five decimals: the text after the dot keeps only its first five digits, so
//! a value is truncated, never rounded.

use crate::crt::{Scanned, scan_i32, scan_i64};

/// Script fixed5 conversion uses two `%lli` scans ([`crate::crt::scan_i64`]): the whole
/// part, then up to five fraction digits after the first `.`. This differs from
/// save-file decimal parsing. Only a no-match first scan fails; EOF leaves its initial
/// zero.
pub fn script_fixed5(bytes: &[u8]) -> Option<i64> {
    let bytes = crate::parser::c_string(bytes);
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

/// Decimal fixed5 conversion of a value token never fails:
///
/// - The whole part accepts leading white space, a sign and decimal digits, saturating
///   on overflow; no digits read as 0.
/// - Every byte after the first `.` is treated as a digit by subtracting `0` as a
///   signed char, with wrapping 64-bit accumulation. Up to four bytes are scaled to five
///   places; five or more are taken as they are (`0.000001` is raw 1).
/// - The fraction is negated when the whole part is negative, or is zero and the text
///   starts with `-`.
/// - A fraction whose multiplication by 100000 wraps below `i64::MIN + 2` becomes
///   0xFFFFFFFF; other fractions pass through the wrapping signed scaling arithmetic.
/// - The whole part times 100000 plus the fraction wraps.
///
/// Token text ends at its first NUL.
pub fn decimal_fixed5(bytes: &[u8]) -> i64 {
    let bytes = crate::parser::c_string(bytes);
    let whole = atoll(bytes);
    let start = bytes
        .iter()
        .position(|byte| *byte == b'.')
        .map_or(bytes.len(), |dot| dot + 1);
    let mut fraction = 0_u64;
    let mut places = 0_u64;
    for byte in &bytes[start..] {
        places += 1;
        let digit = i64::from(*byte as i8) - 0x30;
        fraction = (digit as u64).wrapping_add(fraction.wrapping_mul(10));
    }
    if places <= 4 {
        for _ in places..5 {
            fraction = fraction.wrapping_mul(10);
        }
    }
    if (bytes.first() == Some(&b'-') && whole == 0) || whole < 0 {
        fraction = fraction.wrapping_neg();
    }
    let part = if (fraction.wrapping_mul(100_000) as i64) < -0x7fff_ffff_ffff_fffe {
        0xffff_ffff
    } else {
        (fraction.wrapping_mul(10_000_000_000) as i64).wrapping_div(10_000_000_000)
    };
    whole.wrapping_mul(100_000).wrapping_add(part)
}

/// Signed decimal conversion accepts white space, a sign and decimal digits, clamped to
/// the `i32` range. No digits give 0; text ends at its first NUL.
pub fn decimal_i32(bytes: &[u8]) -> i32 {
    let bytes = crate::parser::c_string(bytes);
    atoll(bytes).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Signed decimal conversion accepts white space, a sign and decimal digits, clamped to
/// the `i64` range. No digits give 0.
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

/// Console and scoped-variable fixed5 conversion multiplies the decimal whole part by
/// 100000 and treats every byte after the first `.` as an unchecked digit. Fewer than
/// five places are padded; more are kept. A leading `-` negates the fraction of a zero
/// whole part.
pub fn parse_fixed5_string(bytes: &[u8]) -> i64 {
    let whole = atoll(bytes);
    let after = bytes
        .iter()
        .position(|byte| *byte == b'.')
        .map_or(&[][..], |dot| &bytes[dot + 1..]);
    let mut fraction = after.iter().fold(0_i64, |fraction, byte| {
        fraction
            .wrapping_mul(10)
            .wrapping_add(i64::from(*byte as i8) - 0x30)
    });
    if after.len() <= 4 {
        for _ in after.len()..5 {
            fraction = fraction.wrapping_mul(10);
        }
    }
    if (bytes.first() == Some(&b'-') && whole == 0) || whole < 0 {
        fraction = fraction.wrapping_neg();
    }
    let part = if fraction.wrapping_mul(100_000) < -0x7fff_ffff_ffff_fffe {
        0xffff_ffff
    } else {
        fraction.wrapping_mul(10_000_000_000) / 10_000_000_000
    };
    whole.wrapping_mul(100_000).wrapping_add(part)
}

/// An integer trigger operand after variable resolution. Keep the resolved token's
/// quoted flag: it decides the conversion path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntOperand {
    /// A number token ([`crate::parser::number_token`]) read by `%i`
    /// ([`crate::crt::scan_i32`]): base 8 after a leading `0`, base 16 after `0x`,
    /// storing the low 32 bits of the 64-bit scan. Bare `010` is 8. EOF stores nothing
    /// and leaves the initial zero.
    Integer(i32),
    /// A number token for which `%i` matches nothing.
    Unreadable,
    /// A scoped-variable token containing only `-+0123456789.` becomes a fixed5
    /// constant through [`parse_fixed5_string`]: quoted `"010"` is 10.
    Constant(i64),
    /// Empty text is reported and leaves the operand unset.
    Empty,
    /// Any other text names a variable or a scope evaluated at run time.
    Scoped,
}

/// See [`IntOperand`].
pub fn int_operand(text: &[u8], quoted: bool) -> IntOperand {
    if crate::parser::number_token(text, quoted) {
        return match scan_i32(text) {
            Scanned::Stored(value) => IntOperand::Integer(value),
            Scanned::End => IntOperand::Integer(0),
            Scanned::NoMatch => IntOperand::Unreadable,
        };
    }
    if text.is_empty() {
        IntOperand::Empty
    } else if text.iter().all(|byte| b"-+0123456789.".contains(byte)) {
        IntOperand::Constant(parse_fixed5_string(text))
    } else {
        IntOperand::Scoped
    }
}

/// Format the whole part as a 32-bit integer and truncate the fraction after
/// `precision` digits. `trim` removes trailing zeros from that cut and drops a bare
/// dot; otherwise the cut is kept.
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

/// Format fixed5 at `precision`. A nonnegative precision keeps exactly that many
/// truncated digits when nonzero, with all five retained for larger cuts. A negative
/// precision retains up to five digits and trims every trailing zero.
pub fn fixed5_to_string(value: i64, precision: i32) -> String {
    if precision < 0 {
        format_fixed5(value, true, 5)
    } else {
        format_fixed5(value, false, precision.min(5) as u8)
    }
}

/// Optionally multiply the value by 100 with wrapping arithmetic, format it through
/// [`fixed5_to_string`] at `precision`, then append `%`.
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
    fn fixed5_text_truncates_or_trims_fractional_digits() {
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
        // The minimum value keeps its wrapped sign and narrowed whole part.
        assert_eq!(fixed5_to_string(i64::MIN, -1), "-702313053.4192");
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
    fn script_fixed5_scans_whole_and_fraction_parts() {
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
        // Only a no-match scan fails. An empty token is EOF, so the whole part stays
        // zero.
        assert_eq!(script_fixed5(b""), Some(0));
    }

    /// Decimal conversion has no radix prefixes or five-digit limit; every byte after
    /// the dot counts as a digit.
    #[test]
    fn decimal_fixed5_keeps_unscaled_long_fractions() {
        assert_eq!(decimal_fixed5(b"-0.05001"), -5_001);
        assert_eq!(decimal_fixed5(b"0.25"), 25_000);
        assert_eq!(decimal_fixed5(b".5"), 50_000);
        assert_eq!(decimal_fixed5(b"-.5"), -50_000);
        assert_eq!(decimal_fixed5(b"010"), 1_000_000);
        assert_eq!(decimal_fixed5(b"0x10"), 0);
        // Six places are taken unscaled.
        assert_eq!(decimal_fixed5(b"0.000001"), 1);
        assert_eq!(decimal_fixed5(b"1.123456"), 223_456);
        // 'e' is 53 to the scanner: 5, 5*10+53, 103*10+3, then two places.
        assert_eq!(decimal_fixed5(b"1.5e3"), 203_300);
        assert_eq!(decimal_fixed5(b"yes"), 0);
        assert_eq!(decimal_fixed5(b""), 0);
        assert_eq!(decimal_fixed5(b"@bonus"), 0);
        assert_eq!(decimal_fixed5(b"99999999999999999999"), -100_000);
    }

    /// `%i` scans number tokens; other tokens use scoped-variable fixed5 conversion.
    #[test]
    fn integer_operands_take_the_token_kinds_path() {
        assert_eq!(int_operand(b"010", false), IntOperand::Integer(8));
        assert_eq!(int_operand(b"0x10", false), IntOperand::Integer(16));
        assert_eq!(int_operand(b"-7", false), IntOperand::Integer(-7));
        assert_eq!(int_operand(b"-x", false), IntOperand::Unreadable);
        // EOF stores nothing; the operand starts at zero.
        assert_eq!(int_operand(b"-", false), IntOperand::Integer(0));
        assert_eq!(int_operand(b"010", true), IntOperand::Constant(1_000_000));
        assert_eq!(int_operand(b"-0.5", true), IntOperand::Constant(-50_000));
        assert_eq!(int_operand(b"+.5", false), IntOperand::Constant(50_000));
        assert_eq!(int_operand(b"", true), IntOperand::Empty);
        assert_eq!(int_operand(b"var:x", false), IntOperand::Scoped);
        assert_eq!(int_operand(b"yes", false), IntOperand::Scoped);
    }

    #[test]
    fn fixed5_strings_keep_every_fraction_byte() {
        assert_eq!(parse_fixed5_string(b"1.5"), 150_000);
        assert_eq!(parse_fixed5_string(b"2.00001"), 200_001);
        // Six places are not cut back to five.
        assert_eq!(parse_fixed5_string(b"0.123456"), 123_456);
        assert_eq!(parse_fixed5_string(b"-1.25"), -125_000);
        assert_eq!(parse_fixed5_string(b"12"), 1_200_000);
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

    /// Decimal integer conversion saturates at 32 bits.
    #[test]
    fn atoi_saturates() {
        assert_eq!(decimal_i32(b"99999999999"), i32::MAX);
        assert_eq!(decimal_i32(b"-99999999999"), i32::MIN);
        assert_eq!(decimal_i32(b"-2147483648"), i32::MIN);
        assert_eq!(decimal_i32(b"2147483647"), i32::MAX);
        assert_eq!(decimal_i32(b"4294967296"), i32::MAX);
        assert_eq!(decimal_i32(b" +10.5"), 10);
        assert_eq!(decimal_i32(b"1e2"), 1);
        assert_eq!(decimal_i32(b"0x10"), 0);
        assert_eq!(decimal_i32(b"px"), 0);
    }

    /// Script fixed5 reads retain at most five fractional digits.
    #[test]
    fn fixed5_reads_digit_by_digit() {
        for (text, value) in [
            (&b"1.5"[..], Some(150_000)),
            (b"1.05", Some(105_000)),
            (b"1.123456", Some(112_345)),
            (b"-0.5", Some(-50_000)),
            (b"-1.5", Some(-150_000)),
            (b"+1.5", Some(150_000)),
            (b"1.", Some(100_000)),
            (b"1.x", Some(100_000)),
            (b"1.5e2", Some(100_005)),
            (b"1.-5", Some(95_000)),
            (b"1e3", Some(100_000)),
            (b"0x10", Some(1_600_000)),
            (b"", Some(0)),
            (b".5", None),
            (b"abc", None),
        ] {
            assert_eq!(script_fixed5(text), value, "{text:?}");
        }
    }
}
