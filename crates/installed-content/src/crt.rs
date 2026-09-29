//! The C runtime's integer scan, as the executable links it.
//!
//! `sscanf` (14029D000) goes through vsscanf (1425478D8) to the UCRT's
//! formatted_scan (14254681C). A signed `%i` or `%lli` conversion
//! (conversion dispatch 142546A2C -> 142546CCC, base 0, signed) parses into
//! an unsigned 64-bit value with `parse_integer` (142542E9C -> 142542A60)
//! and stores it through store_scanned_integer (1425477D0), which writes the
//! low 32 bits for `%i` and all 64 for `%lli`.
//!
//! `parse_integer` (142542A60):
//! - skips white space (the C locale's `isspace`: space, `\t` to `\r`), then
//!   one sign;
//! - base 0: `0x`/`0X` selects hexadecimal (the digits start after the `x`),
//!   another leading `0` octal, anything else decimal; digits are `0-9`,
//!   `a-z`, `A-Z` below the base;
//! - on 64-bit wrap, or a magnitude past the signed range (14253C738), sets
//!   `ERANGE` and returns `i64::MAX` for a positive and `i64::MIN` for a
//!   negative value; otherwise negates a negative one;
//! - with no digit it stores nothing. formatted_scan then returns EOF (-1)
//!   when the scan stopped at the end of the text (it read nothing but
//!   space, a sign or `0x`), else 0.
//!
//! The text ends at its first NUL, as the C string does.

/// What one signed integer conversion does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanned<T> {
    /// sscanf returns 1 and stores the value.
    Stored(T),
    /// sscanf returns EOF (-1) and stores nothing. The game's callers test
    /// `!= 0` (1424B02C0, 1424B0480), so they take this as success.
    End,
    /// sscanf returns 0 and stores nothing: a byte that is no digit
    /// stopped the scan before any digit.
    NoMatch,
}

/// `sscanf(text, "%lli", &value)`: the 64-bit signed conversion
/// (read_fixed5 1424B0480, format 142B7AB90).
pub fn scan_i64(bytes: &[u8]) -> Scanned<i64> {
    let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
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
    let mut magnitude = 0_u64;
    let mut wrapped = false;
    while let Some(digit) = bytes
        .get(cursor)
        .and_then(|byte| char::from(*byte).to_digit(36))
        .filter(|digit| *digit < radix)
    {
        let (product, over_multiply) = magnitude.overflowing_mul(u64::from(radix));
        let (sum, over_add) = product.overflowing_add(u64::from(digit));
        wrapped |= over_multiply || over_add;
        magnitude = sum;
        cursor += 1;
    }
    if cursor == first_digit {
        return if cursor == bytes.len() {
            Scanned::End
        } else {
            Scanned::NoMatch
        };
    }
    let limit = i64::MAX as u64 + u64::from(negative);
    Scanned::Stored(if wrapped || magnitude > limit {
        if negative { i64::MIN } else { i64::MAX }
    } else if negative {
        (magnitude as i64).wrapping_neg()
    } else {
        magnitude as i64
    })
}

/// `sscanf(text, "%i", &value)` into an `int` (assign_signed_integer32_token
/// 1424B02C0, format 142A24BE8): the same 64-bit conversion, of which
/// store_scanned_integer (1425477D0) keeps the low 32 bits, so
/// `2147483648` stores `i32::MIN` and a clamped `i64::MAX` stores -1.
pub fn scan_i32(bytes: &[u8]) -> Scanned<i32> {
    match scan_i64(bytes) {
        Scanned::Stored(value) => Scanned::Stored(value as i32),
        Scanned::End => Scanned::End,
        Scanned::NoMatch => Scanned::NoMatch,
    }
}

