use crate::content_parse::{ContentParseError as GameUiError, Result};
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

/// The recovery message stamped when end-of-file implicitly closes an open
/// block. The Paradox engine accepts exactly this closure on shipped files, so
/// it reports tolerated input rather than damage.
pub const ENGINE_TOLERATED_EOF_CLOSE_RECOVERY: &str =
    "missing closing brace; accepted end of file";

/// True when the document's only recovery is an end-of-file block closure the
/// engine itself tolerates: one open brace closed by EOF on a file whose last
/// non-whitespace byte is the closing brace of its final block. The parsed
/// tree loses nothing, so consumers may treat the file as parsed instead of
/// damaged.
pub fn engine_tolerated_outer_eof(document: &Document, bytes: &[u8]) -> bool {
    document.recoveries.len() == 1
        && document.recoveries[0].message == ENGINE_TOLERATED_EOF_CLOSE_RECOVERY
        && bytes
            .iter()
            .rfind(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| *byte == b'}')
}

pub const MAX_TEXT_DECODE_DIAGNOSTICS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextDecodeLoss {
    InvalidUtf8 {
        byte_offset: usize,
        bytes: Vec<u8>,
        line: usize,
        column: usize,
    },
    UnpairedUtf16Surrogate {
        byte_offset: usize,
        code_unit: u16,
        line: usize,
        column: usize,
    },
    TruncatedUtf16Unit {
        byte_offset: usize,
        byte: u8,
        line: usize,
        column: usize,
    },
    NonUtf8DecodedAsLatin1 {
        byte_offset: usize,
        line: usize,
        column: usize,
    },
    Suppressed {
        count: usize,
    },
}

impl TextDecodeLoss {
    #[must_use]
    pub fn discards_input(&self) -> bool {
        match self {
            TextDecodeLoss::InvalidUtf8 { .. }
            | TextDecodeLoss::UnpairedUtf16Surrogate { .. }
            | TextDecodeLoss::TruncatedUtf16Unit { .. }
            | TextDecodeLoss::Suppressed { .. } => true,
            TextDecodeLoss::NonUtf8DecodedAsLatin1 { .. } => false,
        }
    }
}

#[must_use]
pub fn destructive_losses(losses: &[TextDecodeLoss]) -> Vec<&TextDecodeLoss> {
    losses.iter().filter(|loss| loss.discards_input()).collect()
}

#[must_use]
pub fn destructive_event_count(losses: &[TextDecodeLoss]) -> usize {
    losses
        .iter()
        .map(|loss| match loss {
            TextDecodeLoss::Suppressed { count } => *count,
            TextDecodeLoss::InvalidUtf8 { .. }
            | TextDecodeLoss::UnpairedUtf16Surrogate { .. }
            | TextDecodeLoss::TruncatedUtf16Unit { .. } => 1,
            TextDecodeLoss::NonUtf8DecodedAsLatin1 { .. } => 0,
        })
        .sum()
}

impl std::fmt::Display for TextDecodeLoss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextDecodeLoss::InvalidUtf8 {
                byte_offset,
                bytes,
                line,
                column,
            } => write!(
                f,
                "{line}:{column}: invalid UTF-8 bytes {bytes:02x?} at byte offset {byte_offset} replaced with U+FFFD"
            ),
            TextDecodeLoss::UnpairedUtf16Surrogate {
                byte_offset,
                code_unit,
                line,
                column,
            } => write!(
                f,
                "{line}:{column}: unpaired UTF-16 surrogate {code_unit:#06x} at byte offset {byte_offset} replaced with U+FFFD"
            ),
            TextDecodeLoss::TruncatedUtf16Unit {
                byte_offset,
                byte,
                line,
                column,
            } => write!(
                f,
                "{line}:{column}: truncated UTF-16 code unit (byte {byte:#04x}) at byte offset {byte_offset} dropped"
            ),
            TextDecodeLoss::NonUtf8DecodedAsLatin1 {
                byte_offset,
                line,
                column,
            } => write!(
                f,
                "{line}:{column}: input is not UTF-8 from byte offset {byte_offset}; decoded as Latin-1"
            ),
            TextDecodeLoss::Suppressed { count } => {
                write!(f, "{count} further decode losses not individually recorded")
            }
        }
    }
}

