use std::cmp::Ordering;
use std::sync::Arc;

pub(crate) const MAX_RECURSION_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Arc<Vec<CanonValue>>),
    Object(Arc<Vec<(String, CanonValue)>>),
}

impl CanonValue {
    pub fn array(items: Vec<CanonValue>) -> CanonValue {
        CanonValue::Array(Arc::new(items))
    }

    pub fn object(members: Vec<(String, CanonValue)>) -> CanonValue {
        CanonValue::Object(Arc::new(members))
    }

    /// An ABI number, deliberately, like the rest of `heap_bytes`: this asks
    const fn shared_container_bytes<T>() -> u64 {
        (std::mem::size_of::<usize>() * 2 + std::mem::size_of::<T>()) as u64
    }

    pub fn shares_storage_with(&self, other: &CanonValue) -> bool {
        match (self, other) {
            (CanonValue::Array(left), CanonValue::Array(right)) => Arc::ptr_eq(left, right),
            (CanonValue::Object(left), CanonValue::Object(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
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
    pub fn parse(bytes: &[u8]) -> Result<Self, CanonParseError> {
        Self::parse_counted(bytes).map(|(value, _)| value)
    }

    pub fn parse_counted(bytes: &[u8]) -> Result<(Self, u64), CanonParseError> {
        let mut parser = Parser {
            bytes,
            position: 0,
            heap_bytes: 0,
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
        Ok((value, parser.heap_bytes))
    }

    pub const ARRAY_ELEMENT_WEIGHT: u64 = 16;
    pub const OBJECT_MEMBER_WEIGHT: u64 = 32;

    /// slot at `size_of::<CanonValue>()`, which is an ABI number: it can differ
    /// an ABI-dependent budget could cross its ceiling at a different node on
    pub fn content_bytes(&self) -> u64 {
        let mut total = 0u64;
        let mut stack = vec![self];
        while let Some(value) = stack.pop() {
            total = total.saturating_add(match value {
                Self::Null | Self::Bool(_) => 0,
                Self::Number(token) | Self::String(token) => token.len() as u64,
                Self::Array(items) => {
                    stack.extend(items.iter());
                    items.len() as u64 * Self::ARRAY_ELEMENT_WEIGHT
                }
                Self::Object(members) => {
                    stack.extend(members.iter().map(|(_, value)| value));
                    members
                        .iter()
                        .map(|(name, _)| name.len() as u64)
                        .fold(0u64, |sum, n| sum.saturating_add(n))
                        .saturating_add(members.len() as u64 * Self::OBJECT_MEMBER_WEIGHT)
                }
            });
        }
        total
    }

    pub fn heap_bytes(&self) -> u64 {
        let mut total = 0u64;
        let mut stack = vec![self];
        while let Some(value) = stack.pop() {
            total = total.saturating_add(match value {
                Self::Null | Self::Bool(_) => 0,
                Self::Number(token) | Self::String(token) => token.capacity() as u64,
                Self::Array(items) => {
                    stack.extend(items.iter());
                    ((items.capacity() * std::mem::size_of::<CanonValue>()) as u64)
                        .saturating_add(Self::shared_container_bytes::<Vec<CanonValue>>())
                }
                Self::Object(members) => {
                    let mut names = 0u64;
                    for (name, member) in members.iter() {
                        names = names.saturating_add(name.capacity() as u64);
                        stack.push(member);
                    }
                    names
                        .saturating_add(
                            (members.capacity() * std::mem::size_of::<(String, CanonValue)>())
                                as u64,
                        )
                        .saturating_add(Self::shared_container_bytes::<Vec<(String, CanonValue)>>())
                }
            });
        }
        total
    }

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

    pub fn member(&self, key: &str) -> Option<&CanonValue> {
        match self {
            Self::Object(members) => members
                .binary_search_by(|(candidate, _)| key_cmp_decoded(candidate, key))
                .ok()
                .map(|position| &members[position].1),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(token) => token
                .parse::<i64>()
                .ok()
                .filter(|value| *token == value.to_string()),
            _ => None,
        }
    }

    pub fn as_decoded_str(&self) -> Option<String> {
        match self {
            Self::String(body) => decode_json_string_body(body),
            _ => None,
        }
    }
}

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

pub fn is_valid_object_key(key: &str) -> bool {
    is_json_string_body(key) && decode_json_string_body(key).is_some()
}

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
        _ => left.cmp(right),
    }
}

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
    heap_bytes: u64,
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
                    let body = body.to_owned();
                    self.heap_bytes = self.heap_bytes.saturating_add(body.capacity() as u64);
                    return Ok(body);
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
        let token = token.to_owned();
        self.heap_bytes = self.heap_bytes.saturating_add(token.capacity() as u64);
        Ok(token)
    }

    fn array(&mut self, depth: usize) -> Result<CanonValue, CanonParseError> {
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.position) == Some(&b']') {
            self.position += 1;
            self.heap_bytes = self
                .heap_bytes
                .saturating_add((values.capacity() * std::mem::size_of::<CanonValue>()) as u64)
                .saturating_add(CanonValue::shared_container_bytes::<Vec<CanonValue>>());
            return Ok(CanonValue::array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.bytes.get(self.position) {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    self.heap_bytes = self
                        .heap_bytes
                        .saturating_add(
                            (values.capacity() * std::mem::size_of::<CanonValue>()) as u64,
                        )
                        .saturating_add(CanonValue::shared_container_bytes::<Vec<CanonValue>>());
                    return Ok(CanonValue::array(values));
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
            self.heap_bytes = self
                .heap_bytes
                .saturating_add(
                    (members.capacity() * std::mem::size_of::<(String, CanonValue)>()) as u64,
                )
                .saturating_add(CanonValue::shared_container_bytes::<
                    Vec<(String, CanonValue)>,
                >());
            return Ok(CanonValue::object(members));
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
                    self.heap_bytes = self
                        .heap_bytes
                        .saturating_add(
                            (members.capacity() * std::mem::size_of::<(String, CanonValue)>())
                                as u64,
                        )
                        .saturating_add(CanonValue::shared_container_bytes::<
                            Vec<(String, CanonValue)>,
                        >());
                    return Ok(CanonValue::object(members));
                }
                _ => return self.error("canonical world JSON object is malformed"),
            }
        }
    }
}

