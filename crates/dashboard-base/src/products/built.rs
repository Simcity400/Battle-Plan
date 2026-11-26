
use super::{MAX_PRODUCT_REPLY_BYTES, over_budget};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{self, Write},
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

pub enum Data {
    Tree(Value),
    Built(Built),
}

impl From<Value> for Data {
    fn from(value: Value) -> Self {
        Self::Tree(value)
    }
}

impl From<Built> for Data {
    fn from(built: Built) -> Self {
        Self::Built(built)
    }
}

impl Data {
    pub fn is_null(&self) -> bool {
        match self {
            Self::Tree(value) => value.is_null(),
            Self::Built(built) => built.text.trim_start().starts_with('n'),
        }
    }

    pub fn field(&self, name: &str) -> io::Result<Option<Value>> {
        match self {
            Self::Tree(value) => Ok(value.get(name).cloned()),
            Self::Built(built) => built.field(name),
        }
    }
}

#[derive(Debug)]
pub struct Built {
    text: String,
    set: Vec<(String, String)>,
}

struct Budgeted<'a> {
    text: Vec<u8>,
    limit: usize,
    cancelled: &'a AtomicBool,
    over: bool,
}

impl Write for Budgeted<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelled"));
        }
        if self.text.len() + bytes.len() > self.limit {
            self.over = true;
            return Err(io::Error::other("over budget"));
        }
        self.text
            .try_reserve(bytes.len())
            .map_err(|_| io::Error::other("Not enough memory for the reply"))?;
        self.text.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Built {
    pub fn of(reply: &impl Serialize, cancelled: &AtomicBool) -> io::Result<Self> {
        Self::within(reply, cancelled, MAX_PRODUCT_REPLY_BYTES, || {
            over_budget(format!(
                "This product reply exceeds the transport reply budget of {MAX_PRODUCT_REPLY_BYTES} bytes"
            ))
        })
    }

    pub fn within(
        reply: &impl Serialize,
        cancelled: &AtomicBool,
        limit: usize,
        refusal: impl FnOnce() -> io::Error,
    ) -> io::Result<Self> {
        let mut writer = Budgeted {
            text: Vec::new(),
            limit,
            cancelled,
            over: false,
        };
        if let Err(error) = serde_json::to_writer(&mut writer, reply) {
            crate::metadata::check_cancel(cancelled)?;
            if writer.over {
                return Err(refusal());
            }
            return Err(io::Error::other(error));
        }
        Ok(Self {
            text: String::from_utf8(writer.text).map_err(io::Error::other)?,
            set: Vec::new(),
        })
    }

    fn object(&self) -> bool {
        self.text.trim_start().starts_with('{')
    }

    pub fn field(&self, name: &str) -> io::Result<Option<Value>> {
        if let Some((_, value)) = self.set.iter().find(|(field, _)| field == name) {
            return Ok(Some(serde_json::from_str(value)?));
        }
        if !self.object() {
            return Ok(None);
        }
        let (fields, _) = root_fields(&self.text)?;
        fields
            .into_iter()
            .find(|(field, _)| field == name)
            .map(|(_, range)| serde_json::from_str(&self.text[range]).map_err(io::Error::from))
            .transpose()
    }

    pub fn into_value(self) -> io::Result<Value> {
        Ok(serde_json::from_str(&self.finish()?)?)
    }

    pub fn finish(self) -> io::Result<String> {
        if self.set.is_empty() {
            return Ok(self.text);
        }
        let (fields, close) = root_fields(&self.text)?;
        let mut replaced: Vec<(Range<usize>, &str)> = Vec::new();
        let mut appended = String::new();
        for (name, value) in &self.set {
            match fields.iter().find(|(field, _)| field == name) {
                Some((_, range)) => replaced.push((range.clone(), value)),
                None => {
                    if !fields.is_empty() || !appended.is_empty() {
                        appended.push(',');
                    }
                    appended.push_str(&serde_json::to_string(name)?);
                    appended.push(':');
                    appended.push_str(value);
                }
            }
        }
        replaced.sort_by_key(|(range, _)| range.start);
        let mut text = String::with_capacity(self.text.len() + appended.len() + 64);
        let mut at = 0;
        for (range, value) in replaced {
            text.push_str(&self.text[at..range.start]);
            text.push_str(value);
            at = range.end;
        }
        text.push_str(&self.text[at..close]);
        text.push_str(&appended);
        text.push_str(&self.text[close..]);
        Ok(text)
    }
}