#[must_use = "byte-level decode damage must be recorded, not dropped"]
pub fn decode_text_diagnosed(bytes: &[u8]) -> (String, Vec<TextDecodeLoss>) {
    (decode_text(bytes), diagnose_text_decode(bytes))
}

pub fn diagnose_text_decode(bytes: &[u8]) -> Vec<TextDecodeLoss> {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return diagnose_utf8_lossy(rest, 3);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return diagnose_utf16_lossy(rest, |chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return diagnose_utf16_lossy(rest, |chunk| u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => Vec::new(),
        Err(error) => {
            let byte_offset = error.valid_up_to();
            let line = 1 + bytes[..byte_offset]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            let column = 1 + bytes[..byte_offset]
                .iter()
                .rev()
                .take_while(|byte| **byte != b'\n')
                .count();
            vec![TextDecodeLoss::NonUtf8DecodedAsLatin1 {
                byte_offset,
                line,
                column,
            }]
        }
    }
}

struct DecodeDiagnosisCursor {
    line: usize,
    column: usize,
    losses: Vec<TextDecodeLoss>,
    suppressed: usize,
}

impl DecodeDiagnosisCursor {
    fn new() -> Self {
        Self {
            line: 1,
            column: 1,
            losses: Vec::new(),
            suppressed: 0,
        }
    }

    fn advance(&mut self, ch: char) {
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    fn record(&mut self, loss: TextDecodeLoss) {
        if self.losses.len() < MAX_TEXT_DECODE_DIAGNOSTICS {
            self.losses.push(loss);
        } else {
            self.suppressed += 1;
        }
    }

    fn finish(mut self) -> Vec<TextDecodeLoss> {
        if self.suppressed > 0 {
            self.losses.push(TextDecodeLoss::Suppressed {
                count: self.suppressed,
            });
        }
        self.losses
    }
}

fn diagnose_utf8_lossy(rest: &[u8], bom_len: usize) -> Vec<TextDecodeLoss> {
    let mut cursor = DecodeDiagnosisCursor::new();
    let mut consumed = 0usize;
    for chunk in rest.utf8_chunks() {
        for ch in chunk.valid().chars() {
            cursor.advance(ch);
        }
        consumed += chunk.valid().len();
        if !chunk.invalid().is_empty() {
            cursor.record(TextDecodeLoss::InvalidUtf8 {
                byte_offset: bom_len + consumed,
                bytes: chunk.invalid().to_vec(),
                line: cursor.line,
                column: cursor.column,
            });
            consumed += chunk.invalid().len();
            cursor.advance(char::REPLACEMENT_CHARACTER);
        }
    }
    cursor.finish()
}

fn diagnose_utf16_lossy(rest: &[u8], word: impl Fn(&[u8]) -> u16) -> Vec<TextDecodeLoss> {
    const BOM_LEN: usize = 2;
    let words = rest.chunks_exact(2).map(word);
    let mut cursor = DecodeDiagnosisCursor::new();
    let mut unit_index = 0usize;
    for decoded in char::decode_utf16(words) {
        match decoded {
            Ok(ch) => {
                cursor.advance(ch);
                unit_index += ch.len_utf16();
            }
            Err(error) => {
                cursor.record(TextDecodeLoss::UnpairedUtf16Surrogate {
                    byte_offset: BOM_LEN + unit_index * 2,
                    code_unit: error.unpaired_surrogate(),
                    line: cursor.line,
                    column: cursor.column,
                });
                cursor.advance(char::REPLACEMENT_CHARACTER);
                unit_index += 1;
            }
        }
    }
    if rest.len() % 2 == 1 {
        cursor.record(TextDecodeLoss::TruncatedUtf16Unit {
            byte_offset: BOM_LEN + rest.len() - 1,
            byte: rest[rest.len() - 1],
            line: cursor.line,
            column: cursor.column,
        });
    }
    cursor.finish()
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

const MAX_DEPTH: usize = 512;

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
        let root = Value::Object(self.parse_entries(false, 0));
        Document {
            root,
            recoveries: self.recoveries,
        }
    }

    fn parse_entries(&mut self, stop_on_close: bool, depth: usize) -> Vec<Entry> {
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
                    let value = self.parse_object_body(&token, depth);
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
                        let value = self.parse_value(depth);
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

    fn parse_object_body(&mut self, open: &Token, depth: usize) -> Value {
        if depth >= MAX_DEPTH {
            self.recover_at(open, "nesting depth limit exceeded; subtree skipped");
            self.skip_balanced_subtree();
            return Value::Object(Vec::new());
        }
        Value::Object(self.parse_entries(true, depth + 1))
    }

    fn skip_balanced_subtree(&mut self) {
        let mut balance = 1usize;
        loop {
            let token = self.next_token();
            match token.kind {
                TokenKind::Open => balance += 1,
                TokenKind::Close => {
                    balance -= 1;
                    if balance == 0 {
                        break;
                    }
                }
                TokenKind::Eof => {
                    self.recover_at(&token, "missing closing brace; accepted end of file");
                    break;
                }
                TokenKind::Atom(_) | TokenKind::Quoted(_) | TokenKind::Equals => {}
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Value {
        let token = self.next_token();
        match token.kind {
            TokenKind::Open => self.parse_object_body(&token, depth),
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

    #[test]
    fn deeply_nested_braces_recover_instead_of_overflowing() {
        let text = "{".repeat(200_000);
        let doc = parse_str(&text);
        assert!(
            doc.recoveries
                .iter()
                .any(|recovery| recovery.message.contains("nesting depth limit exceeded")),
            "expected a depth-limit recovery, got: {:?}",
            doc.recoveries.first()
        );
    }

    #[test]
    fn deeply_nested_value_braces_recover_instead_of_overflowing() {
        let text = format!("key = {}", "{".repeat(200_000));
        let doc = parse_str(&text);
        assert!(
            doc.recoveries
                .iter()
                .any(|recovery| recovery.message.contains("nesting depth limit exceeded")),
            "expected a depth-limit recovery, got: {:?}",
            doc.recoveries.first()
        );
        assert_eq!(doc.root.fields().len(), 1);
        assert_eq!(doc.root.fields()[0].key.as_deref(), Some("key"));
    }

    #[test]
    fn depth_limited_subtree_is_skipped_and_siblings_survive() {
        let mut text = String::new();
        for _ in 0..(MAX_DEPTH + 8) {
            text.push_str("a = { ");
        }
        text.push_str("inner = 1 ");
        for _ in 0..(MAX_DEPTH + 8) {
            text.push('}');
        }
        text.push_str(" after = 2");
        let doc = parse_str(&text);
        assert!(doc
            .recoveries
            .iter()
            .any(|recovery| recovery.message.contains("nesting depth limit exceeded")));
        assert_eq!(doc.root.first_scalar_ci("after"), Some("2"));
    }

    #[test]
    fn diagnosed_decode_is_byte_identical_to_plain_decode() {
        let samples: &[&[u8]] = &[
            b"",
            b"key = value",
            b"\xef\xbb\xbfkey = value",
            b"\xef\xbb\xbfbad \xff\xfe bytes",
            b"\xef\xbb\xbftruncated \xe2\x82",
            b"\xff\xfek\x00e\x00y\x00",
            b"\xff\xfek\x00\x00\xd8y\x00",
            b"\xff\xfek\x00e\x00y",
            b"\xfe\xff\x00k\x00e\x00y",
            b"\xfe\xff\x00k\xd8\x00\x00y",
            b"\xfe\xff\x00k\x00e\x00",
            b"latin1 caf\xe9 = value",
            b"\xff\xfe",
            b"\xfe\xff",
            b"\xef\xbb\xbf",
            b"\xff",
        ];
        for sample in samples {
            let (text, _) = decode_text_diagnosed(sample);
            assert_eq!(text, decode_text(sample), "sample {sample:02x?}");
        }
    }

    #[test]
    fn clean_inputs_produce_no_decode_diagnostics() {
        for sample in [
            &b"key = value\nnext = 1"[..],
            b"\xef\xbb\xbfname = \"caf\xc3\xa9\"",
            b"\xff\xfek\x00=\x00v\x00",
            b"\xfe\xff\x00k\x00=\x00v",
            b"",
        ] {
            assert_eq!(diagnose_text_decode(sample), Vec::new(), "{sample:02x?}");
        }
    }

    #[test]
    fn invalid_utf8_after_bom_is_attributed_with_position_and_lost_bytes() {
        let (text, losses) = decode_text_diagnosed(b"\xef\xbb\xbfa = 1\nk = \xe2\x82ok");
        assert_eq!(text, "a = 1\nk = \u{fffd}ok");
        assert_eq!(
            losses,
            vec![TextDecodeLoss::InvalidUtf8 {
                byte_offset: 13,
                bytes: vec![0xe2, 0x82],
                line: 2,
                column: 5,
            }]
        );
        assert_eq!(
            losses[0].to_string(),
            "2:5: invalid UTF-8 bytes [e2, 82] at byte offset 13 replaced with U+FFFD"
        );
    }

    #[test]
    fn unpaired_utf16_surrogates_are_attributed_in_both_byte_orders() {
        let (text, losses) = decode_text_diagnosed(b"\xff\xfea\x00\n\x00\x00\xd8b\x00");
        assert_eq!(text, "a\n\u{fffd}b");
        assert_eq!(
            losses,
            vec![TextDecodeLoss::UnpairedUtf16Surrogate {
                byte_offset: 6,
                code_unit: 0xd800,
                line: 2,
                column: 1,
            }]
        );

        let (text, losses) = decode_text_diagnosed(b"\xfe\xff\xd8\x3d\xde\x00\xdc\x00\x00b");
        assert_eq!(text, "\u{1f600}\u{fffd}b");
        assert_eq!(
            losses,
            vec![TextDecodeLoss::UnpairedUtf16Surrogate {
                byte_offset: 6,
                code_unit: 0xdc00,
                line: 1,
                column: 2,
            }]
        );
    }

    #[test]
    fn truncated_utf16_tail_byte_is_reported_not_silently_dropped() {
        let (text, losses) = decode_text_diagnosed(b"\xff\xfea\x00b");
        assert_eq!(text, "a");
        assert_eq!(
            losses,
            vec![TextDecodeLoss::TruncatedUtf16Unit {
                byte_offset: 4,
                byte: b'b',
                line: 1,
                column: 2,
            }]
        );
    }

    #[test]
    fn latin1_fallback_is_attributed_at_the_first_non_utf8_byte() {
        let (text, losses) = decode_text_diagnosed(b"a = 1\nk = caf\xe9");
        assert_eq!(text, "a = 1\nk = caf\u{e9}");
        assert_eq!(
            losses,
            vec![TextDecodeLoss::NonUtf8DecodedAsLatin1 {
                byte_offset: 13,
                line: 2,
                column: 8,
            }]
        );
    }

    #[test]
    fn decode_diagnostics_are_bounded_and_the_overflow_stays_counted() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        for _ in 0..MAX_TEXT_DECODE_DIAGNOSTICS + 40 {
            bytes.push(b'x');
            bytes.push(0xff);
        }
        let (text, losses) = decode_text_diagnosed(&bytes);
        assert_eq!(
            text.chars().filter(|ch| *ch == '\u{fffd}').count(),
            MAX_TEXT_DECODE_DIAGNOSTICS + 40
        );
        assert_eq!(losses.len(), MAX_TEXT_DECODE_DIAGNOSTICS + 1);
        assert_eq!(
            losses.last(),
            Some(&TextDecodeLoss::Suppressed { count: 40 })
        );
    }

    #[test]
    fn only_events_that_destroy_input_are_classified_destructive() {
        let destroys = [
            TextDecodeLoss::InvalidUtf8 {
                byte_offset: 0,
                bytes: vec![0xff],
                line: 1,
                column: 1,
            },
            TextDecodeLoss::UnpairedUtf16Surrogate {
                byte_offset: 0,
                code_unit: 0xd800,
                line: 1,
                column: 1,
            },
            TextDecodeLoss::TruncatedUtf16Unit {
                byte_offset: 0,
                byte: b'x',
                line: 1,
                column: 1,
            },
            TextDecodeLoss::Suppressed { count: 1 },
        ];
        for loss in &destroys {
            assert!(loss.discards_input(), "{loss:?}");
        }
        let lossless = TextDecodeLoss::NonUtf8DecodedAsLatin1 {
            byte_offset: 0,
            line: 1,
            column: 1,
        };
        assert!(!lossless.discards_input());

        let mut all = destroys.to_vec();
        all.push(lossless);
        assert_eq!(destructive_losses(&all).len(), 4);
    }

    #[test]
    fn suppressed_overflow_counts_every_event_not_every_entry() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        for _ in 0..MAX_TEXT_DECODE_DIAGNOSTICS + 40 {
            bytes.push(b'x');
            bytes.push(0xff);
        }
        let losses = diagnose_text_decode(&bytes);
        assert_eq!(
            destructive_losses(&losses).len(),
            MAX_TEXT_DECODE_DIAGNOSTICS + 1
        );
        assert_eq!(
            destructive_event_count(&losses),
            MAX_TEXT_DECODE_DIAGNOSTICS + 40
        );
        assert_eq!(
            destructive_event_count(&diagnose_text_decode(b"caf\xe9")),
            0
        );
    }

    #[test]
    fn latin1_fallback_reports_without_destroying_input() {
        let bytes = b"text = \"Email\xa7R *\xa7!\"";
        let losses = diagnose_text_decode(bytes);
        assert_eq!(losses.len(), 1);
        assert!(destructive_losses(&losses).is_empty());
        let (text, _) = decode_text_diagnosed(bytes);
        assert_eq!(
            text.chars().count(),
            bytes.len(),
            "Latin-1 fallback is one char per byte, so nothing is lost"
        );
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn destroyed_bytes_can_leave_the_parse_completely_clean() {
        let bytes = b"\xef\xbb\xbfname = \"a\xffb\"";
        let document = parse_bytes(bytes);
        assert!(document.recoveries.is_empty(), "{:?}", document.recoveries);
        assert_eq!(document.root.first_scalar_ci("name"), Some("a\u{fffd}b"));
        assert_eq!(destructive_losses(&diagnose_text_decode(bytes)).len(), 1);
    }

    #[test]
    fn nesting_below_the_depth_limit_is_untouched() {
        let mut text = String::new();
        for _ in 0..64 {
            text.push_str("a = { ");
        }
        text.push_str("inner = 1 ");
        for _ in 0..64 {
            text.push('}');
        }
        let doc = parse_str(&text);
        assert!(doc.recoveries.is_empty());
        let mut value = &doc.root;
        for _ in 0..64 {
            value = value.first_value_ci("a").unwrap();
        }
        assert_eq!(value.first_scalar_ci("inner"), Some("1"));
    }
}
