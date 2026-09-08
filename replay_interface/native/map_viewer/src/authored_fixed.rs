/// The a729 1424b0480 authored fixed-point conversion, including its `%lli` scans.
pub fn parse_fixed5(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let bytes = &bytes[..bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len())];
    let whole = scan_signed_auto(bytes)?;
    let Some(dot) = bytes.iter().position(|&byte| byte == b'.') else {
        return Some(whole.wrapping_mul(100_000));
    };
    let mut fraction = [b'0'; 5];
    let suffix = &bytes[dot + 1..];
    let copied = suffix.len().min(fraction.len());
    fraction[..copied].copy_from_slice(&suffix[..copied]);
    let start = fraction.iter().position(|&byte| byte != b'0').unwrap_or(fraction.len());
    // The second scan ignores failure and retains its zero-initialized destination.
    let mut part = scan_signed_auto(&fraction[start..]).unwrap_or(0);
    if whole < 0 {
        part = part.wrapping_neg();
    }
    let raw = whole.wrapping_mul(100_000).wrapping_add(part);
    Some(if whole == 0 && bytes.first() == Some(&b'-') { raw.wrapping_neg() } else { raw })
}

/// Result of the native `%i` assignment used by 1424b02c0.
/// Both failures leave the destination unchanged, but the owning `!= 0`
/// predicate accepts end of input (CRT return -1) and rejects matching failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeI32Assignment {
    Match,
    EndOfInput,
    MatchingFailure,
}

pub fn assign_native_i32(text: &str, destination: &mut i32) -> NativeI32Assignment {
    let bytes = text.as_bytes();
    let bytes = &bytes[..bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len())];
    match scan_signed_auto_outcome(bytes) {
        SignedScan::Match(value) => {
            // 142546ccc scans signed64; 1425477d0's width-four arm stores EAX.
            *destination = value as i32;
            NativeI32Assignment::Match
        }
        SignedScan::EndOfInput => NativeI32Assignment::EndOfInput,
        SignedScan::MatchingFailure => NativeI32Assignment::MatchingFailure,
    }
}

/// 14254af28 calls 14253c770 with radix 10 and signed32. Unlike the `%i`
/// assignment above, no digits yields zero and overflow saturates before return.
pub fn parse_native_decimal_i32(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let (mut cursor, negative) = scan_space_and_sign(bytes);
    let mut magnitude = 0_u32;
    while let Some(&byte) = bytes.get(cursor) {
        if !byte.is_ascii_digit() { break; }
        magnitude = magnitude.saturating_mul(10).saturating_add(u32::from(byte - b'0'));
        cursor += 1;
    }
    let bound = i32::MAX as u32 + u32::from(negative);
    let signed = magnitude.min(bound) as i32;
    if negative { signed.wrapping_neg() } else { signed }
}

enum SignedScan {
    Match(i64),
    EndOfInput,
    MatchingFailure,
}

fn scan_signed_auto(bytes: &[u8]) -> Option<i64> {
    match scan_signed_auto_outcome(bytes) {
        SignedScan::Match(value) => Some(value),
        SignedScan::EndOfInput => Some(0),
        SignedScan::MatchingFailure => None,
    }
}

fn scan_space_and_sign(bytes: &[u8]) -> (usize, bool) {
    let mut cursor = 0;
    while bytes.get(cursor).is_some_and(|byte| matches!(byte, b' ' | b'\t'..=b'\r')) {
        cursor += 1;
    }
    let negative = bytes.get(cursor) == Some(&b'-');
    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    (cursor, negative)
}

fn scan_signed_auto_outcome(bytes: &[u8]) -> SignedScan {
    let (mut cursor, negative) = scan_space_and_sign(bytes);
    let radix = if bytes.get(cursor) == Some(&b'0') {
        if matches!(bytes.get(cursor + 1), Some(b'x' | b'X')) {
            cursor += 2;
            16
        } else {
            8
        }
    } else {
        10
    };
    let first_digit = cursor;
    let mut magnitude = 0u64;
    while let Some(&byte) = bytes.get(cursor) {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'z' => u64::from(byte - b'a' + 10),
            b'A'..=b'Z' => u64::from(byte - b'A' + 10),
            _ => break,
        };
        if digit >= radix { break; }
        magnitude = magnitude.saturating_mul(radix).saturating_add(digit);
        cursor += 1;
    }
    if cursor == first_digit {
        // 14254681c returns EOF (-1) at end, matching failure (0) otherwise.
        // 1424b0480 rejects only 0; EOF leaves the initialized destination intact.
        return if cursor == bytes.len() {
            SignedScan::EndOfInput
        } else {
            SignedScan::MatchingFailure
        };
    }
    let bound = i64::MAX as u64 + u64::from(negative);
    let signed = magnitude.min(bound) as i64;
    SignedScan::Match(if negative { signed.wrapping_neg() } else { signed })
}

#[cfg(test)]
mod tests {
    use super::{assign_native_i32, parse_fixed5, parse_native_decimal_i32, NativeI32Assignment};

    #[test]
    fn decimal_i32_accepts_only_decimal_prefix_and_returns_zero_without_digits() {
        for (text, expected) in [
            ("", 0), (" \t\n\r\x0b\x0c", 0), ("+", 0), ("-", 0), ("bad", 0),
            (".5", 0), ("- 1", 0), ("0x20", 0), ("010", 10), ("08", 8),
            ("12suffix", 12), ("-12.75", -12), (" +17%", 17), ("1e3", 1),
            ("12\0more", 12), ("\0 123", 0), ("--12", 0), ("-0", 0),
        ] {
            assert_eq!(parse_native_decimal_i32(text), expected, "{text:?}");
        }
    }