pub trait Fields {
    fn set_field(&mut self, name: &str, value: &impl Serialize) -> io::Result<()>;
}

impl Fields for Value {
    fn set_field(&mut self, name: &str, value: &impl Serialize) -> io::Result<()> {
        if let Some(reply) = self.as_object_mut() {
            reply.insert(name.into(), serde_json::to_value(value)?);
        }
        Ok(())
    }
}

impl Fields for Built {
    fn set_field(&mut self, name: &str, value: &impl Serialize) -> io::Result<()> {
        if !self.object() {
            return Ok(());
        }
        let value = serde_json::to_string(value)?;
        match self.set.iter_mut().find(|(field, _)| field == name) {
            Some((_, held)) => *held = value,
            None => self.set.push((name.to_owned(), value)),
        }
        Ok(())
    }
}

impl Fields for Data {
    fn set_field(&mut self, name: &str, value: &impl Serialize) -> io::Result<()> {
        match self {
            Self::Tree(tree) => tree.set_field(name, value),
            Self::Built(built) => built.set_field(name, value),
        }
    }
}

fn malformed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "A built reply is not JSON")
}

struct Scan<'t> {
    text: &'t [u8],
    at: usize,
}

impl<'t> Scan<'t> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> io::Result<u8> {
        self.space();
        self.text.get(self.at).copied().ok_or_else(malformed)
    }

    fn expect(&mut self, byte: u8) -> io::Result<()> {
        if self.peek()? != byte {
            return Err(malformed());
        }
        self.at += 1;
        Ok(())
    }

    fn string(&mut self) -> io::Result<Range<usize>> {
        let start = self.at;
        self.expect(b'"')?;
        loop {
            match self.text.get(self.at).ok_or_else(malformed)? {
                b'"' => {
                    self.at += 1;
                    return Ok(start..self.at);
                }
                b'\\' => self.at += 2,
                _ => self.at += 1,
            }
        }
    }

    fn key(&mut self, text: &str) -> io::Result<String> {
        let range = self.string()?;
        let raw = &text[range.start + 1..range.end - 1];
        if raw.contains('\\') {
            Ok(serde_json::from_str(&text[range])?)
        } else {
            Ok(raw.to_owned())
        }
    }

    fn skip(&mut self) -> io::Result<Range<usize>> {
        let start = {
            self.space();
            self.at
        };
        match self.peek()? {
            b'"' => {
                self.string()?;
            }
            b'{' | b'[' => {
                let mut depth = 0_usize;
                loop {
                    match self.text.get(self.at).ok_or_else(malformed)? {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                self.at += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    self.at += 1;
                }
            }
            _ => {
                while self
                    .text
                    .get(self.at)
                    .is_some_and(|byte| !matches!(byte, b',' | b'}' | b']') && !byte.is_ascii_whitespace())
                {
                    self.at += 1;
                }
            }
        }
        Ok(start..self.at)
    }
}

fn root_fields(text: &str) -> io::Result<(Vec<(String, Range<usize>)>, usize)> {
    let mut scan = Scan {
        text: text.as_bytes(),
        at: 0,
    };
    scan.expect(b'{')?;
    let mut fields = Vec::new();
    if scan.peek()? != b'}' {
        loop {
            let name = scan.key(text)?;
            scan.expect(b':')?;
            fields.push((name, scan.skip()?));
            match scan.peek()? {
                b',' => scan.at += 1,
                b'}' => break,
                _ => return Err(malformed()),
            }
        }
    }
    scan.space();
    Ok((fields, scan.at))
}

pub fn reach(text: &str, paths: &[&[&str]]) -> io::Result<Vec<(usize, Range<usize>, bool)>> {
    let mut scan = Scan {
        text: text.as_bytes(),
        at: 0,
    };
    let mut reached = Vec::new();
    let active: Vec<(usize, &[&str])> = paths.iter().copied().enumerate().collect();
    value(&mut scan, text, &active, &mut reached)?;
    Ok(reached)
}

