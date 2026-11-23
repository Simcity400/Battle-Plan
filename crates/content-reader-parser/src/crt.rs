
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

fn trim(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(text.len());
    &text[start..]
}

fn sign(text: &[u8]) -> (bool, &[u8]) {
    match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

fn digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanned<T> {
    Stored(T),
    End,
    NoMatch,
}

impl<T> Scanned<T> {
    pub fn stored(self) -> Option<T> {
        match self {
            Self::Stored(value) => Some(value),
            Self::End | Self::NoMatch => None,
        }
    }

    fn map<U>(self, change: impl FnOnce(T) -> U) -> Scanned<U> {
        match self {
            Self::Stored(value) => Scanned::Stored(change(value)),
            Self::End => Scanned::End,
            Self::NoMatch => Scanned::NoMatch,
        }
    }
}

fn parse_integer(bytes: &[u8], base: u32, signed: bool) -> Scanned<u64> {
    let (negative, mut rest) = sign(trim(crate::parser::c_string(bytes)));
    let mut radix = base;
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
    let mut magnitude = 0_u64;
    let mut wrapped = false;
    let mut read = false;
    for value in rest
        .iter()
        .map_while(|byte| digit(*byte).filter(|value| *value < radix))
    {
        let (product, over_multiply) = magnitude.overflowing_mul(u64::from(radix));
        let (sum, over_add) = product.overflowing_add(u64::from(value));
        wrapped |= over_multiply || over_add;
        magnitude = sum;
        read = true;
    }
    if !read {
        return if rest.is_empty() {
            Scanned::End
        } else {
            Scanned::NoMatch
        };
    }
    let limit = i64::MAX as u64 + u64::from(negative);
    Scanned::Stored(if signed && (wrapped || magnitude > limit) {
        if negative {
            i64::MIN as u64
        } else {
            i64::MAX as u64
        }
    } else if wrapped {
        u64::MAX
    } else if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    })
}

pub fn scan_i64(bytes: &[u8]) -> Scanned<i64> {
    parse_integer(bytes, 0, true).map(|value| value as i64)
}

pub fn scan_i32(bytes: &[u8]) -> Scanned<i32> {
    parse_integer(bytes, 0, true).map(|value| value as u32 as i32)
}

pub fn scan_d32(bytes: &[u8]) -> Scanned<i32> {
    parse_integer(bytes, 10, true).map(|value| value as u32 as i32)
}

#[cfg(test)]
#[path = "crt/tests.rs"]
mod decimal_tests;

pub fn scan_u32(bytes: &[u8]) -> Scanned<u32> {
    parse_integer(bytes, 10, false).map(|value| value as u32)
}

pub fn scan_x32(bytes: &[u8]) -> Scanned<u32> {
    parse_integer(bytes, 16, false).map(|value| value as u32)
}