    #[test]
    fn decimal_i32_saturates_at_signed_width_without_changing_percent_i_assignment() {
        for (text, expected) in [
            ("2147483647", i32::MAX), ("2147483648", i32::MAX),
            ("-2147483648", i32::MIN), ("-2147483649", i32::MIN),
            ("4294967295", i32::MAX), ("4294967296", i32::MAX),
            ("9223372036854775808", i32::MAX), ("-9223372036854775809", i32::MIN),
        ] {
            assert_eq!(parse_native_decimal_i32(text), expected, "{text:?}");
        }
        assert_eq!(parse_native_decimal_i32(&"9".repeat(4096)), i32::MAX);
        assert_eq!(parse_native_decimal_i32(&format!("-{}", "9".repeat(4096))), i32::MIN);
        let mut destination = 7;
        assert_eq!(assign_native_i32("2147483648", &mut destination), NativeI32Assignment::Match);
        assert_eq!(destination, i32::MIN);
        assert_eq!(assign_native_i32("010", &mut destination), NativeI32Assignment::Match);
        assert_eq!(destination, 8);
        assert_eq!(assign_native_i32("", &mut destination), NativeI32Assignment::EndOfInput);
        assert_eq!(destination, 8);
    }

    #[test]
    fn native_i32_assignment_keeps_eof_distinct_and_preserves_destination() {
        for text in ["", " \t", "+", "-", "0x", "-0X", "\0bad"] {
            let mut destination = 1000;
            assert_eq!(assign_native_i32(text, &mut destination), NativeI32Assignment::EndOfInput, "{text:?}");
            assert_eq!(destination, 1000);
        }
        for text in ["bad", ".5", "+ ", "0xg", "--"] {
            let mut destination = -7;
            assert_eq!(assign_native_i32(text, &mut destination), NativeI32Assignment::MatchingFailure, "{text:?}");
            assert_eq!(destination, -7);
        }
    }

    #[test]
    fn native_i32_assignment_stores_low_word_after_shared_signed_saturation() {
        for (text, expected) in [
            ("010", 8), ("0x20suffix", 32), ("-12trailing", -12),
            ("08", 0), ("12\0ignored", 12), ("2147483648", i32::MIN),
            ("4294967295", -1), ("4294967296", 0),
            ("9223372036854775808", -1), ("-9223372036854775809", 0),
        ] {
            let mut destination = 1000;
            assert_eq!(assign_native_i32(text, &mut destination), NativeI32Assignment::Match, "{text:?}");
            assert_eq!(destination, expected, "{text:?}");
        }
    }

    #[test]
    fn eof_retains_initialized_zero_but_matching_failure_is_refused() {
        for text in ["", " ", "\t\n\r\x0b\x0c", "+", "-", " \t-", "0x", "-0X", "\0bad"] {
            assert_eq!(parse_fixed5(text), Some(0), "{text:?}");
        }
        for text in ["bad", ".5", "+ ", "-\t", "0xg", "--", " \t.bad"] {
            assert_eq!(parse_fixed5(text), None, "{text:?}");
        }
    }

    #[test]
    fn integer_scan_accepts_prefixes_and_autodetects_radix() {
        for (text, raw) in [("1",100_000), ("010",800_000), ("0x1f",3_100_000),
            ("-0X1F",-3_100_000), ("08",0), ("019",100_000), ("1junk.25",125_000),
            ("  +12trailing",1_200_000), ("1e3",100_000), ("0b10",0),
            ("0x1.fp2",100_000), ("08.1",10_000), ("1\0.5",100_000)] {
            assert_eq!(parse_fixed5(text), Some(raw), "{text:?}");
        }
    }

    #[test]
    fn fraction_has_its_own_padded_five_byte_scan_and_sign() {
        for (text, raw) in [("1.234567",123_456), ("1.2",120_000), ("1.",100_000),
            ("010.00008",800_008), ("1.0012x",100_012), ("1.0x2",100_000), ("1.+0x2",100_032),
            ("1.-2",98_000), ("-1.-2",-98_000), ("1.+2",102_000),
            ("1.bad.5",100_000), ("1. 2",102_000), ("1.\0ignored",100_000),
            ("-0.5",-50_000), ("  -0.5",50_000), ("-0x0.5",-50_000),
            ("-0.-2",2_000), ("1.0000x",100_000), ("1.12.34",100_012)] {
            assert_eq!(parse_fixed5(text), Some(raw), "{text:?}");
        }
    }

    #[test]
    fn signed_scan_saturates_before_fixed_point_wrapping() {
        for (text, raw) in [("9223372036854775807",-100_000),
            ("9223372036854775808",-100_000), ("18446744073709551616",-100_000),
            ("-9223372036854775808",0), ("-9223372036854775809",0),
            ("-18446744073709551616",0), ("0xffffffffffffffff",-100_000),
            ("-0xffffffffffffffff.25",-25_000), ("9223372036854775808.25",-75_000),
            ("184467440737095.51615",-1), ("184467440737095.51616",0)] {
            assert_eq!(parse_fixed5(text), Some(raw), "{text:?}");
        }
        assert_eq!(parse_fixed5(&"9".repeat(4096)), Some(-100_000));
        assert_eq!(parse_fixed5(&format!("-{}", "9".repeat(4096))), Some(0));
    }
}