fn value<'p>(
    scan: &mut Scan<'_>,
    text: &str,
    active: &[(usize, &'p [&'p str])],
    reached: &mut Vec<(usize, Range<usize>, bool)>,
) -> io::Result<()> {
    scan.space();
    let start = scan.at;
    let follows = |keyed: bool| {
        active
            .iter()
            .any(|(_, rest)| rest.first().is_some_and(|first| (*first != "*") == keyed))
    };
    let first = scan.peek()?;
    match first {
        b'{' if follows(true) => {
            scan.at += 1;
            if scan.peek()? != b'}' {
                loop {
                    let key = scan.key(text)?;
                    scan.expect(b':')?;
                    let next: Vec<_> = active
                        .iter()
                        .filter_map(|&(path, rest)| match rest.split_first() {
                            Some((first, rest)) if *first != "*" && *first == key => {
                                Some((path, rest))
                            }
                            _ => None,
                        })
                        .collect();
                    if next.is_empty() {
                        scan.skip()?;
                    } else {
                        value(scan, text, &next, reached)?;
                    }
                    match scan.peek()? {
                        b',' => scan.at += 1,
                        b'}' => break,
                        _ => return Err(malformed()),
                    }
                }
            }
            scan.expect(b'}')?;
        }
        b'[' if follows(false) => {
            let next: Vec<_> = active
                .iter()
                .filter_map(|&(path, rest)| match rest.split_first() {
                    Some((&"*", rest)) => Some((path, rest)),
                    _ => None,
                })
                .collect();
            scan.at += 1;
            if scan.peek()? != b']' {
                loop {
                    value(scan, text, &next, reached)?;
                    match scan.peek()? {
                        b',' => scan.at += 1,
                        b']' => break,
                        _ => return Err(malformed()),
                    }
                }
            }
            scan.expect(b']')?;
        }
        _ => {
            scan.skip()?;
        }
    }
    for &(path, rest) in active {
        if rest.is_empty() {
            reached.push((path, start..scan.at, first == b'['));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_fields_replace_written_ones_where_they_stand_and_follow_the_rest() {
        let flag = AtomicBool::new(false);
        let mut built = Built::of(
            &json!({"a": {"coverage": 1}, "coverage": [1, "}"], "z": "\"{"}),
            &flag,
        )
        .unwrap();
        built.set_field("coverage", &json!({"complete": true})).unwrap();
        built.set_field("pending", &json!([])).unwrap();
        assert_eq!(built.field("coverage").unwrap(), Some(json!({"complete": true})));
        assert_eq!(built.field("z").unwrap(), Some(json!("\"{")));
        let mut empty = Built::of(&json!({}), &flag).unwrap();
        empty.set_field("pending", &1).unwrap();
        assert_eq!(empty.into_value().unwrap(), json!({"pending": 1}));
        assert_eq!(
            built.into_value().unwrap(),
            json!({"a": {"coverage": 1}, "coverage": {"complete": true}, "z": "\"{", "pending": []})
        );
        let mut null = Built::of(&Value::Null, &flag).unwrap();
        null.set_field("coverage", &1).unwrap();
        assert_eq!(Data::from(null).is_null(), true);
    }

    #[test]
    fn reach_finds_what_a_tree_walk_finds() {
        let text = r#"{"*":{"warnings":["x"]},"items":[{"warnings":[]},{"warnings":null}],"warnings":["a\"]"]}"#;
        let found = reach(text, &[&["warnings"], &["items", "*", "warnings"]]).unwrap();
        let spans: Vec<_> = found
            .iter()
            .map(|(path, range, list)| (*path, &text[range.clone()], *list))
            .collect();
        assert_eq!(
            spans,
            vec![
                (1, "[]", true),
                (1, "null", false),
                (0, r#"["a\"]"]"#, true)
            ]
        );
    }

    #[test]
    fn a_built_reply_past_the_product_budget_is_refused_as_over_budget() {
        let long = "x".repeat(MAX_PRODUCT_REPLY_BYTES);
        let error = Built::of(&long, &AtomicBool::new(false)).err().unwrap();
        assert!(super::super::is_over_budget(&error));
        let error = Built::of(&long, &AtomicBool::new(true)).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        let own = Built::within(&long, &AtomicBool::new(false), 10, || {
            io::Error::new(io::ErrorKind::InvalidInput, "the product's own words")
        })
        .err()
        .unwrap();
        assert_eq!(own.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(own.to_string(), "the product's own words");
    }
}
