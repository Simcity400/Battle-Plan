use crate::content_parse::{ContentParseError as GameUiError, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub root: Value,
    pub recoveries: Vec<Recovery>,
    /// Whether the LAST significant token the reader saw was a structural `}`.
    ///
    /// The reader's own answer, not a scan of the file's trailing bytes. A `}`
    /// inside a comment or a quoted value is text, and a suffix scan that
    /// counted it as a closing brace would call a truncated file tolerated -- the
    /// exact scanner disagreement this module exists to prevent.
    pub ends_on_structural_close: bool,
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

pub fn parse_bytes(bytes: &[u8]) -> Document {
    let text = decode_text(bytes);
    parse_str(&text)
}

// Asset readers terminate at the first root close, and EOF closes open persistent blocks.
pub fn parse_asset_str(text: &str) -> std::result::Result<(Value, std::borrow::Cow<'_, str>), Recovery> {
    let mut parser = Parser::new(text);
    let root = Value::Object(parser.parse_entries(true, 0));
    let mut eof_closures = 0usize;
    for recovery in parser.recoveries {
        if recovery.message != ENGINE_TOLERATED_EOF_CLOSE_RECOVERY { return Err(recovery); }
        eof_closures += 1;
    }
    let consumed = text.len() - parser.chars.as_str().len()
        - parser.lookahead_char.map(char::len_utf8).unwrap_or(0);
    let prefix = &text[..consumed];
    let normalized = if eof_closures > 1 {
        std::borrow::Cow::Owned(format!("{prefix}\n{}", "}".repeat(eof_closures - 1)))
    } else { std::borrow::Cow::Borrowed(prefix) };
    Ok((root, normalized))
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
/// significant TOKEN is the closing brace of its final block. The parsed tree
/// loses nothing, so consumers may treat the file as parsed instead of damaged.
///
/// The last-token test comes from the reader, not from the file's trailing
/// bytes. A file ending `... }` newline `# }` has a `}` as its last non-blank
/// byte and no closing token at all, and a byte scan called it tolerated.
pub fn engine_tolerated_outer_eof(document: &Document) -> bool {
    document.recoveries.len() == 1
        && document.recoveries[0].message == ENGINE_TOLERATED_EOF_CLOSE_RECOVERY
        && document.ends_on_structural_close
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

/// The parsed root, or the FIRST recovery that is not one the engine itself
/// tolerates.
///
/// `parse_str` recovers, and a consumer that reads what it recovered from
/// answers questions from text the reader could not follow. But not every
/// recovery is damage: [`engine_tolerated_outer_eof`] names the one the engine
/// accepts on shipped files -- a single outer block closed by end of file, on a
/// file whose last non-whitespace byte is its final `}` -- and the parsed tree
/// loses nothing there.
///
/// WHICH recoveries are tolerated is decided here, in one place, so no consumer
/// reclassifies the reader's diagnostics for itself.
pub fn parse_str_strict(text: &str) -> std::result::Result<Value, Recovery> {
    let document = parse_str(text);
    match document.recoveries.first() {
        None => Ok(document.root),
        Some(recovery) => {
            let recovery = recovery.clone();
            if engine_tolerated_outer_eof(&document) {
                Ok(document.root)
            } else {
                Err(recovery)
            }
        }
    }
}

/// The decoded text, or the FIRST loss that discarded input.
///
/// `decode_text` recovers, and a consumer that reads what it recovered from
/// answers questions about content the file does not hold: an invalid byte
/// inside a quoted token becomes `U+FFFD`, an unpaired UTF-16 surrogate becomes
/// the same character, and a truncated UTF-16 unit is dropped outright. This is
/// for consumers that will not read damage.
///
/// The Latin-1 fallback is NOT a loss here. It discards nothing -- every byte
/// becomes the character with its value -- and it is how Paradox text files that
/// are not UTF-8 are read.
///
/// The variant test is [`TextDecodeLoss::discards_input`] and it lives in ONE
/// place, so that a consumer never samples the destructive variants itself and
/// never misses one.
pub fn decode_text_strict(bytes: &[u8]) -> std::result::Result<String, TextDecodeLoss> {
    let (text, losses) = decode_text_diagnosed(bytes);
    match losses.into_iter().find(TextDecodeLoss::discards_input) {
        Some(loss) => Err(loss),
        None => Ok(text),
    }
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
    ends_on_close: bool,
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
            ends_on_close: false,
        }
    }

    fn parse_document(mut self) -> Document {
        let root = Value::Object(self.parse_entries(false, 0));
        Document {
            root,
            recoveries: self.recoveries,
            ends_on_structural_close: self.ends_on_close,
        }
    }

    fn parse_entries(&mut self, stop_on_close: bool, depth: usize) -> Vec<Entry> {
        let mut entries = Vec::new();
        loop {
            let token = self.next_token();
            let quoted = matches!(&token.kind, TokenKind::Quoted(_));
            match token.kind {
                TokenKind::Eof => {
                    if stop_on_close {
                        self.recover_at(&token, ENGINE_TOLERATED_EOF_CLOSE_RECOVERY);
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
                            value: Value::Scalar(if quoted { Scalar::Quoted(text) } else { Scalar::Bare(text) }),
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
                    self.recover_at(&token, ENGINE_TOLERATED_EOF_CLOSE_RECOVERY);
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
        // Recorded HERE, where comments and quoted values have already been
        // resolved, so that "the file ends on a closing brace" is a fact about
        // the token stream and not about the last byte.
        self.ends_on_close = matches!(kind, TokenKind::Close);
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

/// A file-scoped `@name` reference that could not be expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariableExpansion {
    /// The file reads `@name` and defines no `@name`.
    Undefined { reference: String, undefined: String },
    /// The definitions lead back to a name already on the chain.
    Cyclic { reference: String },
}

impl std::fmt::Display for VariableExpansion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariableExpansion::Undefined {
                reference,
                undefined,
            } => write!(f, "{reference} reads {undefined}, which the file does not define"),
            VariableExpansion::Cyclic { reference } => {
                write!(f, "{reference} is defined in terms of itself")
            }
        }
    }
}

/// Expand every file-scoped `@name` reference in `root`, IN PLACE.
///
/// Clausewitz files may define `@name = value` at the top level and then read
/// `@name` wherever a value is expected. This expands a reference to the
/// SCALAR its definition holds, kind and all: `@font = "hoi_16mbs"` read as
/// `font = @font` yields a QUOTED scalar, because a consumer that tells a
/// quoted value from a bare one -- as authored map content does -- would
/// otherwise see a different type from the one the file authored.
///
/// Only a BARE scalar is a reference; a quoted one is text. A chain is followed
/// until it reaches a non-reference, and a name that leads back onto its own
/// chain, or one nothing defines, is REFUSED rather than left in place: a
/// consumer that received `@name` as a literal would parse it as content.
///
/// This is deliberately separate from `parse_str`, so no existing consumer's
/// reading changes by adding it.
pub fn expand_file_scoped_variables(root: &mut Value) -> std::result::Result<(), VariableExpansion> {
    let definitions = root
        .fields()
        .iter()
        .filter_map(|entry| {
            let name = entry.key.as_deref()?.strip_prefix('@')?;
            let Value::Scalar(scalar) = &entry.value else {
                return None;
            };
            (!name.trim().is_empty()).then(|| (format!("@{name}"), scalar.clone()))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let Value::Object(entries) = root else {
        return expand_into(root, &definitions);
    };
    for entry in entries.iter_mut() {
        // A definition is the ENVIRONMENT, not content. Expanding it in place
        // would report a cycle or an undefined name against the definition
        // rather than against the content that reads it, and each definition's
        // own chain is followed anyway when a reference reaches it.
        if entry.key.as_deref().is_some_and(|key| key.starts_with('@')) {
            continue;
        }
        expand_into(&mut entry.value, &definitions)?;
    }
    Ok(())
}

fn expand_into(
    value: &mut Value,
    definitions: &std::collections::BTreeMap<String, Scalar>,
) -> std::result::Result<(), VariableExpansion> {
    match value {
        Value::Scalar(Scalar::Bare(token)) if token.starts_with('@') => {
            let reference = token.clone();
            let mut seen = std::collections::BTreeSet::new();
            let mut current = Scalar::Bare(reference.clone());
            while let Scalar::Bare(name) = &current {
                if !name.starts_with('@') {
                    break;
                }
                if !seen.insert(name.clone()) {
                    return Err(VariableExpansion::Cyclic { reference });
                }
                let Some(next) = definitions.get(name) else {
                    return Err(VariableExpansion::Undefined {
                        reference,
                        undefined: name.clone(),
                    });
                };
                current = next.clone();
            }
            *value = Value::Scalar(current);
            Ok(())
        }
        Value::Scalar(_) => Ok(()),
        Value::Object(entries) => {
            for entry in entries.iter_mut() {
                expand_into(&mut entry.value, definitions)?;
            }
            Ok(())
        }
    }
}

pub fn required_object<'a>(value: &'a Value, label: &str) -> Result<&'a [Entry]> {
    value
        .as_object()
        .ok_or_else(|| GameUiError::parse(format!("{label} is not a block")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn keyless_lists_preserve_mixed_and_nested_scalar_kinds() {
        use super::{Scalar, Value};
        let root = super::parse_str_strict(r#"list = { 5 "5" { "6" 6 } "quoted_key" = "7" }"#).unwrap();
        let rows = root.first_value_ci("list").unwrap().as_object().unwrap();
        assert_eq!(rows[0].value, Value::Scalar(Scalar::Bare("5".into())));
        assert_eq!(rows[1].value, Value::Scalar(Scalar::Quoted("5".into())));
        assert_eq!(rows[0].value.as_scalar(), rows[1].value.as_scalar());
        let nested = rows[2].value.as_object().unwrap();
        assert_eq!(nested[0].value, Value::Scalar(Scalar::Quoted("6".into())));
        assert_eq!(nested[1].value, Value::Scalar(Scalar::Bare("6".into())));
        assert_eq!(rows[3].key.as_deref(), Some("quoted_key"));
        assert_eq!(rows[3].value, Value::Scalar(Scalar::Quoted("7".into())));
        assert!(rows[..3].iter().all(|row| row.key.is_none()));
    }

    #[test]
    fn keyless_variable_expansion_keeps_definition_kind_and_quoted_literals() {
        use super::{Scalar, Value};
        let mut root = super::parse_str_strict(r#"@quoted = "8" @bare = 9 list = { @quoted @bare "@missing" { @quoted "@bare" } }"#).unwrap();
        super::expand_file_scoped_variables(&mut root).unwrap();
        let rows = root.first_value_ci("list").unwrap().as_object().unwrap();
        assert_eq!(rows[0].value, Value::Scalar(Scalar::Quoted("8".into())));
        assert_eq!(rows[1].value, Value::Scalar(Scalar::Bare("9".into())));
        assert_eq!(rows[2].value, Value::Scalar(Scalar::Quoted("@missing".into())));
        let nested = rows[3].value.as_object().unwrap();
        assert_eq!(nested[0].value, Value::Scalar(Scalar::Quoted("8".into())));
        assert_eq!(nested[1].value, Value::Scalar(Scalar::Quoted("@bare".into())));
        let mut unknown = super::parse_str_strict("list = { @missing }").unwrap();
        assert!(matches!(super::expand_file_scoped_variables(&mut unknown), Err(super::VariableExpansion::Undefined { .. })));
    }

    #[test]
    fn asset_reader_terminates_at_root_close_without_reading_later_declarations() {
        let text = "entity = { name = \"\u{e9}t\u{e9}}\" } } entity = { name = discarded }";
        let (root, prefix) = super::parse_asset_str(text).unwrap();
        assert_eq!(root.values_ci("entity").count(), 1);
        assert_eq!(root.values_ci("entity").next().unwrap().first_scalar_ci("name"), Some("\u{e9}t\u{e9}}"));
        assert_eq!(prefix, "entity = { name = \"\u{e9}t\u{e9}}\" } }");
    }

    #[test]
    fn asset_reader_eof_closes_persistent_blocks_but_refuses_missing_values() {
        let (root, normalized) = super::parse_asset_str("entity = { state = { name = idle").unwrap();
        assert_eq!(normalized, "entity = { state = { name = idle\n}}");
        assert_eq!(root.values_ci("entity").next().unwrap().values_ci("state").next().unwrap().first_scalar_ci("name"), Some("idle"));
        let (_, commented) = super::parse_asset_str("entity = { name = idle # }").unwrap();
        assert_eq!(commented, "entity = { name = idle # }\n}");
        assert!(super::parse_str_strict(&commented).is_ok());
        assert!(super::parse_asset_str("entity = { name = }").is_err());
        assert!(super::parse_asset_str("entity = { name =").is_err());
    }

    use super::*;

    /// The strict parse accepts the one recovery the engine itself tolerates
    /// and refuses every other.
    ///
    /// kills: a consumer that treats every recovery as damage, which refuses a
    /// file the game reads; and one that treats the tolerance as covering more
    /// than the single outer end-of-file closure.
    #[test]
    fn the_strict_parse_tolerates_only_the_engines_own_eof_closure() {
        let tolerated = "type = { name = \"a\" object = { name = \"o\" }";
        let root = parse_str_strict(tolerated).expect("the engine reads this file");
        let declared = root.first_value_ci("type").expect("the type survives");
        assert_eq!(declared.first_scalar_ci("name"), Some("a"));
        assert!(declared.first_value_ci("object").is_some());
        // Two unclosed blocks are not the tolerated shape.
        assert!(parse_str_strict("a = { b = { c = 1").is_err());
        // A closing brace inside a COMMENT is text, not the file's last token,
        // and a file that ends there is truncated.
        assert!(parse_str_strict("a = { b = 1 # }").is_err());
        assert!(parse_str_strict("a = { b = 1 #").is_err());
        // The same brace inside a quoted value is text too.
        assert!(parse_str_strict("a = { b = \"}\"").is_err());
        assert!(!parse_str("a = { b = 1 # }").ends_on_structural_close);
        assert!(parse_str("a = { b = 1 }").ends_on_structural_close);
        // Neither is a file that does not end on its final closing brace.
        assert!(parse_str_strict("a = { b = 1 } c = { d = 2").is_err());
        // Nor any other recovery.
        assert!(parse_str_strict("} a = 1").is_err());
        assert!(parse_str_strict("a = \"unterminated").is_err());
    }

    /// The strict decoder refuses EVERY destructive loss and accepts the two
    /// that discard nothing.
    ///
    /// kills: a consumer that samples the destructive variants itself and misses
    /// one -- an unpaired surrogate inside a quoted token then decodes to
    /// `U+FFFD` and is read as the authored value.
    #[test]
    fn the_strict_decoder_refuses_every_loss_that_discards_input() {
        let utf16le = |text: &str, injected: Option<u16>| {
            let mut bytes = vec![0xffu8, 0xfe];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            if let Some(unit) = injected {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        };
        // Invalid UTF-8 after a byte-order mark.
        assert!(matches!(
            decode_text_strict(b"\xef\xbb\xbfname = \"a\xe2\x82\""),
            Err(TextDecodeLoss::InvalidUtf8 { .. })
        ));
        // An unpaired UTF-16 surrogate.
        assert!(matches!(
            decode_text_strict(&utf16le("name = ", Some(0xd800))),
            Err(TextDecodeLoss::UnpairedUtf16Surrogate { .. })
        ));
        // A truncated UTF-16 code unit.
        assert!(matches!(
            decode_text_strict(b"\xff\xfea\x00b"),
            Err(TextDecodeLoss::TruncatedUtf16Unit { .. })
        ));
        // Every variant `discards_input` reports is one of the three above plus
        // `Suppressed`, and this is what says so if a variant is ever added.
        assert!(TextDecodeLoss::Suppressed { count: 1 }.discards_input());
        assert!(!TextDecodeLoss::NonUtf8DecodedAsLatin1 {
            byte_offset: 0,
            line: 1,
            column: 1
        }
        .discards_input());
        // The two that discard nothing.
        assert_eq!(
            decode_text_strict(b"name = \"caf\xe9\"").as_deref(),
            Ok("name = \"caf\u{e9}\"")
        );
        assert_eq!(
            decode_text_strict(b"\xef\xbb\xbfname = \"a\"").as_deref(),
            Ok("name = \"a\"")
        );
        assert_eq!(decode_text_strict(&utf16le("name = 1", None)).as_deref(), Ok("name = 1"));
    }

    #[test]
    fn file_scoped_variables_expand_to_the_definitions_own_scalar_kind() {
        let mut doc = parse_str(
            r#"
            @font = "hoi_16mbs"
            @alias = @font
            @size = 4
            root = { font = @alias size = @size literal = "@font" }
            "#,
        );
        assert!(doc.recoveries.is_empty());
        expand_file_scoped_variables(&mut doc.root).expect("every reference is defined");
        let root = doc.root.first_value_ci("root").unwrap();
        assert_eq!(
            root.first_value_ci("font"),
            Some(&Value::Scalar(Scalar::Quoted("hoi_16mbs".into())))
        );
        assert_eq!(
            root.first_value_ci("size"),
            Some(&Value::Scalar(Scalar::Bare("4".into())))
        );
        // A quoted value is text, not a reference.
        assert_eq!(
            root.first_value_ci("literal"),
            Some(&Value::Scalar(Scalar::Quoted("@font".into())))
        );
    }

    #[test]
    fn an_undefined_or_cyclic_variable_is_refused_rather_than_left_in_place() {
        let mut doc = parse_str("root = { size = @missing }");
        assert_eq!(
            expand_file_scoped_variables(&mut doc.root),
            Err(VariableExpansion::Undefined {
                reference: "@missing".into(),
                undefined: "@missing".into()
            })
        );
        let mut doc = parse_str("@a = @b\n@b = @a\nroot = { size = @a }");
        assert_eq!(
            expand_file_scoped_variables(&mut doc.root),
            Err(VariableExpansion::Cyclic {
                reference: "@a".into()
            })
        );
    }

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

    #[test]
    fn lone_eof_block_close_on_a_cleanly_ended_file_is_engine_tolerated() {
        let bytes = b"spriteTypes = {\n spriteType = { name = A }\n";
        let doc = parse_bytes(bytes);
        assert_eq!(doc.recoveries.len(), 1);
        assert_eq!(
            doc.recoveries[0].message,
            ENGINE_TOLERATED_EOF_CLOSE_RECOVERY
        );
        assert!(engine_tolerated_outer_eof(&doc));
    }

    #[test]
    fn eof_close_tolerance_requires_one_recovery_and_a_final_brace() {
        let doc = parse_str("a = { b = { c = 1\n");
        assert_eq!(
            doc.recoveries.len(),
            2,
            "two open braces closed by EOF are not the tolerated single closure"
        );
        assert!(!engine_tolerated_outer_eof(&doc));

        let no_final_brace = b"a = { b = 1\ntrailing\n";
        let doc = parse_bytes(no_final_brace);
        assert_eq!(doc.recoveries.len(), 1);
        assert!(!engine_tolerated_outer_eof(&doc));

        let damaged = b"a = { b = 1 } }\n";
        let doc = parse_bytes(damaged);
        assert_eq!(doc.recoveries.len(), 1);
        assert_ne!(
            doc.recoveries[0].message,
            ENGINE_TOLERATED_EOF_CLOSE_RECOVERY
        );
        assert!(!engine_tolerated_outer_eof(&doc));

        let clean = b"a = { b = 1 }\n";
        let doc = parse_bytes(clean);
        assert!(doc.recoveries.is_empty());
        assert!(!engine_tolerated_outer_eof(&doc));
    }
}
