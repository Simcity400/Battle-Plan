use crate::{GameUiError, Result};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub root: Value,
    pub recoveries: Vec<Recovery>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recovery {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Scalar(Scalar),
    Object(Vec<Entry>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Bare(String),
    Quoted(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: Option<String>,
    pub value: Value,
    pub line: usize,
    pub column: usize,
}

impl Value {
    pub fn as_object(&self) -> Option<&[Entry]> {
        match self {
            Value::Object(entries) => Some(entries),
            Value::Scalar(_) => None,
        }
    }

    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            Value::Scalar(Scalar::Bare(value) | Scalar::Quoted(value)) => Some(value),
            Value::Object(_) => None,
        }
    }

    pub fn first_value_ci(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|entry| entry.key_matches(key))
            .map(|entry| &entry.value)
    }

    pub fn first_scalar_ci(&self, key: &str) -> Option<&str> {
        self.first_value_ci(key)?.as_scalar()
    }

    pub fn values_ci<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.as_object()
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter(move |entry| entry.key_matches(key))
            .map(|entry| &entry.value)
    }

    pub fn fields(&self) -> &[Entry] {
        match self {
            Value::Object(entries) => entries,
            Value::Scalar(_) => &[],
        }
    }
}

impl Entry {
    pub fn key_matches(&self, key: &str) -> bool {
        self.key
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(key))
    }
}

pub fn parse_file(path: &Path) -> Result<Document> {
    let bytes = fs::read(path)?;
    Ok(parse_bytes(&bytes))
}

pub fn parse_bytes(bytes: &[u8]) -> Document {
    let text = decode_text(bytes);
    parse_str(&text)
}

pub fn parse_str(text: &str) -> Document {
    Parser::new(text).parse_document()
}

pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let words = rest
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&words);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let words = rest
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&words);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().copied().map(char::from).collect(),
    }
}

pub fn scalar_i32(value: &Value) -> Option<i32> {
    value.as_scalar()?.trim_end_matches('%').parse().ok()
}

pub fn scalar_u32(value: &Value) -> Option<u32> {
    value.as_scalar()?.trim_end_matches('%').parse().ok()
}

pub fn scalar_f32(value: &Value) -> Option<f32> {
    value.as_scalar()?.trim_end_matches('%').parse().ok()
}

pub fn object_number_pair(
    value: &Value,
    x_keys: &[&str],
    y_keys: &[&str],
) -> Option<(String, String)> {
    let x = first_scalar_any(value, x_keys)?;
    let y = first_scalar_any(value, y_keys)?;
    Some((x.to_string(), y.to_string()))
}

pub fn first_scalar_any<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.first_scalar_ci(key))
}

