//! A JSON tree that carries the SOURCE SPELLING of every scalar token.
//!
//! The certified replay cache stores one full world checkpoint every N hours
//! and bounded structural deltas in between, and every reconstructed hour must
//! come back as the *exact bytes* the `replay_decoder` fold emitted. A
//! `serde_json::Value` cannot carry that guarantee: it holds a decoded `f64`,
//! not the token that produced it, and `serde_json`'s default number parser is
//! deliberately not correctly-rounded (that is what its `float_roundtrip`
//! feature buys). On capture t26 hour 17032314 the fold emits
//!
//! ```text
//! "current":189352.52932739258
//! ```
//!
//! which reparses one ULP low and re-serializes as `189352.52932739255`
//! (`0x1.71d443c100000p+17` vs `0x1.71d443c0fffffp+17` — genuinely different
//! doubles, not merely a different spelling of one). Across all 644 sealed
//! hours of that capture — 16,337,177,524 bytes of world JSON — 510 of the
//! 2,990,242 distinct number tokens move under the round trip.
//!
//! Turning on `serde_json/float_roundtrip` was rejected: Cargo unifies features
//! across the workspace, so it would also change how the *fold* parses every
//! number it ingests, and therefore the canonical snapshot bytes that
//! `fold_check`'s G1 hashes are bound to. It also would not cover a token that
//! is simply not in `serde_json`'s output spelling (`1.50`, `1e3`, an integer
//! past `u64::MAX`).
//!
//! So the cache stores the spelling. [`CanonValue`] keeps every number as its
//! source token and every string as its source token *body* (the bytes between
//! the quotes, escapes and all); [`CanonValue::write_json`] emits those bytes
//! back verbatim in `serde_json`'s compact layout. The round trip is then
//! exact by construction for any compact JSON document, and
//! `ReplayCacheReader::verify_every_hour` *proves* it per hour over the sealed
//! artifact -- it re-serializes each reconstructed hour and requires the
//! `world_json_sha256` the encoder took from the SOURCE bytes -- so an
//! unanticipated token shape fails a build loudly instead of corrupting a seek
//! quietly.

use std::cmp::Ordering;

/// Deepest nesting a canonical world body may use.
pub(crate) const MAX_RECURSION_DEPTH: usize = 256;

/// One JSON value, with the source spelling of every scalar retained.
///
/// Objects are a sorted `Vec` rather than a map: the canonical fold emits
/// `serde_json`'s `BTreeMap`-backed member order, [`CanonValue::parse`] requires
/// strictly increasing member names, and every operation here is a merge walk
/// that preserves that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonValue {
    Null,
    Bool(bool),
    /// The exact source token, e.g. `189352.52932739258` or `-0.0`.
    Number(String),
    /// The exact source token BODY — the bytes between the quotes, with
    /// escapes left in their source spelling.
    String(String),
    Array(Vec<CanonValue>),
    /// `(name body, value)` in strictly increasing DECODED name order.
    Object(Vec<(String, CanonValue)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonParseError {
    message: String,
    offset: usize,
}

impl CanonParseError {
    fn new(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset,
        }
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl std::fmt::Display for CanonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for CanonParseError {}

impl CanonValue {
    /// Parse one complete JSON document, retaining scalar spellings.
    ///
    /// Stricter than a general JSON reader in one place: object member names
    /// must be strictly increasing by their DECODED text. That is the order
    /// `serde_json` emits from a `BTreeMap`, it is what the binary cache format
    /// has always required of a decoded record, and it makes duplicate members
    /// impossible.
    pub fn parse(bytes: &[u8]) -> Result<Self, CanonParseError> {
        let mut parser = Parser {
            bytes,
            position: 0,
        };
        parser.skip_whitespace();
        let value = parser.value(0)?;
        parser.skip_whitespace();
        if parser.position != bytes.len() {
            return Err(CanonParseError::new(
                "canonical world JSON has trailing bytes",
                parser.position,
            ));
        }
        Ok(value)
    }

    /// Append this value in `serde_json`'s compact layout, reusing the source
    /// spelling of every scalar.
    pub fn write_json(&self, out: &mut Vec<u8>) {
        match self {
            Self::Null => out.extend_from_slice(b"null"),
            Self::Bool(false) => out.extend_from_slice(b"false"),
            Self::Bool(true) => out.extend_from_slice(b"true"),
            Self::Number(token) => out.extend_from_slice(token.as_bytes()),
            Self::String(body) => {
                out.push(b'"');
                out.extend_from_slice(body.as_bytes());
                out.push(b'"');
            }
            Self::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    value.write_json(out);
                }
                out.push(b']');
            }
            Self::Object(members) => {
                out.push(b'{');
                for (index, (key, value)) in members.iter().enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    out.push(b'"');
                    out.extend_from_slice(key.as_bytes());
                    out.extend_from_slice(b"\":");
                    value.write_json(out);
                }
                out.push(b'}');
            }
        }
    }

    /// The complete compact JSON encoding of this value.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.json_len_hint());
        self.write_json(&mut out);
        out
    }

    fn json_len_hint(&self) -> usize {
        match self {
            Self::Null | Self::Bool(_) => 5,
            Self::Number(token) => token.len(),
            Self::String(body) => body.len() + 2,
            Self::Array(values) => {
                2 + values.len() + values.iter().map(Self::json_len_hint).sum::<usize>()
            }
            Self::Object(members) => {
                2 + members.len() * 4
                    + members
                        .iter()
                        .map(|(key, value)| key.len() + value.json_len_hint())
                        .sum::<usize>()
            }
        }
    }

    /// The member of an object body, by its decoded name.
    pub fn member(&self, key: &str) -> Option<&CanonValue> {
        match self {
            Self::Object(members) => members
                .binary_search_by(|(candidate, _)| key_cmp_decoded(candidate, key))
                .ok()
                .map(|position| &members[position].1),
            _ => None,
        }
    }

    /// This value as an `i64`, when its token spells one exactly.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(token) => token
                .parse::<i64>()
                .ok()
                // `str::parse` accepts `+1` and `007`; a canonical token does not.
                .filter(|value| *token == value.to_string()),
            _ => None,
        }
    }

    /// The DECODED text of a string value.
    pub fn as_decoded_str(&self) -> Option<String> {
        match self {
            Self::String(body) => decode_json_string_body(body),
            _ => None,
        }
    }
}

