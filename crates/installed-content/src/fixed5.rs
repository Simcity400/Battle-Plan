//! The engine's conversion of an authored script number to its fixed point with
//! five decimals: the text after the dot keeps only its first five digits, so
//! a value is truncated, never rounded.

/// `1424b0480`, the script fixed5 conversion: two signed `%lli`-style
/// auto-base scans, unlike save-file decimal parsing.
pub fn script_fixed5(bytes: &[u8]) -> Option<i64> {
    let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
    let whole = scan_signed_auto(bytes)?;
    let Some(dot) = bytes.iter().position(|byte| *byte == b'.') else {
        return Some(whole.wrapping_mul(100_000));
    };
    let mut fraction = [b'0'; 5];
    let suffix = &bytes[dot + 1..];
    let copied = suffix.len().min(5);
    fraction[..copied].copy_from_slice(&suffix[..copied]);
    let start = fraction.iter().position(|byte| *byte != b'0').unwrap_or(5);
    let mut part = scan_signed_auto(&fraction[start..]).unwrap_or(0);
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

fn scan_signed_auto(bytes: &[u8]) -> Option<i64> {
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
    let mut magnitude = 0u64;
    while let Some(digit) = bytes
        .get(cursor)
        .and_then(|byte| char::from(*byte).to_digit(36))
        .filter(|digit| u64::from(*digit) < radix)
    {
        magnitude = magnitude
            .saturating_mul(radix)
            .saturating_add(u64::from(digit));
        cursor += 1;
    }
    if cursor == first_digit {
        return (cursor == bytes.len()).then_some(0);
    }
    let bound = i64::MAX as u64 + u64::from(negative);
    let signed = magnitude.min(bound) as i64;
    Some(if negative {
        signed.wrapping_neg()
    } else {
        signed
    })
}