pub fn block_items(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .as_object()
        .into_iter()
        .flat_map(|entries| entries.iter())
        .filter(|entry| entry.key.is_none())
        .map(|entry| &entry.value)
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Atom(String),
    Quoted(String),
    Equals,
    Open,
    Close,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
struct Token {
    kind: TokenKind,
    line: usize,
    column: usize,
}

struct Parser<'a> {
    chars: std::str::Chars<'a>,
    lookahead_char: Option<char>,
    line: usize,
    column: usize,
    pushed: Option<Token>,
    recoveries: Vec<Recovery>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            chars: text.chars(),
            lookahead_char: None,
            line: 1,
            column: 1,
            pushed: None,
            recoveries: Vec::new(),
        }
    }

    fn parse_document(mut self) -> Document {
        let root = Value::Object(self.parse_entries(false));
        Document {
            root,
            recoveries: self.recoveries,
        }
    }

    fn parse_entries(&mut self, stop_on_close: bool) -> Vec<Entry> {
        let mut entries = Vec::new();
        loop {
            let token = self.next_token();
            match token.kind {
                TokenKind::Eof => {
                    if stop_on_close {
                        self.recover_at(&token, "missing closing brace; accepted end of file");
                    }
                    break;
                }
                TokenKind::Close => {
                    if stop_on_close {
                        break;
                    }
                    self.recover_at(&token, "stray closing brace ignored");
                }
                TokenKind::Equals => {
                    self.recover_at(&token, "stray equals sign ignored");
                }
                TokenKind::Open => {
                    let value = Value::Object(self.parse_entries(true));
                    entries.push(Entry {
                        key: None,
                        value,
                        line: token.line,
                        column: token.column,
                    });
                }
                TokenKind::Atom(text) | TokenKind::Quoted(text) => {
                    let next = self.next_token();
                    if next.kind == TokenKind::Equals {
                        let value = self.parse_value();
                        entries.push(Entry {
                            key: Some(text),
                            value,
                            line: token.line,
                            column: token.column,
                        });
                    } else {
                        self.push_token(next);
                        entries.push(Entry {
                            key: None,
                            value: Value::Scalar(Scalar::Bare(text)),
                            line: token.line,
                            column: token.column,
                        });
                    }
                }
            }
        }
        entries
    }

    fn parse_value(&mut self) -> Value {
        let token = self.next_token();
        match token.kind {
            TokenKind::Open => Value::Object(self.parse_entries(true)),
            TokenKind::Atom(value) => Value::Scalar(Scalar::Bare(value)),
            TokenKind::Quoted(value) => Value::Scalar(Scalar::Quoted(value)),
            TokenKind::Close => {
                self.recover_at(&token, "missing value before closing brace");
                self.push_token(token);
                Value::Object(Vec::new())
            }
            TokenKind::Equals => {
                self.recover_at(&token, "equals sign cannot start a value");
                Value::Object(Vec::new())
            }
            TokenKind::Eof => {
                self.recover_at(&token, "missing value at end of file");
                Value::Object(Vec::new())
            }
        }
    }

    fn next_token(&mut self) -> Token {
        if let Some(token) = self.pushed.take() {
            return token;
        }
        self.skip_ws_and_comments();
        let line = self.line;
        let column = self.column;
        let Some(ch) = self.bump() else {
            return Token {
                kind: TokenKind::Eof,
                line,
                column,
            };
        };
        let kind = match ch {
            '=' => TokenKind::Equals,
            '{' => TokenKind::Open,
            '}' => TokenKind::Close,
            '"' => TokenKind::Quoted(self.read_quoted(line, column)),
            _ => TokenKind::Atom(self.read_atom(ch)),
        };
        Token { kind, line, column }
    }

    fn push_token(&mut self, token: Token) {
        debug_assert!(self.pushed.is_none());
        self.pushed = Some(token);
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.peek() != Some('#') {
                break;
            }
            while let Some(ch) = self.bump() {
                if ch == '\n' {
                    break;
                }
            }
        }
    }

    fn read_atom(&mut self, first: char) -> String {
        let mut out = String::new();
        out.push(first);
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || matches!(ch, '{' | '}' | '=' | '#') {
                break;
            }
            out.push(self.bump().expect("peeked char must bump"));
        }
        out
    }

    fn read_quoted(&mut self, line: usize, column: usize) -> String {
        let mut out = String::new();
        let mut escaped = false;
        while let Some(ch) = self.bump() {
            if escaped {
                out.push(match ch {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => return out,
                other => out.push(other),
            }
        }
        self.recoveries.push(Recovery {
            line,
            column,
            message: "unterminated quoted string; accepted end of file".to_string(),
        });
        out
    }

    fn peek(&mut self) -> Option<char> {
        if self.lookahead_char.is_none() {
            self.lookahead_char = self.chars.next();
        }
        self.lookahead_char
    }

    fn bump(&mut self) -> Option<char> {
        let ch = match self.lookahead_char.take() {
            Some(ch) => Some(ch),
            None => self.chars.next(),
        }?;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn recover_at(&mut self, token: &Token, message: impl Into<String>) {
        self.recoveries.push(Recovery {
            line: token.line,
            column: token.column,
            message: message.into(),
        });
    }
}

pub fn required_object<'a>(value: &'a Value, label: &str) -> Result<&'a [Entry]> {
    value
        .as_object()
        .ok_or_else(|| GameUiError::parse(format!("{label} is not a block")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_repeated_keys_and_unassigned_block_values() {
        let doc = parse_str(
            r#"
            spriteTypes = {
              spriteType = { name = "A" color = { 1.0 0.5 0.0 } }
              spriteType = { name = B }
            }
            "#,
        );
        assert!(doc.recoveries.is_empty());
        let sprite_types = doc.root.first_value_ci("spriteTypes").unwrap();
        assert_eq!(sprite_types.values_ci("spriteType").count(), 2);
        let color = sprite_types
            .values_ci("spriteType")
            .next()
            .unwrap()
            .first_value_ci("color")
            .unwrap();
        assert_eq!(block_items(color).count(), 3);
    }

    #[test]
    fn comments_do_not_break_quoted_hashes() {
        let doc = parse_str(r#"name = "not # comment" # comment"#);
        assert_eq!(doc.root.first_scalar_ci("name"), Some("not # comment"));
    }
}