/// Decode a JSON string body (the bytes between the quotes) into its text.
///
/// `None` when the body is not a well-formed body, including an unpaired
/// surrogate escape — which the cache is happy to *carry* verbatim but cannot
/// give a `String` for.
pub fn decode_json_string_body(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            index += 1;
            let escape = *bytes.get(index)?;
            index += 1;
            match escape {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let high = hex4(bytes.get(index..index + 4)?)?;
                    index += 4;
                    let scalar = if (0xd800..0xdc00).contains(&high) {
                        if bytes.get(index..index + 2)? != b"\\u" {
                            return None;
                        }
                        index += 2;
                        let low = hex4(bytes.get(index..index + 4)?)?;
                        index += 4;
                        if !(0xdc00..0xe000).contains(&low) {
                            return None;
                        }
                        0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
                    } else if (0xdc00..0xe000).contains(&high) {
                        return None;
                    } else {
                        high
                    };
                    out.push(char::from_u32(scalar)?);
                }
                _ => return None,
            }
        } else {
            let start = index;
            while index < bytes.len() && bytes[index] != b'\\' {
                index += 1;
            }
            out.push_str(body.get(start..index)?);
        }
    }
    Some(out)
}

fn hex4(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for byte in bytes {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        value = (value << 4) | digit as u32;
    }
    Some(value)
}

/// A JSON number token, exactly as RFC 8259 spells one.
pub fn is_json_number_token(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut index = 0;
    if bytes.first() == Some(&b'-') {
        index = 1;
    }
    let integer_start = index;
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                index += 1;
            }
        }
        _ => return false,
    }
    if index == integer_start {
        return false;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

/// A JSON string body: no raw control character, no bare `"`, and every
/// backslash starting a defined escape.
///
/// A `u`-escape only has to be four hex digits. An unpaired surrogate is not
/// rejected here: it is a spelling a source may legitimately contain, and the
/// cache's whole job is to give those bytes back unchanged.
pub fn is_json_string_body(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => return false,
            byte if byte < 0x20 => return false,
            b'\\' => {
                index += 1;
                match bytes.get(index) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => index += 1,
                    Some(b'u') => {
                        index += 1;
                        match bytes.get(index..index + 4) {
                            Some(digits) if hex4(digits).is_some() => index += 4,
                            _ => return false,
                        }
                    }
                    _ => return false,
                }
            }
            _ => index += 1,
        }
    }
    true
}

