//! The C runtime conversions the game's readers store through, as its statically linked UCRT
//! performs them: `sscanf@14029D000` with `%i`, `%u`, `%x` and `%f`, `atoi` and the float-to-int
//! instructions. Each returns what the conversion stores, or `None` when it stores nothing.
//!
//! Every conversion reads its text as a C string (`c_str`): a NUL byte ends it.
//!
//! - `%i`, `%u` and `%x` (`ReadScriptSignedInt32@1424AB970`, `ReadScriptUnsignedInt32@1424AB9A0`,
//!   `1425477D0`) and store the low 32 bits of the result. After the leading space and a sign,
//!   `%i` takes hexadecimal after `0x`, octal after a leading `0`, else decimal; `%x` skips a
//!   `0x`. A 64-bit overflow saturates: a signed conversion to `i64::MAX` or `i64::MIN`, an
//!   unsigned one to `u64::MAX`; otherwise a `-` negates the 64-bit value. So `2147483648` is
//!   `i32::MIN`, `%u 4294967296` is 0 and `%x 0x4ffffffff` is `0xffffffff`.
//! - A conversion that reads no digit stores nothing: the text matches nothing, or the scanf
//!   input's single-character push-back cannot restore what it consumed (a lone sign, `0x` with no
//!   hexadecimal digit after it).
//! - `%f` (`read_float_token@1424B0320`, into a `float`) is the UCRT's `parse_floating_point`
//!   through the same push-back: decimal and hexadecimal (`0x`, a `p` exponent) mantissas of up to
//!   768 significant digits, `inf`, `infinity`, `nan`, `nan(...)`, `nan(snan)` and `nan(ind)`,
//!   rounded once to the nearest `float` (ties to even), an overflow storing an infinity and an
//!   underflow a zero. A partial `infinity`, an exponent marker with no digit, `0x` with no digit
//!   and an unclosed or malformed `nan(` consumed more than the push-back restores: nothing is
//!   stored.
//! - `atoi` (`parse_signed_decimal32@14254AF28` through `14253C770`) reads a sign and decimal
//!   digits at 32 bits and saturates to `i32::MIN` or `i32::MAX`; 0 when no digit follows.
//! - `cvttss2si` gives `i32::MIN` for a value it cannot convert (a NaN, an infinity, anything
//!   outside `i32`), where Rust's `as` saturates.

/// The text a C string reader sees: the bytes before the first NUL.
pub(super) fn c_str(text: &[u8]) -> &[u8] {
    text.iter()
        .position(|byte| *byte == 0)
        .map_or(text, |end| &text[..end])
}

/// The C locale's `isspace`: space and tab to carriage return.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// The text after the leading space.
fn trim(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(text.len());
    &text[start..]
}