#[cfg(test)]
mod heap_bytes_tests {
    use super::*;

    /// arithmetic about ownership and not a restatement of `Vec`'s growth
    #[test]
    fn heap_bytes_counts_every_owned_allocation_exactly_once() {
        let mut items = Vec::with_capacity(2);
        items.push(CanonValue::Number("1".to_string()));
        items.push(CanonValue::Number("2".to_string()));
        let mut members = Vec::with_capacity(2);
        members.push(("a".to_string(), CanonValue::array(items)));
        members.push(("bb".to_string(), CanonValue::String("xyz".to_string())));
        let value = CanonValue::object(members);

        let member = std::mem::size_of::<(String, CanonValue)>() as u64;
        let element = std::mem::size_of::<CanonValue>() as u64;
        let expected = CanonValue::shared_container_bytes::<Vec<(String, CanonValue)>>()
            + 2 * member
            + 1
            + 2
            + CanonValue::shared_container_bytes::<Vec<CanonValue>>()
            + 2 * element
            + 1
            + 1
            + 3;
        assert_eq!(value.heap_bytes(), expected);
    }

    #[test]
    fn heap_bytes_reports_reserved_capacity_not_occupied_length() {
        let mut items: Vec<CanonValue> = Vec::with_capacity(64);
        items.push(CanonValue::Null);
        let value = CanonValue::array(items);
        assert_eq!(
            value.heap_bytes(),
            64 * std::mem::size_of::<CanonValue>() as u64
                + CanonValue::shared_container_bytes::<Vec<CanonValue>>()
        );
    }

    #[test]
    fn parse_counted_matches_the_independent_walk() {
        let documents: &[&[u8]] = &[
            br#"{}"#,
            br#"[]"#,
            br#"null"#,
            br#"{"a":[1,2],"bb":"xyz"}"#,
            br#"[[[[[]]]]]"#,
            br#"{"a":{"b":{"c":[true,false,null,-0.0,1e3]}},"z":""}"#,
            br#"[{"k":"A\\"},{"k":"plain"}]"#,
            br#"{"date":"1936.1.1.1","game_id":"g","hour_index":0,"schema_version":"1","world":{"n":189352.52932739258}}"#,
        ];
        for document in documents {
            let (value, counted) = CanonValue::parse_counted(document).unwrap();
            assert_eq!(
                counted,
                value.heap_bytes(),
                "counted parse disagrees with the walk on {}",
                String::from_utf8_lossy(document)
            );
        }
    }

    /// per node regardless of ownership would inflate every world document by
    #[test]
    fn heap_bytes_charges_ownership_and_nothing_else() {
        assert_eq!(CanonValue::Null.heap_bytes(), 0);
        assert_eq!(CanonValue::Bool(true).heap_bytes(), 0);
        assert_eq!(
            CanonValue::array(Vec::new()).heap_bytes(),
            CanonValue::shared_container_bytes::<Vec<CanonValue>>(),
            "an empty array still owns the allocation that makes it shareable"
        );
        assert_eq!(
            CanonValue::object(Vec::new()).heap_bytes(),
            CanonValue::shared_container_bytes::<Vec<(String, CanonValue)>>(),
            "an empty object still owns the allocation that makes it shareable"
        );
        assert_eq!(
            CanonValue::array(vec![CanonValue::array(Vec::new())]).heap_bytes(),
            CanonValue::shared_container_bytes::<Vec<CanonValue>>() * 2
                + std::mem::size_of::<CanonValue>() as u64,
            "a nested container is charged its own allocation"
        );
    }

    #[test]
    fn heap_bytes_survives_a_value_too_deep_to_recurse() {
        const DEPTH: usize = 200_000;
        let mut value = CanonValue::String("leaf".to_string());
        for _ in 0..DEPTH {
            value = CanonValue::array(vec![value]);
        }
        let element = std::mem::size_of::<CanonValue>() as u64;
        let container = CanonValue::shared_container_bytes::<Vec<CanonValue>>();
        assert_eq!(value.heap_bytes(), DEPTH as u64 * (element + container) + 4);

        let mut unwind = value;
        loop {
            let next = match &mut unwind {
                CanonValue::Array(items) if !items.is_empty() => Arc::make_mut(items).pop(),
                _ => None,
            };
            match next {
                Some(inner) => unwind = inner,
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const NON_IDENTITY_TOKENS: &[&str] = &[
        "189352.52932739258",
        "11268.982520000001",
        "26914.271699999998",
        "10.069999694824219",
        "10.100000381469727",
        "9.930000305175781",
        "1.50",
        "1e3",
        "1E3",
        "1e+3",
        "0.0000000000000000000000001",
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

    #[test]
    fn escaped_member_names_are_accepted_ordered_by_their_decoded_text() {
        let source = r#"{"a\"b":1,"aXb":2}"#;
        let parsed = CanonValue::parse(source.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(parsed.to_json_bytes()).unwrap(), source);
        assert_eq!(parsed.member("a\"b").and_then(CanonValue::as_i64), Some(1));
        assert_eq!(parsed.member("aXb").and_then(CanonValue::as_i64), Some(2));

        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), source);

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
