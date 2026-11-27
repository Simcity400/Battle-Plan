
use std::{fmt, io, sync::Arc};

use crate::{
    crt,
    parser::{Item, Value},
};

#[derive(Debug)]
struct ContentError(String);

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContentError {}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, ContentError(message.into()))
}

pub fn is_content_error(error: &io::Error) -> bool {
    let mut inner: Option<&(dyn std::error::Error + 'static)> = error
        .get_ref()
        .map(|error| error as &(dyn std::error::Error + 'static));
    while let Some(current) = inner {
        if current.is::<ContentError>() {
            return true;
        }
        inner = if let Some(wrapped) = current.downcast_ref::<io::Error>() {
            wrapped
                .get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        } else if let Some(wrapped) = current.downcast_ref::<Arc<io::Error>>() {
            wrapped
                .get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        } else {
            current.source()
        };
    }
    false
}

pub fn text<'a>(value: &'a Value, what: &str) -> io::Result<&'a str> {
    value
        .scalar_str()
        .ok_or_else(|| invalid(format!("{what} is not a text scalar")))
}

pub fn number(value: &Value, what: &str) -> io::Result<f32> {
    let scalar = text(value, what)?;
    let parsed = crt::scan_f32(scalar.as_bytes())
        .ok_or_else(|| invalid(format!("{what} is not a number: {scalar:?}")))?;
    if !parsed.is_finite() {
        return Err(invalid(format!("{what} is not finite: {scalar:?}")));
    }
    Ok(parsed)
}

pub fn boolean(value: &Value, what: &str) -> io::Result<bool> {
    let scalar = text(value, what)?;
    crate::parser::read_bool(scalar.as_bytes())
        .ok_or_else(|| invalid(format!("{what} is not yes or no: {scalar:?}")))
}

pub fn block<'a>(value: &'a Value, what: &str) -> io::Result<&'a [Item]> {
    value
        .items()
        .ok_or_else(|| invalid(format!("{what} is not a block")))
}

pub fn last<'a>(items: &'a [Item], key: &[u8]) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

pub fn all<'a>(items: &'a [Item], key: &'a [u8]) -> impl Iterator<Item = &'a Value> {
    items
        .iter()
        .filter(move |item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

pub fn require<'a>(items: &'a [Item], key: &str, what: &str) -> io::Result<&'a Value> {
    last(items, key.as_bytes()).ok_or_else(|| invalid(format!("{what} has no {key}")))
}

pub fn scalar_list(value: &Value, what: &str) -> io::Result<Vec<u32>> {
    let mut list = Vec::new();
    for item in block(value, what)? {
        if item.key.is_some() {
            return Err(invalid(format!("{what} holds a keyed entry")));
        }
        let scalar = item
            .value
            .scalar()
            .ok_or_else(|| invalid(format!("{what} holds a non-scalar entry")))?;
        list.push(crt::scan_i32(scalar).stored().unwrap_or(0) as u32);
    }
    Ok(list)
}

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
        let items = items(b"n = { \"12tail\" five \"\" } f = \"1.25tail\"");
        assert_eq!(
            scalar_list(last(&items, b"n").unwrap(), "n").unwrap(),
            [12, 0, 0]
        );
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
        assert!(items[1].key_str().unwrap_err().to_string().contains("no key"));
        let raw = Item {
            key: Some(vec![0xff, 0xfe]),
            key_quoted: false,
            operator: Default::default(),
            value: Value::Scalar(b"1".to_vec()),
            kind: crate::parser::Kind::Bare,
        };
        assert!(key_str(&raw, "test").is_err());
    }

    #[test]
    fn content_error_marker_follows_parse_errors_through_wrapped_io_errors() {
        let field = invalid("broken field");
        assert_eq!(field.kind(), io::ErrorKind::InvalidData);
        assert!(is_content_error(&field));
        let wrapped = io::Error::new(field.kind(), field);
        assert!(is_content_error(&wrapped));
        let shared = io::Error::new(wrapped.kind(), Arc::new(wrapped));
        assert!(is_content_error(&shared));

        let parse = parser::parse(b"a = {").unwrap_err();
        assert_eq!(parse.kind(), io::ErrorKind::InvalidData);
        assert!(is_content_error(&parse));

        let decimal = crate::parser::decimal::Decimal::parse(b"not_a_number").unwrap_err();
        assert_eq!(decimal.kind(), io::ErrorKind::InvalidData);
        assert!(is_content_error(&decimal));

        let plain = io::Error::new(io::ErrorKind::InvalidData, "content file changed since verification");
        assert!(!is_content_error(&plain));
        assert!(!is_content_error(&io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(!is_content_error(&io::Error::from(io::ErrorKind::NotFound)));
    }
}
