//! Shared readers for the scalars inside a parsed Clausewitz document.
//!
//! Every reader names the field it could not read, so a content error points at
//! the key rather than at a byte offset in a file the caller no longer holds.

use std::io;

use crate::{
    crt,
    parser::{Item, Value},
};

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The scalar text of a value.
pub(crate) fn text<'a>(value: &'a Value, what: &str) -> io::Result<&'a str> {
    value
        .scalar_str()
        .ok_or_else(|| invalid(format!("{what} is not a text scalar")))
}

/// A scalar parsed as a finite decimal number.
pub(crate) fn number(value: &Value, what: &str) -> io::Result<f32> {
    let scalar = text(value, what)?;
    // Float fields use the shared C-runtime `%f` scanner.
    let parsed = crt::scan_f32(scalar.as_bytes())
        .ok_or_else(|| invalid(format!("{what} is not a number: {scalar:?}")))?;
    if !parsed.is_finite() {
        return Err(invalid(format!("{what} is not finite: {scalar:?}")));
    }
    Ok(parsed)
}

/// A `yes` or `no` flag; anything else is an error rather than a default.
pub(crate) fn boolean(value: &Value, what: &str) -> io::Result<bool> {
    // Boolean fields and standalone tokens share exact yes/no conversion. Keep this
    // adapter's named refusal when its consumer cannot follow a read.
    let scalar = text(value, what)?;
    crate::parser::read_bool(scalar.as_bytes())
        .ok_or_else(|| invalid(format!("{what} is not yes or no: {scalar:?}")))
}

/// The items of a block value.
pub fn block<'a>(value: &'a Value, what: &str) -> io::Result<&'a [Item]> {
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
pub(crate) fn scalar_list(value: &Value, what: &str) -> io::Result<Vec<u32>> {
    let mut list = Vec::new();
    for item in block(value, what)? {
        if item.key.is_some() {
            return Err(invalid(format!("{what} holds a keyed entry")));
        }
        let scalar = item
            .value
            .scalar()
            .ok_or_else(|| invalid(format!("{what} holds a non-scalar entry")))?;
        // Append a zeroed slot before scanning each signed integer token.
        list.push(crt::scan_i32(scalar).stored().unwrap_or(0) as u32);
    }
    Ok(list)
}

/// The key of an item as text, for keys the game writes as identifiers.
pub fn key_str<'a>(item: &'a Item, what: &str) -> io::Result<&'a str> {
    let key = item
        .key
        .as_deref()
        .ok_or_else(|| invalid(format!("{what} has an unnamed entry")))?;
    std::str::from_utf8(key).map_err(|_| invalid(format!("{what} has a non-UTF-8 key")))
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
        assert!(boolean(last(&items, b"b").unwrap(), "b").unwrap());
        assert_eq!(number(last(&items, b"c").unwrap(), "c").unwrap(), 1.5);
        assert_eq!(
            scalar_list(last(&items, b"d").unwrap(), "d").unwrap(),
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
    fn numeric_fields_use_the_crt_prefix_scan() {
        let items = items(b"n = { \"12tail\" } f = \"1.25tail\"");
        assert_eq!(scalar_list(last(&items, b"n").unwrap(), "n").unwrap(), [12]);
        assert_eq!(number(last(&items, b"f").unwrap(), "f").unwrap(), 1.25);
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
        assert!(
            number(last(&items, b"c").unwrap(), "c")
                .unwrap_err()
                .to_string()
                .contains("not finite")
        );
        assert!(scalar_list(last(&items, b"d").unwrap(), "d").is_err());
        assert!(
            scalar_list(last(&items, b"b").unwrap(), "b")
                .unwrap_err()
                .to_string()
                .contains("keyed entry")
        );
        assert_eq!(
            scalar_list(last(&items, b"e").unwrap(), "e").unwrap(),
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
            key_quoted: false,
            operator: Default::default(),
            value: Value::Scalar(b"1".to_vec()),
            kind: crate::parser::Kind::Bare,
        };
        assert!(key_str(&raw, "test").is_err());
    }
}
