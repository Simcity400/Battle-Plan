//! Shared readers for the scalars inside a parsed Clausewitz document.
//!
//! Every reader names the field it could not read, so a content error points at
//! the key rather than at a byte offset in a file the caller no longer holds.

use std::{io, str::FromStr};

use crate::parser::{Item, Value};

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The scalar text of a value.
pub(crate) fn text<'a>(value: &'a Value, what: &str) -> io::Result<&'a str> {
    value
        .scalar_str()
        .ok_or_else(|| invalid(format!("{what} is not a text scalar")))
}

/// A scalar parsed as an integer, with no rounding and no default.
pub(crate) fn integer<T: FromStr>(value: &Value, what: &str) -> io::Result<T> {
    let scalar = text(value, what)?;
    scalar
        .parse()
        .map_err(|_| invalid(format!("{what} is not an integer: {scalar:?}")))
}

/// A scalar parsed as a finite decimal number.
pub(crate) fn number(value: &Value, what: &str) -> io::Result<f64> {
    let scalar = text(value, what)?;
    let parsed: f64 = scalar
        .parse()
        .map_err(|_| invalid(format!("{what} is not a number: {scalar:?}")))?;
    if !parsed.is_finite() {
        return Err(invalid(format!("{what} is not finite: {scalar:?}")));
    }
    Ok(parsed)
}

/// A `yes` or `no` flag; anything else is an error rather than a default.
pub(crate) fn boolean(value: &Value, what: &str) -> io::Result<bool> {
    match text(value, what)? {
        "yes" => Ok(true),
        "no" => Ok(false),
        other => Err(invalid(format!("{what} is not yes or no: {other:?}"))),
    }
}

/// The items of a block value.
pub(crate) fn block<'a>(value: &'a Value, what: &str) -> io::Result<&'a [Item]> {
    value
        .items()
        .ok_or_else(|| invalid(format!("{what} is not a block")))
}

/// The last value under a key. The game's own files assign a key twice and
/// expect the later assignment to win, as `433-Rajputana.txt` does.
pub(crate) fn last<'a>(items: &'a [Item], key: &[u8]) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

/// Every value under a key, in source order.
pub(crate) fn all<'a>(items: &'a [Item], key: &'a [u8]) -> impl Iterator<Item = &'a Value> {
    items
        .iter()
        .filter(move |item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

/// A required value under a key.
pub(crate) fn require<'a>(items: &'a [Item], key: &str, what: &str) -> io::Result<&'a Value> {
    last(items, key.as_bytes()).ok_or_else(|| invalid(format!("{what} has no {key}")))
}

/// An unkeyed list of scalars, as `provinces = { 1 2 3 }` stores them.
pub(crate) fn scalar_list<T: FromStr>(value: &Value, what: &str) -> io::Result<Vec<T>> {
    let mut list = Vec::new();
    for item in block(value, what)? {
        if item.key.is_some() {
            return Err(invalid(format!("{what} holds a keyed entry")));
        }
        list.push(integer(&item.value, what)?);
    }
    Ok(list)
}

/// The key of an item as text, for keys the game writes as identifiers.
pub(crate) fn key_str<'a>(item: &'a Item, what: &str) -> io::Result<&'a str> {
    let key = item
        .key
        .as_deref()
        .ok_or_else(|| invalid(format!("{what} has an unnamed entry")))?;
    std::str::from_utf8(key).map_err(|_| invalid(format!("{what} has a non-UTF-8 key")))
}

/// A script scalar as the reader's fixed5 conversion (a729 `1424B0480`)
/// reads it: two signed `%lli`-style auto-base scans, the whole part and up
/// to five fraction digits. It is intentionally different from save-file
/// decimal parsing: `010` is octal and trailing junk ends a scan. `None` when
/// the whole part has no digits and is not empty.
pub(crate) fn script_fixed5(bytes: &[u8]) -> Option<i64> {
    let bytes = &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())];
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

/// [`script_fixed5`] of a scalar value, naming the field it cannot read.
pub(crate) fn fixed5(value: &Value, what: &str) -> io::Result<i64> {
    let scalar = value
        .scalar()
        .ok_or_else(|| invalid(format!("{what} is not a number")))?;
    script_fixed5(scalar).ok_or_else(|| {
        invalid(format!(
            "{what} is not a native fixed5 number: {:?}",
            String::from_utf8_lossy(scalar)
        ))
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
    while let Some(&byte) = bytes.get(cursor) {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'z' => u64::from(byte - b'a' + 10),
            b'A'..=b'Z' => u64::from(byte - b'A' + 10),
            _ => break,
        };
        if digit >= radix {
            break;
        }
        magnitude = magnitude.saturating_mul(radix).saturating_add(digit);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn items(bytes: &[u8]) -> Vec<Item> {
        parser::parse(bytes).unwrap().items
    }

    #[test]
    fn scalars_are_read_without_defaults_and_name_their_field() {
        let items = items(b"a = 5 b = yes c = 1.5 d = { 1 2 3 } a = 7 e = { x = 1 }");
        assert_eq!(
            integer::<u32>(all(&items, b"a").next().unwrap(), "a").unwrap(),
            5
        );
        assert_eq!(integer::<u32>(last(&items, b"a").unwrap(), "a").unwrap(), 7);
        assert!(boolean(last(&items, b"b").unwrap(), "b").unwrap());
        assert_eq!(number(last(&items, b"c").unwrap(), "c").unwrap(), 1.5);
        assert_eq!(
            scalar_list::<u32>(last(&items, b"d").unwrap(), "d").unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(all(&items, b"a").count(), 2);
        assert_eq!(block(last(&items, b"e").unwrap(), "e").unwrap().len(), 1);
        assert!(require(&items, "a", "test").is_ok());
        assert!(
            require(&items, "missing", "test")
                .unwrap_err()
                .to_string()
                .contains("test has no missing")
        );
    }

    #[test]
    fn wrong_shapes_and_values_are_refused_rather_than_coerced() {
        let items = items(b"a = maybe b = { x = 1 } c = 1e999 d = five e = { } f = 2");
        assert!(
            boolean(last(&items, b"a").unwrap(), "flag")
                .unwrap_err()
                .to_string()
                .contains("not yes or no")
        );
        assert!(integer::<u32>(last(&items, b"b").unwrap(), "b").is_err());
        assert!(
            number(last(&items, b"c").unwrap(), "c")
                .unwrap_err()
                .to_string()
                .contains("not finite")
        );
        assert!(integer::<u32>(last(&items, b"d").unwrap(), "d").is_err());
        assert!(
            scalar_list::<u32>(last(&items, b"b").unwrap(), "b")
                .unwrap_err()
                .to_string()
                .contains("keyed entry")
        );
        assert_eq!(
            scalar_list::<u32>(last(&items, b"e").unwrap(), "e").unwrap(),
            Vec::<u32>::new()
        );
        assert!(block(last(&items, b"f").unwrap(), "f").is_err());
        assert!(text(last(&items, b"b").unwrap(), "b").is_err());
    }

    #[test]
    fn keys_must_be_named_and_utf8() {
        let items = items(b"a = 1 { 2 }");
        assert_eq!(key_str(&items[0], "test").unwrap(), "a");
        assert!(
            key_str(&items[1], "test")
                .unwrap_err()
                .to_string()
                .contains("unnamed entry")
        );
        let raw = Item {
            key: Some(vec![0xff, 0xfe]),
            value: Value::Scalar(b"1".to_vec()),
        };
        assert!(key_str(&raw, "test").is_err());
    }
}
