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

fn scan_signed_auto(bytes: &[u8]) -> Option<i64> {
    let mut cursor = 0;
    while bytes.get(cursor).is_some_and(|byte| matches!(byte, b' ' | b'\t'..=b'\r')) {
        cursor += 1;
    }
    let negative = bytes.get(cursor) == Some(&b'-');
    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
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
        return (cursor == bytes.len()).then_some(0);
    }
    let bound = i64::MAX as u64 + u64::from(negative);
    let signed = magnitude.min(bound) as i64;
    Some(if negative { signed.wrapping_neg() } else { signed })
}

#[cfg(test)]
mod tests {
    use super::parse_fixed5;

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