/// A sign, and the text after it.
fn sign(text: &[u8]) -> (bool, &[u8]) {
    match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

/// The UCRT's `parse_digit`: `0`-`9`, then an ASCII letter from 10; anything else is no digit.
fn digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

/// The UCRT's `parse_integer<uint64_t>` as a scanf conversion: `radix` 0 detects the base, 16
/// skips a `0x`; `None` when it reads no digit.
fn integer(text: &[u8], radix: u32, signed: bool) -> Option<u64> {
    let (negative, mut rest) = sign(trim(c_str(text)));
    let mut radix = radix;
    if radix == 0 || radix == 16 {
        match rest {
            [b'0', b'x' | b'X', tail @ ..] => {
                radix = 16;
                rest = tail;
            }
            [b'0', ..] if radix == 0 => radix = 8,
            _ if radix == 0 => radix = 10,
            _ => {}
        }
    }
    let mut number = 0u64;
    let mut overflow = false;
    let mut read = false;
    for byte in rest {
        let Some(value) = digit(*byte).filter(|value| *value < radix) else {
            break;
        };
        read = true;
        match number
            .checked_mul(u64::from(radix))
            .and_then(|number| number.checked_add(u64::from(value)))
        {
            Some(next) => number = next,
            None => {
                overflow = true;
                number = number
                    .wrapping_mul(u64::from(radix))
                    .wrapping_add(u64::from(value));
            }
        }
    }
    if !read {
        return None;
    }
    let limit = if negative {
        1u64 << 63
    } else {
        i64::MAX.unsigned_abs()
    };
    Some(if signed && (overflow || number > limit) {
        if negative {
            i64::MIN as u64
        } else {
            i64::MAX as u64
        }
    } else if overflow {
        u64::MAX
    } else if negative {
        number.wrapping_neg()
    } else {
        number
    })
}

/// `%i`: the low 32 bits of the signed conversion.
pub(super) fn i32_of(text: &[u8]) -> Option<i32> {
    integer(text, 0, true).map(|value| value as u32 as i32)
}

/// `%u`: the low 32 bits of the unsigned decimal conversion.
pub(super) fn u32_of(text: &[u8]) -> Option<u32> {
    integer(text, 10, false).map(|value| value as u32)
}

/// The UCRT's mantissa buffer (`floating_point_string::_mantissa`): digits past it are dropped.
const MANTISSA_DIGITS: usize = 768;
/// The UCRT's `maximum_temporary_decimal_exponent`; the minimum is its negation.
const EXPONENT_LIMIT: i32 = 5200;

/// Whether `text` starts with `word`, compared without ASCII case.
fn starts_with_word(text: &[u8], word: &[u8]) -> bool {
    text.len() >= word.len() && text[..word.len()].eq_ignore_ascii_case(word)
}

/// `%f` into a `float`: the value stored, `None` when nothing is.
pub(super) fn f32_of(text: &[u8]) -> Option<f32> {
    let (negative, rest) = sign(trim(c_str(text)));
    let signed = |value: f32| if negative { -value } else { value };
    match rest.first() {
        Some(b'i' | b'I') => {
            // `inf`, then `inity` or nothing of it: a partial match consumed more than the
            // push-back restores.
            if !starts_with_word(rest, b"inf") {
                return None;
            }
            let tail = &rest[3..];
            let matched = tail
                .iter()
                .zip(b"inity")
                .take_while(|(byte, want)| byte.eq_ignore_ascii_case(want))
                .count();
            return (matched == 0 || matched == 5).then(|| signed(f32::INFINITY));
        }
        Some(b'n' | b'N') => return nan(rest, negative),
        _ => {}
    }
    let (hexadecimal, mut rest) = match rest {
        [b'0', b'x' | b'X', tail @ ..] => (true, tail),
        _ => (false, rest),
    };
    let radix = if hexadecimal { 16 } else { 10 };
    let is_digit = |byte: u8| digit(byte).is_some_and(|value| value < radix);
    let mut mantissa: Vec<u8> = Vec::new();
    let mut adjustment = 0i64;
    let mut found = false;
    while rest.first() == Some(&b'0') {
        found = true;
        rest = &rest[1..];
    }
    while let Some(&byte) = rest.first().filter(|byte| is_digit(**byte)) {
        found = true;
        if mantissa.len() < MANTISSA_DIGITS {
            mantissa.push(digit(byte).unwrap_or(0) as u8);
        }
        adjustment += 1;
        rest = &rest[1..];
    }
    if rest.first() == Some(&b'.') {
        rest = &rest[1..];
        if mantissa.is_empty() {
            while rest.first() == Some(&b'0') {
                found = true;
                adjustment -= 1;
                rest = &rest[1..];
            }
        }
        while let Some(&byte) = rest.first().filter(|byte| is_digit(**byte)) {
            found = true;
            if mantissa.len() < MANTISSA_DIGITS {
                mantissa.push(digit(byte).unwrap_or(0) as u8);
            }
            rest = &rest[1..];
        }
    }
    if !found {
        return None;
    }
    let mut exponent = 0i32;
    let marker: &[u8] = if hexadecimal { b"pP" } else { b"eE" };
    if rest.first().is_some_and(|byte| marker.contains(byte)) {
        let (exponent_negative, tail) = sign(&rest[1..]);
        let count = tail.iter().take_while(|byte| byte.is_ascii_digit()).count();
        if count == 0 {
            // The marker and what follows it cannot be pushed back.
            return None;
        }
        for byte in &tail[..count] {
            exponent = exponent * 10 + i32::from(byte - b'0');
            if exponent > EXPONENT_LIMIT {
                exponent = EXPONENT_LIMIT + 1;
                break;
            }
        }
        if exponent_negative {
            exponent = -exponent;
        }
    }
    while mantissa.last() == Some(&0) {
        mantissa.pop();
    }
    if mantissa.is_empty() {
        return Some(signed(0.0));
    }
    let range = |exponent: i64| {
        if exponent > i64::from(EXPONENT_LIMIT) {
            Some(signed(f32::INFINITY))
        } else if exponent < -i64::from(EXPONENT_LIMIT) {
            Some(signed(0.0))
        } else {
            None
        }
    };
    if let Some(value) = range(i64::from(exponent)) {
        return Some(value);
    }
    let exponent = i64::from(exponent) + adjustment * if hexadecimal { 4 } else { 1 };
    if let Some(value) = range(exponent) {
        return Some(value);
    }
    Some(signed(if hexadecimal {
        hexadecimal_value(&mantissa, exponent)
    } else {
        decimal_value(&mantissa, exponent)
    }))
}

/// `nan`, `nan(...)`, `nan(snan)` and `nan(ind)` after the sign.
fn nan(rest: &[u8], negative: bool) -> Option<f32> {
    let sign_bit = if negative { 0x8000_0000 } else { 0 };
    let quiet = f32::from_bits(sign_bit | 0x7FFF_FFFF);
    if !starts_with_word(rest, b"nan") {
        return None;
    }
    let tail = &rest[3..];
    let Some((b'(', inner)) = tail.split_first() else {
        return Some(quiet);
    };
    // Each special spelling is compared from where the last stopped.
    let mut at = 0;
    let special = |at: &mut usize, word: &[u8]| {
        for want in word {
            match inner.get(*at) {
                Some(byte) if byte.eq_ignore_ascii_case(want) => *at += 1,
                _ => return false,
            }
        }
        true
    };
    if special(&mut at, b"snan)") {
        return Some(f32::from_bits(sign_bit | 0x7F80_0001));
    }
    if special(&mut at, b"ind)") {
        return Some(f32::from_bits(0xFFC0_0000));
    }
    loop {
        match inner.get(at) {
            Some(b')') => return Some(quiet),
            None => return None,
            Some(byte) if byte.is_ascii_alphanumeric() || *byte == b'_' => at += 1,
            Some(_) => return None,
        }
    }
}

/// `0.mantissa * 10^exponent`, rounded once to the nearest `float`.
fn decimal_value(mantissa: &[u8], exponent: i64) -> f32 {
    let digits: String = mantissa
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    format!("0.{digits}e{exponent}")
        .parse::<f32>()
        .unwrap_or(f32::NAN)
}

/// `0x0.mantissa * 2^exponent`, rounded once to the nearest `float` (the UCRT's
/// `assemble_floating_point_value`: ties to even, the dropped digits a sticky tail).
fn hexadecimal_value(mantissa: &[u8], exponent: i64) -> f32 {
    let mut value = 0u64;
    let mut used = 0;
    for digit in mantissa.iter().take(15) {
        value = value << 4 | u64::from(*digit);
        used += 1;
    }
    let tail = mantissa[used..].iter().any(|digit| *digit != 0);
    // value * 2^(exponent - 4 * used), with `tail` below its last bit.
    round_to_f32(value, exponent - 4 * used as i64, tail)
}

/// `value * 2^exponent` (positive, `value` nonzero) with a sticky bit below `value`, rounded to
/// the nearest `float`, ties to even.
fn round_to_f32(value: u64, exponent: i64, tail: bool) -> f32 {
    let shift = value.leading_zeros();
    let value = value << shift;
    // The value is 1.f * 2^top.
    let top = exponent - i64::from(shift) + 63;
    if top > 127 {
        return f32::INFINITY;
    }
    // Bits kept: 24 for a normal value, fewer below 2^-126.
    let kept = if top >= -126 { 24 } else { 24 - (-126 - top) };
    if kept <= 0 {
        // Below half the smallest subnormal, or at most half of it: a tie rounds to even (0).
        let half = kept == 0 && (value > 1 << 63 || (value == 1 << 63 && tail));
        return if half { f32::from_bits(1) } else { 0.0 };
    }
    let drop = 64 - kept as u32;
    let mut kept_bits = value >> drop;
    let rest = value & ((1u64 << drop) - 1);
    let half = 1u64 << (drop - 1);
    if rest > half || (rest == half && (tail || kept_bits & 1 == 1)) {
        kept_bits += 1;
    }
    let mut top = top;
    if top >= -126 {
        if kept_bits == 1 << 24 {
            kept_bits >>= 1;
            top += 1;
            if top > 127 {
                return f32::INFINITY;
            }
        }
        let biased = (top + 127) as u32;
        f32::from_bits(biased << 23 | (kept_bits as u32 & 0x7F_FFFF))
    } else {
        // A subnormal: the kept bits are the fraction; rounding up to 2^23 is the smallest
        // normal, which the same bits spell.
        f32::from_bits(kept_bits as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `%i`, `%u` and `%x` accumulate at 64 bits and store the low 32 (`142542A60`,
    /// `1425477D0`); a 64-bit overflow saturates; no digit stores nothing.
    #[test]
    fn integers_store_the_low_32_bits_of_a_64_bit_scan() {
        assert_eq!(i32_of(b"2147483648"), Some(i32::MIN));
        assert_eq!(i32_of(b"-2147483649"), Some(i32::MAX));
        assert_eq!(i32_of(b"4294967297"), Some(1));
        assert_eq!(i32_of(b"0x1F"), Some(31));
        assert_eq!(i32_of(b"017"), Some(15));
        assert_eq!(i32_of(b"08"), Some(0));
        assert_eq!(i32_of(b"  -12px"), Some(-12));
        // Past 64 bits: `i64::MAX`, whose low 32 bits are -1, or `i64::MIN`, whose are 0.
        assert_eq!(i32_of(b"99999999999999999999"), Some(-1));
        assert_eq!(i32_of(b"-99999999999999999999"), Some(0));
        assert_eq!(i32_of(b"9223372036854775808"), Some(-1));
        assert_eq!(i32_of(b"-9223372036854775808"), Some(0));
        for nothing in [&b""[..], b"x", b"-", b"+", b"0x", b"0xg", b" ", b"\0 5"] {
            assert_eq!(i32_of(nothing), None, "{nothing:?}");
        }
        assert_eq!(u32_of(b"4294967296"), Some(0));
        assert_eq!(u32_of(b"4294967295"), Some(u32::MAX));
        assert_eq!(u32_of(b"-1"), Some(u32::MAX));
        assert_eq!(u32_of(b"99999999999999999999"), Some(u32::MAX));
        assert_eq!(u32_of(b"0x10"), Some(0));
    }

    /// `%f`: decimal, hexadecimal, the named values, overflow to an infinity, and the forms that
    /// store nothing.
    #[test]
    fn floats_follow_the_ucrt() {
        assert_eq!(f32_of(b"1.5e1"), Some(15.0));
        assert_eq!(f32_of(b"  2px"), Some(2.0));
        assert_eq!(f32_of(b"-.5"), Some(-0.5));
        assert_eq!(f32_of(b"0x1p3"), Some(8.0));
        assert_eq!(f32_of(b"0x1.8"), Some(1.5));
        assert_eq!(f32_of(b"0x1e"), Some(30.0));
        assert_eq!(f32_of(b"-0x.8p-1"), Some(-0.25));
        assert_eq!(f32_of(b"1e39"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"-1e39"), Some(f32::NEG_INFINITY));
        assert_eq!(f32_of(b"1e-50"), Some(0.0));
        assert_eq!(f32_of(b"1e99999"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"0e99999"), Some(0.0));
        assert_eq!(f32_of(b"0x1p128"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"0x1p-149"), Some(f32::from_bits(1)));
        assert_eq!(f32_of(b"0x1p-150"), Some(0.0));
        assert_eq!(f32_of(b"0x1.8p-150"), Some(f32::from_bits(1)));
        assert_eq!(f32_of(b"0x1.000001p0"), Some(1.0));
        assert_eq!(f32_of(b"0x1.000003p0"), Some(1.000_000_2));
        assert_eq!(f32_of(b"0x1.fffffffp127"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"16777217"), Some(16_777_216.0));
        assert_eq!(f32_of(b"inf"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"-Infinity"), Some(f32::NEG_INFINITY));
        assert_eq!(f32_of(b"infx"), Some(f32::INFINITY));
        assert_eq!(f32_of(b"nan").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(f32_of(b"-nan").map(f32::to_bits), Some(0xFFFF_FFFF));
        assert_eq!(f32_of(b"nan()").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(f32_of(b"nan(sx)").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(f32_of(b"nan(snan)").map(f32::to_bits), Some(0x7F80_0001));
        assert_eq!(f32_of(b"nan(IND)").map(f32::to_bits), Some(0xFFC0_0000));
        for nothing in [
            &b""[..],
            b".",
            b"-",
            b"x",
            b"1e",
            b"1e+",
            b"1ex",
            b"0x",
            b"0xp1",
            b"0x1p",
            b"i",
            b"infin",
            b"infinit",
            b"na",
            b"nan(",
            b"nan(a-b)",
            b"nan(snan",
        ] {
            assert_eq!(f32_of(nothing), None, "{nothing:?}");
        }
        // 768 significant digits are kept; the rest are dropped, not rounded.
        let long = format!("1.{}9", "0".repeat(800));
        assert_eq!(f32_of(long.as_bytes()), Some(1.0));
    }
}