pub fn truncate_f32(value: f32) -> i32 {
    if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

const MANTISSA_DIGITS: usize = 768;
const EXPONENT_LIMIT: i32 = 5200;

fn starts_with_word(text: &[u8], word: &[u8]) -> bool {
    text.len() >= word.len() && text[..word.len()].eq_ignore_ascii_case(word)
}

pub fn scan_f32(text: &[u8]) -> Option<f32> {
    let (negative, rest) = sign(trim(crate::parser::c_string(text)));
    let signed = |value: f32| if negative { -value } else { value };
    match rest.first() {
        Some(b'i' | b'I') => {
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

fn decimal_value(mantissa: &[u8], exponent: i64) -> f32 {
    let digits: String = mantissa
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    format!("0.{digits}e{exponent}")
        .parse::<f32>()
        .unwrap_or(f32::NAN)
}

fn hexadecimal_value(mantissa: &[u8], exponent: i64) -> f32 {
    let mut value = 0u64;
    let mut used = 0;
    for digit in mantissa.iter().take(15) {
        value = value << 4 | u64::from(*digit);
        used += 1;
    }
    let tail = mantissa[used..].iter().any(|digit| *digit != 0);
    round_to_f32(value, exponent - 4 * used as i64, tail)
}

fn round_to_f32(value: u64, exponent: i64, tail: bool) -> f32 {
    let shift = value.leading_zeros();
    let value = value << shift;
    let top = exponent - i64::from(shift) + 63;
    if top > 127 {
        return f32::INFINITY;
    }
    let kept = if top >= -126 { 24 } else { 24 - (-126 - top) };
    if kept <= 0 {
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
        f32::from_bits(kept_bits as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_conversions_follow_the_ucrt() {
        assert_eq!(scan_i64(b" 010"), Scanned::Stored(8));
        assert_eq!(scan_i64(b"0x1F"), Scanned::Stored(31));
        assert_eq!(scan_i64(b"-0x10"), Scanned::Stored(-16));
        assert_eq!(scan_i64(b"09"), Scanned::Stored(0));
        assert_eq!(scan_i64(b"300.75"), Scanned::Stored(300));
        for text in [&b""[..], b"   ", b"-", b"+", b"0x", b" -0X"] {
            assert_eq!(scan_i64(text), Scanned::End, "{text:?}");
        }
        for text in [&b"x"[..], b"-x", b"0xg", b"@cost", b".5"] {
            assert_eq!(scan_i64(text), Scanned::NoMatch, "{text:?}");
        }
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
        assert_eq!(scan_i32(b"99999999999999999999"), Scanned::Stored(-1));
        assert_eq!(scan_i32(b"-99999999999999999999"), Scanned::Stored(0));
        assert_eq!(scan_i32(b"lots"), Scanned::NoMatch);
        assert_eq!(scan_i32(b""), Scanned::End);
    }

    #[test]
    fn integers_store_the_low_32_bits_of_a_64_bit_scan() {
        let i32_of = |text: &[u8]| scan_i32(text).stored();
        assert_eq!(i32_of(b"2147483648"), Some(i32::MIN));
        assert_eq!(i32_of(b"-2147483649"), Some(i32::MAX));
        assert_eq!(i32_of(b"4294967297"), Some(1));
        assert_eq!(i32_of(b"0x1F"), Some(31));
        assert_eq!(i32_of(b"017"), Some(15));
        assert_eq!(i32_of(b"08"), Some(0));
        assert_eq!(i32_of(b"  -12px"), Some(-12));
        assert_eq!(i32_of(b"99999999999999999999"), Some(-1));
        assert_eq!(i32_of(b"-99999999999999999999"), Some(0));
        assert_eq!(i32_of(b"9223372036854775808"), Some(-1));
        assert_eq!(i32_of(b"-9223372036854775808"), Some(0));
        for nothing in [&b""[..], b"x", b"-", b"+", b"0x", b"0xg", b" ", b"\0 5"] {
            assert_eq!(i32_of(nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn unsigned_conversions_store_the_low_32_bits() {
        assert_eq!(scan_u32(b"4294967296"), Scanned::Stored(0));
        assert_eq!(scan_u32(b"4294967295"), Scanned::Stored(u32::MAX));
        assert_eq!(scan_u32(b"-1"), Scanned::Stored(u32::MAX));
        assert_eq!(scan_u32(b"99999999999999999999"), Scanned::Stored(u32::MAX));
        assert_eq!(scan_u32(b"0x10"), Scanned::Stored(0));
        assert_eq!(scan_x32(b"0x4ffffffff"), Scanned::Stored(0xFFFF_FFFF));
        assert_eq!(scan_x32(b"ff102030"), Scanned::Stored(0xFF10_2030));
        assert_eq!(scan_x32(b"0x"), Scanned::End);
        assert_eq!(scan_x32(b"0xg"), Scanned::NoMatch);
        assert_eq!(scan_x32(b"-1"), Scanned::Stored(u32::MAX));
        assert_eq!(scan_x32(b"12\x0034"), Scanned::Stored(0x12));
        for nothing in [&b""[..], b"x", b"-", b"+", b" ", b"\0 5", b"0xg"] {
            assert_eq!(scan_i32(nothing).stored(), None, "{nothing:?}");
        }
        assert_eq!(scan_i32(b"017"), Scanned::Stored(15));
        assert_eq!(scan_i32(b"  -12px"), Scanned::Stored(-12));
    }

    #[test]
    fn floats_follow_the_ucrt() {
        assert_eq!(scan_f32(b"1.5e1"), Some(15.0));
        assert_eq!(scan_f32(b"  2px"), Some(2.0));
        assert_eq!(scan_f32(b"-.5"), Some(-0.5));
        assert_eq!(scan_f32(b"0x1p3"), Some(8.0));
        assert_eq!(scan_f32(b"0x1.8"), Some(1.5));
        assert_eq!(scan_f32(b"0x1e"), Some(30.0));
        assert_eq!(scan_f32(b"-0x.8p-1"), Some(-0.25));
        assert_eq!(scan_f32(b"1e39"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"-1e39"), Some(f32::NEG_INFINITY));
        assert_eq!(scan_f32(b"1e-50"), Some(0.0));
        assert_eq!(scan_f32(b"1e99999"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"0e99999"), Some(0.0));
        assert_eq!(scan_f32(b"0x1p128"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"0x1p-149"), Some(f32::from_bits(1)));
        assert_eq!(scan_f32(b"0x1p-150"), Some(0.0));
        assert_eq!(scan_f32(b"0x1.8p-150"), Some(f32::from_bits(1)));
        assert_eq!(scan_f32(b"0x1.000001p0"), Some(1.0));
        assert_eq!(scan_f32(b"0x1.000003p0"), Some(1.000_000_2));
        assert_eq!(scan_f32(b"0x1.fffffffp127"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"16777217"), Some(16_777_216.0));
        assert_eq!(scan_f32(b"inf"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"-Infinity"), Some(f32::NEG_INFINITY));
        assert_eq!(scan_f32(b"infx"), Some(f32::INFINITY));
        assert_eq!(scan_f32(b"nan").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(scan_f32(b"-nan").map(f32::to_bits), Some(0xFFFF_FFFF));
        assert_eq!(scan_f32(b"nan()").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(scan_f32(b"nan(sx)").map(f32::to_bits), Some(0x7FFF_FFFF));
        assert_eq!(scan_f32(b"nan(snan)").map(f32::to_bits), Some(0x7F80_0001));
        assert_eq!(scan_f32(b"nan(IND)").map(f32::to_bits), Some(0xFFC0_0000));
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
            assert_eq!(scan_f32(nothing), None, "{nothing:?}");
        }
        let long = format!("1.{}9", "0".repeat(800));
        assert_eq!(scan_f32(long.as_bytes()), Some(1.0));
    }

    #[test]
    fn float_to_int_is_int_min_when_invalid() {
        assert_eq!(truncate_f32(-1.9), -1);
        assert_eq!(truncate_f32(f32::NAN), i32::MIN);
        assert_eq!(truncate_f32(f32::INFINITY), i32::MIN);
        assert_eq!(truncate_f32(3e9), i32::MIN);
        assert_eq!(truncate_f32(-2_147_483_648.0), i32::MIN);
    }
}