/// An object member name: a well-formed string body that also decodes.
///
/// Member names are the one place the cache needs the DECODED text — it orders,
/// de-duplicates and merge-walks on it — so a name must be decodable. Escapes
/// are allowed; an unpaired surrogate is not, and cannot occur: `serde_json`
/// serializes a member name from a Rust `String`, which cannot hold one.
pub fn is_valid_object_key(key: &str) -> bool {
    is_json_string_body(key) && decode_json_string_body(key).is_some()
}

/// Order two member NAMES by their decoded text.
///
/// `serde_json` writes object members in `BTreeMap` order, which is the order of
/// the decoded names, not of their escaped spellings — and those disagree. A
/// name with no escape is its own decoding, which is every name in a 16 GB
/// census of real capture t26, so the common path is a plain byte compare.
pub fn key_cmp(left: &str, right: &str) -> Ordering {
    let left_escaped = left.as_bytes().contains(&b'\\');
    let right_escaped = right.as_bytes().contains(&b'\\');
    if !left_escaped && !right_escaped {
        return left.cmp(right);
    }
    let left_text = if left_escaped {
        decode_json_string_body(left)
    } else {
        Some(left.to_owned())
    };
    let right_text = if right_escaped {
        decode_json_string_body(right)
    } else {
        Some(right.to_owned())
    };
    match (left_text, right_text) {
        (Some(left_text), Some(right_text)) => left_text.cmp(&right_text),
        // Unreachable once `is_valid_object_key` has admitted both, and still a
        // total order if it somehow is not.
        _ => left.cmp(right),
    }
}