/// `sscanf(text, "%f", &value)` into a `float` (read_float_token 1424B0320,
/// format 1427179D8 `%f`): white space and a sign, then the longest decimal
/// prefix (digits with at most one `.`, and an `e`/`E` exponent when digits
/// follow it), correctly rounded to `f32`. Hexadecimal (`0x`), infinity and
/// NaN inputs are refused by name: the port does not follow the UCRT's parse
/// of them.
pub fn scan_f32(bytes: &[u8]) -> Result<Scanned<f32>, String> {
    let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
    let digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
    let mut at = 0;
    while bytes
        .get(at)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'..=b'\r'))
    {
        at += 1;
    }
    let start = at;
    if matches!(bytes.get(at), Some(b'+' | b'-')) {
        at += 1;
    }
    let rest = &bytes[at..];
    if rest.len() >= 2 && rest[0] == b'0' && matches!(rest[1], b'x' | b'X') {
        return Err("a hexadecimal floating form".to_owned());
    }
    if matches!(rest.first(), Some(b'i' | b'I' | b'n' | b'N')) {
        return Err("an infinity or NaN form".to_owned());
    }
    let mut mantissa = 0;
    while digit(at) {
        at += 1;
        mantissa += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while digit(at) {
            at += 1;
            mantissa += 1;
        }
    }
    if mantissa == 0 {
        return Ok(if rest.is_empty() {
            Scanned::End
        } else {
            Scanned::NoMatch
        });
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let mut exponent = at + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if digit(exponent) {
            while digit(exponent) {
                exponent += 1;
            }
            at = exponent;
        }
    }
    let text = std::str::from_utf8(&bytes[start..at]).map_err(|error| error.to_string())?;
    text.parse::<f32>()
        .map(Scanned::Stored)
        .map_err(|error| format!("{text}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_conversions_take_the_longest_decimal_prefix() {
        assert_eq!(scan_f32(b" 1.5tail"), Ok(Scanned::Stored(1.5)));
        assert_eq!(scan_f32(b"-2e2"), Ok(Scanned::Stored(-200.0)));
        assert_eq!(scan_f32(b"3e"), Ok(Scanned::Stored(3.0)));
        assert_eq!(scan_f32(b".25"), Ok(Scanned::Stored(0.25)));
        assert_eq!(scan_f32(b"7."), Ok(Scanned::Stored(7.0)));
        assert_eq!(scan_f32(b""), Ok(Scanned::End));
        assert_eq!(scan_f32(b" -"), Ok(Scanned::End));
        assert_eq!(scan_f32(b"x"), Ok(Scanned::NoMatch));
        assert!(scan_f32(b"0x1p3").is_err());
        assert!(scan_f32(b"inf").is_err());
    }

    #[test]
    fn signed_conversions_follow_the_ucrt() {
        assert_eq!(scan_i64(b" 010"), Scanned::Stored(8));
        assert_eq!(scan_i64(b"0x1F"), Scanned::Stored(31));
        assert_eq!(scan_i64(b"-0x10"), Scanned::Stored(-16));
        assert_eq!(scan_i64(b"09"), Scanned::Stored(0));
        assert_eq!(scan_i64(b"300.75"), Scanned::Stored(300));
        // No digit: EOF when the scan reached the end, else a mismatch.
        for text in [&b""[..], b"   ", b"-", b"+", b"0x", b" -0X"] {
            assert_eq!(scan_i64(text), Scanned::End, "{text:?}");
        }
        for text in [&b"x"[..], b"-x", b"0xg", b"@cost", b".5"] {
            assert_eq!(scan_i64(text), Scanned::NoMatch, "{text:?}");
        }
        // Past the signed range: clamped, whatever the wrap.
        assert_eq!(scan_i64(b"9223372036854775807"), Scanned::Stored(i64::MAX));
        assert_eq!(scan_i64(b"9223372036854775808"), Scanned::Stored(i64::MAX));
        assert_eq!(scan_i64(b"-9223372036854775808"), Scanned::Stored(i64::MIN));
        assert_eq!(scan_i64(b"-9223372036854775809"), Scanned::Stored(i64::MIN));
        assert_eq!(
            scan_i64(b"99999999999999999999999"),
            Scanned::Stored(i64::MAX)
        );
        assert_eq!(scan_i64(b"12\x0034"), Scanned::Stored(12));
    }

    #[test]
    fn a_32_bit_conversion_keeps_the_low_32_bits() {
        assert_eq!(scan_i32(b"2147483647"), Scanned::Stored(i32::MAX));
        assert_eq!(scan_i32(b"2147483648"), Scanned::Stored(i32::MIN));
        assert_eq!(scan_i32(b"4294967297"), Scanned::Stored(1));
        assert_eq!(scan_i32(b"-2147483649"), Scanned::Stored(i32::MAX));
        // Clamped to i64::MAX (low bits all set) and i64::MIN (low bits 0).
        assert_eq!(scan_i32(b"99999999999999999999"), Scanned::Stored(-1));
        assert_eq!(scan_i32(b"-99999999999999999999"), Scanned::Stored(0));
        assert_eq!(scan_i32(b"lots"), Scanned::NoMatch);
        assert_eq!(scan_i32(b""), Scanned::End);
    }
}