/// Compare a STORED member name against a plain, already-decoded name.
pub fn key_cmp_decoded(stored: &str, decoded: &str) -> Ordering {
    if !stored.as_bytes().contains(&b'\\') {
        return stored.cmp(decoded);
    }
    match decode_json_string_body(stored) {
        Some(text) => text.as_str().cmp(decoded),
        None => stored.cmp(decoded),
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn skip_whitespace(&mut self) {
        while matches!(
            self.bytes.get(self.position),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.position += 1;
        }
    }

    fn error<T>(&self, message: &str) -> Result<T, CanonParseError> {
        Err(CanonParseError::new(message, self.position))
    }

    fn literal(&mut self, text: &[u8]) -> Result<(), CanonParseError> {
        if self.bytes.get(self.position..self.position + text.len()) == Some(text) {
            self.position += text.len();
            Ok(())
        } else {
            self.error("canonical world JSON has an unrecognised literal")
        }
    }

    fn value(&mut self, depth: usize) -> Result<CanonValue, CanonParseError> {
        if depth > MAX_RECURSION_DEPTH {
            return self.error("canonical world JSON exceeds the recursion bound");
        }
        match self.bytes.get(self.position) {
            None => self.error("canonical world JSON ended early"),
            Some(b'n') => {
                self.literal(b"null")?;
                Ok(CanonValue::Null)
            }
            Some(b't') => {
                self.literal(b"true")?;
                Ok(CanonValue::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(CanonValue::Bool(false))
            }
            Some(b'"') => Ok(CanonValue::String(self.string_body()?)),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => Ok(CanonValue::Number(self.number_token()?)),
            Some(_) => self.error("canonical world JSON has an unexpected token"),
        }
    }

    /// The bytes between the quotes, verbatim.
    fn string_body(&mut self) -> Result<String, CanonParseError> {
        debug_assert_eq!(self.bytes.get(self.position), Some(&b'"'));
        self.position += 1;
        let start = self.position;
        loop {
            match self.bytes.get(self.position) {
                None => return self.error("canonical world JSON string is unterminated"),
                Some(b'"') => {
                    let body = &self.bytes[start..self.position];
                    self.position += 1;
                    let body = std::str::from_utf8(body).map_err(|_| {
                        CanonParseError::new("canonical world JSON string is not UTF-8", start)
                    })?;
                    if !is_json_string_body(body) {
                        return Err(CanonParseError::new(
                            "canonical world JSON string body is malformed",
                            start,
                        ));
                    }
                    return Ok(body.to_owned());
                }
                Some(b'\\') => {
                    // Skip the escape so an escaped quote does not end the body.
                    // `is_json_string_body` validates the escape's shape.
                    self.position += 2;
                }
                Some(byte) if *byte < 0x20 => {
                    return self.error("canonical world JSON string has a raw control byte")
                }
                Some(_) => self.position += 1,
            }
        }
    }

    fn number_token(&mut self) -> Result<String, CanonParseError> {
        let start = self.position;
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        while matches!(
            self.bytes.get(self.position),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.position += 1;
        }
        let token = std::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| CanonParseError::new("canonical world JSON number is not UTF-8", start))?;
        if !is_json_number_token(token) {
            return Err(CanonParseError::new(
                "canonical world JSON number token is malformed",
                start,
            ));
        }
        Ok(token.to_owned())
    }

    fn array(&mut self, depth: usize) -> Result<CanonValue, CanonParseError> {
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.position) == Some(&b']') {
            self.position += 1;
            return Ok(CanonValue::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.bytes.get(self.position) {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(CanonValue::Array(values));
                }
                _ => return self.error("canonical world JSON array is malformed"),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<CanonValue, CanonParseError> {
        self.position += 1;
        let mut members: Vec<(String, CanonValue)> = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.position) == Some(&b'}') {
            self.position += 1;
            return Ok(CanonValue::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.position) != Some(&b'"') {
                return self.error("canonical world JSON object member name is missing");
            }
            let key_offset = self.position;
            let key = self.string_body()?;
            if !is_valid_object_key(&key) {
                return Err(CanonParseError::new(
                    "canonical world JSON object member name does not decode",
                    key_offset,
                ));
            }
            if members
                .last()
                .is_some_and(|(previous, _)| key_cmp(previous, &key) != Ordering::Less)
            {
                return Err(CanonParseError::new(
                    "canonical world JSON object members are not strictly increasing",
                    key_offset,
                ));
            }
            self.skip_whitespace();
            if self.bytes.get(self.position) != Some(&b':') {
                return self.error("canonical world JSON object member has no ':'");
            }
            self.position += 1;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.bytes.get(self.position) {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(CanonValue::Object(members));
                }
                _ => return self.error("canonical world JSON object is malformed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every scalar shape whose `serde_json` parse -> serialize round trip is
    /// NOT the identity, and which the cache therefore has to carry as a
    /// spelling rather than a value.
    ///
    /// The first block is capture t26 verbatim. A census of all 644 sealed
    /// hours (16,337,177,524 bytes of world JSON) found 2,990,242 distinct
    /// number tokens, of which exactly 510 move under the round trip; all 510
    /// are plain fractional decimals losing one ULP to `serde_json`'s
    /// best-effort number parser, and `189352.52932739258` is hour 17032314
    /// byte 808222. That corpus contains no exponent-form, integer-form or
    /// escaped-string token that moves, so the second block is synthetic: those
    /// shapes are legal JSON the format must still survive if a future domain
    /// ever emits one.
    ///
    /// Every entry is asserted to actually move, in
    /// `replay_cache::tests::number_spellings_serde_json_cannot_reproduce_survive_a_seek`,
    /// so the fixture cannot quietly decay into a tautology. `-0.0`,
    /// `1.0000000000000002` and `1.6e-6` were tried and are NOT here: they
    /// round-trip through `serde_json` unchanged and so witness nothing.
    pub(crate) const NON_IDENTITY_TOKENS: &[&str] = &[
        // 1-ULP parse loss (serde_json without `float_roundtrip`). Real t26.
        "189352.52932739258",
        "11268.982520000001",
        "26914.271699999998",
        "10.069999694824219",
        "10.100000381469727",
        "9.930000305175781",
        // Spellings serde_json's serializer never produces, so a value round
        // trip cannot return them even when the parse is exact.
        "1.50",
        "1e3",
        "1E3",
        "1e+3",
        "0.0000000000000000000000001",
        // Past i64/u64: `serde_json::Number` cannot even hold these without
        // `arbitrary_precision`, so a `Value` round trip is not merely lossy.
        "18446744073709551616",
        "-9223372036854775809",
        "123456789012345678901234567890",
    ];

    #[test]
    fn every_non_identity_token_survives_the_spelling_round_trip() {
        for token in NON_IDENTITY_TOKENS {
            assert!(is_json_number_token(token), "{token} must be a JSON number");
            let source = format!(r#"{{"v":{token}}}"#);
            let parsed = CanonValue::parse(source.as_bytes()).unwrap();
            assert_eq!(
                String::from_utf8(parsed.to_json_bytes()).unwrap(),
                source,
                "token {token} lost its spelling"
            );
        }
    }

    #[test]
    fn string_bodies_keep_their_source_escapes() {
        for body in [
            r#"plain"#,
            r#"quote \" backslash \\ slash \/"#,
            r#"\b\f\n\r\t"#,
            "caf\u{e9} \u{4e2d}\u{6587}",
            r#"😀"#,
            // Unpaired surrogate: carried verbatim, never decoded.
            r#"\ud800"#,
        ] {
            let source = format!(r#"{{"v":"{body}"}}"#);
            let parsed = CanonValue::parse(source.as_bytes()).unwrap();
            assert_eq!(String::from_utf8(parsed.to_json_bytes()).unwrap(), source);
        }
    }

    #[test]
    fn malformed_documents_fail_closed() {
        for (source, needle) in [
            (r#"{"b":1,"a":2}"#, "strictly increasing"),
            (r#"{"a":1,"a":2}"#, "strictly increasing"),
            (r#"{"a\ud800":1}"#, "does not decode"),
            (r#"{"a":01}"#, "malformed"),
            (r#"{"a":1.}"#, "malformed"),
            (r#"{"a":.1}"#, "unexpected token"),
            (r#"{"a":1e}"#, "malformed"),
            (r#"{"a":+1}"#, "unexpected token"),
            (r#"{"a":"\q"}"#, "malformed"),
            (r#"{"a":1}x"#, "trailing bytes"),
            (r#"{"a":1"#, "malformed"),
        ] {
            let error = CanonValue::parse(source.as_bytes())
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "{source}: {error}");
        }
    }

    /// An object member name MAY carry an escape. `serde_json` writes members in
    /// decoded-name order, which is not the order of their escaped spellings, so
    /// the cache orders on the decoded name and stores the spelling.
    #[test]
    fn escaped_member_names_are_accepted_ordered_by_their_decoded_text() {
        // Decoded, `a"b` (0x22 in the middle) sorts before `aXb` (0x58) - which
        // is what serde_json writes. As raw bodies the escaped name carries
        // 0x5C at that position, GREATER than 0x58, so a byte-order check on
        // the stored spelling would call this document unsorted.
        let source = r#"{"a\"b":1,"aXb":2}"#;
        let parsed = CanonValue::parse(source.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(parsed.to_json_bytes()).unwrap(), source);
        assert_eq!(parsed.member("a\"b").and_then(CanonValue::as_i64), Some(1));
        assert_eq!(parsed.member("aXb").and_then(CanonValue::as_i64), Some(2));

        // And serde_json agrees this is the order it would have written.
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), source);

        // Two spellings of ONE name are a duplicate member, not two members.
        let error = CanonValue::parse(r#"{"a":1,"a":2}"#.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("strictly increasing"), "{error}");
    }

    #[test]
    fn whitespace_parses_but_does_not_round_trip_to_itself() {
        let value = CanonValue::parse(b" { \"a\" : [ 1 , 2 ] } ").unwrap();
        assert_eq!(value.to_json_bytes(), br#"{"a":[1,2]}"#);
    }

    #[test]
    fn identity_accessors_read_the_spelling() {
        let value = CanonValue::parse(br#"{"date":"1936.1.1.12","hour_index":17032314}"#).unwrap();
        assert_eq!(value.member("hour_index").unwrap().as_i64(), Some(17032314));
        assert_eq!(
            value.member("date").unwrap().as_decoded_str().as_deref(),
            Some("1936.1.1.12")
        );
        assert_eq!(value.member("absent"), None);
        // A non-canonical integer spelling is not an i64 the index may bind to.
        let odd = CanonValue::parse(br#"{"hour_index":1.0}"#).unwrap();
        assert_eq!(odd.member("hour_index").unwrap().as_i64(), None);
    }

    #[test]
    fn decoded_strings_resolve_escapes_and_reject_lone_surrogates() {
        assert_eq!(
            decode_json_string_body(r#"aé\n\/"#).as_deref(),
            Some("a\u{e9}\n/")
        );
        assert_eq!(
            decode_json_string_body(r#"😀"#).as_deref(),
            Some("\u{1f600}")
        );
        assert_eq!(decode_json_string_body(r#"\ud800"#), None);
        assert_eq!(decode_json_string_body(r#"\ud800A"#), None);
        assert!(!is_valid_object_key(r#"\ud800"#));
        assert!(is_valid_object_key(r#"a\"b"#));
    }
}

#[cfg(test)]
pub(crate) use tests::NON_IDENTITY_TOKENS;
