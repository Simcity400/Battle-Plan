//! Script reader consumption over the shared lexer, token by token, including text
//! whose consumption differs from balanced tree grammar.
//!
//! The tree ([`super::parse`]) pairs braces as written. Script fields instead consume
//! according to their dispatch, and the next field continues there. A block passed to a
//! one-value reader is not skipped; a value without a key consumes the next token as
//! its operator; a definition whose name is taken leaves its `=` for the next key.
//! [`read`] reproduces that consumption:
//!
//! - [`lex`] uses each loader's [`Mode`], skipping a byte-order mark only in mode 1. A
//!   byte 0xFF ends a read, and reading can continue after it.
//! - [`Reader`] holds key, operator and value slots, retaining them when a later read
//!   finds no operator, plus the file's `@name` table.
//! - Block loops peek, stop at `}` or the end, otherwise read a key/operator/value and
//!   use the caller's [`Shape`] dispatch.
//! - File loops stop only at the end.
//!
//! The result retains each key, its resolved value token or consumed block, and
//! [`Read::misreads`] for each departure from the text's shape, by line.

use super::{Item, Operator, Value};
use crate::key_names::KeyNames;

mod math;

/// A lexer token kind distinguished by field readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A bare token (kind 0 before the keyword lookup).
    Bare,
    /// A quoted string (token 0xF).
    Quoted,
    /// `=` (token 1).
    Equals,
    /// `<` (token 0x1D4).
    Less,
    /// `>` (token 0x1D3).
    Greater,
    /// `{` (token 3).
    Open,
    /// `}` (token 4).
    Close,
    /// `(` `)` `,` and `?=` (tokens 5, 6, 8, 0x318).
    Other,
    /// The end of the text, or the empty value stored before a `}`.
    End,
    /// A comment marker (`!`, `#`, `;`), stored before the comment is skipped. It
    /// remains the lexer's token only when the following retry fails ([`lex`]).
    Comment,
    /// An effect file's `[[ ... ]]` code ([`lex_effect`]): raw bytes consumed after
    /// `[[`, without lexing.
    Code,
}

/// One token: its kind, its text (a single-character token's text is that
/// character) and the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub text: Vec<u8>,
    pub line: u32,
    /// The token's quoted flag, stored separately from its kind. A failed peek changes
    /// the kind to End and keeps this flag, which variable resolution tests.
    pub quoted: bool,
    /// Byte position after this lexer read, before the next peek.
    pub byte_end: usize,
}

impl Token {
    fn end(line: u32) -> Self {
        Self {
            kind: Kind::End,
            text: Vec::new(),
            line,
            quoted: false,
            byte_end: 0,
        }
    }

    /// An initial reader slot: token 0, unquoted and empty, matching no keyword.
    fn slot() -> Self {
        Self {
            kind: Kind::Bare,
            text: Vec::new(),
            line: 0,
            quoted: false,
            byte_end: 0,
        }
    }

    fn operator(&self) -> bool {
        matches!(self.kind, Kind::Equals | Kind::Less | Kind::Greater)
    }

    /// The operator this token is, as [`Item::operator`] keeps it.
    fn as_operator(&self) -> Operator {
        match self.kind {
            Kind::Less => Operator::Less,
            Kind::Greater => Operator::Greater,
            _ => Operator::Equals,
        }
    }

    fn shown(&self) -> String {
        match self.kind {
            Kind::End => "the end of the file".into(),
            Kind::Quoted => format!("\"{}\"", String::from_utf8_lossy(&self.text)),
            _ => format!("`{}`", String::from_utf8_lossy(&self.text)),
        }
    }

    /// The token's quoted flag: a quoted string, whatever id a failed peek gave it.
    pub fn quoted(&self) -> bool {
        self.quoted
    }

    /// A read that returned 0 (token 0x13).
    pub fn is_end(&self) -> bool {
        self.kind == Kind::End
    }

    /// `}` or End: the tokens that stop every block loop.
    pub fn ends_block(&self) -> bool {
        matches!(self.kind, Kind::Close | Kind::End)
    }

    /// The first byte of the text, which the readers compare with `@`.
    pub fn starts_with_at(&self) -> bool {
        self.text.first() == Some(&b'@')
    }

    /// The text up to its first NUL, as every consumer reads it.
    pub fn c_str(&self) -> &[u8] {
        crate::parser::c_string(&self.text)
    }

    /// The number id 0xC ([`super::number_token`]) of a bare token.
    pub fn is_number(&self) -> bool {
        self.kind == Kind::Bare && super::number_token(&self.text, false)
    }

    /// Text eligible for keyword lookup: a bare token not numbered by its first byte,
    /// so `3dposition` cannot be a keyword. Other kinds and empty C strings give
    /// `None`.
    pub fn lookup(&self) -> Option<&[u8]> {
        let numbered = self
            .text
            .first()
            .is_some_and(|first| first.is_ascii_digit() || *first == b'-');
        if self.kind != Kind::Bare || numbered {
            return None;
        }
        let text = self.c_str();
        (!text.is_empty()).then_some(text)
    }

    /// Whether the token is keyword `word`: its [`Self::lookup`] text matches under the
    /// lexer's ASCII case fold.
    pub fn is_keyword(&self, word: &str) -> bool {
        self.lookup()
            .is_some_and(|text| text.eq_ignore_ascii_case(word.as_bytes()))
    }
}

/// C-locale white space skipped by the lexer.
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// [`lex`] returns tokens in read order. A [`Kind::End`] token represents a failed read
/// at 0xFF or EOF, retaining the text of a bare token it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lexed<'s> {
    pub tokens: Vec<Token>,
    source: Option<TextSource<'s>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextSource<'s> {
    bytes: &'s [u8],
    code: bool,
}

/// The loader's lexer mode. Mode 1 skips a leading UTF-8 byte-order mark; mode 0
/// retains it. Each reader supplies its loader's mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Mode 0: a mark's three bytes start the first token.
    KeepMark,
    /// Mode 1: a leading mark is skipped.
    SkipMark,
}

/// Read one byte, returning `None` for 0xFF or past EOF. The position advances in
/// either case.
fn next(bytes: &[u8], at: &mut usize) -> Option<u8> {
    let byte = bytes.get(*at).copied();
    *at += 1;
    byte.filter(|byte| *byte != 0xFF)
}

/// Lex the whole text, skipping a leading UTF-8 byte-order mark in [`Mode::SkipMark`].
/// `!`, `#` and `;` start line comments. A quote starts a string where escaped quotes
/// and backslashes become one character and other backslashes stay. `(` `)` `,` `<` `=`
/// `>` `{` `}` are single-character tokens; `?=` is one token and a lone `?` is
/// dropped. Other text is bare up to white space or those characters.
///
/// A byte 0xFF ends a read like EOF, and reading continues after it ([`next`]). At a
/// token start it returns [`Kind::End`]; after a bare token it also retains the text.
/// In a comment it ends only the comment; in a quoted string it ends the string, which
/// succeeds.
///
/// A lone `?` or comment marker is stored before retrying. The retry skips white space
/// and may fail before clearing that token, leaving its text in End. A token found by
/// the retry replaces it.
pub fn lex(bytes: &[u8], mode: Mode) -> Lexed<'_> {
    lex_with(bytes, mode, false)
}

/// [`lex`] for an effect (`.shader`/`.fxh`) file consumes code after `[[` raw: skip
/// white space through the first line feed (only spaces before any other byte), then
/// take everything to the first `]]` or EOF, without nesting or escaping. NUL cuts the
/// text, which becomes one [`Kind::Code`] token.
pub fn lex_effect(bytes: &[u8], mode: Mode) -> Lexed<'_> {
    lex_with(bytes, mode, true)
}

fn lex_with(bytes: &[u8], mode: Mode, code: bool) -> Lexed<'_> {
    let bytes = match mode {
        Mode::KeepMark => bytes,
        Mode::SkipMark => bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes),
    };
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line = 1_u32;
    let single = |byte: u8| match byte {
        b'=' => Some(Kind::Equals),
        b'<' => Some(Kind::Less),
        b'>' => Some(Kind::Greater),
        b'{' => Some(Kind::Open),
        b'}' => Some(Kind::Close),
        b'(' | b')' | b',' => Some(Kind::Other),
        _ => None,
    };
    // The `?` or comment marker this read wrote before retrying.
    let mut kept = Vec::new();
    loop {
        if at >= bytes.len() {
            // Past the end every read returns 0; the first keeps what the
            // retry left.
            if !kept.is_empty() {
                tokens.push(Token {
                    kind: Kind::End,
                    text: std::mem::take(&mut kept),
                    line,
                    quoted: false,
                    byte_end: at.min(bytes.len()),
                });
            }
            break;
        }
        let Some(byte) = next(bytes, &mut at) else {
            tokens.push(Token {
                kind: Kind::End,
                text: std::mem::take(&mut kept),
                line,
                quoted: false,
                byte_end: at.min(bytes.len()),
            });
            continue;
        };
        if space(byte) {
            if byte == b'\n' {
                line += 1;
            }
            continue;
        }
        kept.clear();
        match byte {
            b'!' | b'#' | b';' => {
                while bytes
                    .get(at)
                    .is_some_and(|byte| *byte != b'\n' && *byte != 0xFF)
                {
                    at += 1;
                }
                // A -1 ends the comment, and is consumed with it.
                if bytes.get(at) == Some(&0xFF) {
                    at += 1;
                }
                kept.push(byte);
            }
            b'"' => {
                let start_line = line;
                let mut text = Vec::new();
                let mut previous = 0_u8;
                while let Some(byte) = next(bytes, &mut at) {
                    if byte == b'\n' {
                        line += 1;
                    }
                    if byte == b'"' && previous != b'\\' {
                        break;
                    }
                    if previous == b'\\' && (byte == b'"' || byte == b'\\') {
                        text.pop();
                        text.push(byte);
                        previous = 0;
                        continue;
                    }
                    text.push(byte);
                    previous = byte;
                }
                tokens.push(Token {
                    kind: Kind::Quoted,
                    text,
                    line: start_line,
                    quoted: true,
                    byte_end: at.min(bytes.len()),
                });
            }
            b'?' => match bytes.get(at) {
                Some(b'=') => {
                    at += 1;
                    tokens.push(Token {
                        kind: Kind::Other,
                        text: b"?=".to_vec(),
                        line,
                        quoted: false,
                        byte_end: at.min(bytes.len()),
                    });
                }
                // The byte is pushed back and the read retries, the `?`
                // kept as the token until a token replaces it.
                _ => kept.push(b'?'),
            },
            _ => {
                if let Some(kind) = single(byte) {
                    tokens.push(Token {
                        kind,
                        text: vec![byte],
                        line,
                        quoted: false,
                        byte_end: at.min(bytes.len()),
                    });
                    continue;
                }
                let start = at - 1;
                while at < bytes.len()
                    && bytes[at] != 0xFF
                    && !space(bytes[at])
                    && !matches!(
                        bytes[at],
                        b'!' | b'"'
                            | b'#'
                            | b'('
                            | b')'
                            | b','
                            | b';'
                            | b'<'
                            | b'='
                            | b'>'
                            | b'{'
                            | b'}'
                    )
                {
                    at += 1;
                }
                let text = bytes[start..at].to_vec();
                // A -1 ends the token and is consumed: the failed read
                // keeps the token text.
                let kind = if at == bytes.len() || bytes[at] == 0xFF {
                    at += 1;
                    Kind::End
                } else {
                    Kind::Bare
                };
                if code && kind == Kind::Bare && text == b"[[" {
                    let mut from = at;
                    match bytes[from..]
                        .iter()
                        .position(|byte| !space(*byte) || *byte == b'\n')
                    {
                        Some(offset) if bytes[from + offset] == b'\n' => from += offset + 1,
                        _ => {
                            while bytes.get(from).is_some_and(|byte| space(*byte)) {
                                from += 1;
                            }
                        }
                    }
                    let end = bytes[from..]
                        .windows(2)
                        .position(|pair| pair == b"]]")
                        .map_or(bytes.len(), |end| from + end);
                    let start_line = line;
                    line += bytes[at..end].iter().filter(|byte| **byte == b'\n').count() as u32;
                    at = (end + 2).min(bytes.len());
                    tokens.push(Token {
                        kind: Kind::Code,
                        text: crate::parser::c_string(&bytes[from..end]).to_vec(),
                        line: start_line,
                        quoted: false,
                        byte_end: at.min(bytes.len()),
                    });
                    continue;
                }
                tokens.push(Token {
                    kind,
                    text,
                    line,
                    quoted: false,
                    byte_end: at.min(bytes.len()),
                });
            }
        }
    }
    Lexed {
        tokens,
        source: Some(TextSource { bytes, code }),
    }
}

/// Field dispatch after reading its key, operator and value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape<S> {
    /// Consume only the value token, or log a key without a reporting skip. Given `{`,
    /// subsequent contents become enclosing fields and its `}` closes the enclosing
    /// block. Reporting an unexpected key instead uses [`Shape::Skip`] to skip its
    /// block.
    One,
    /// Like [`Shape::One`], but take the lexer's current token rather than the value
    /// slot. An `@name` retains its reference text because only the value slot
    /// resolves; an empty value before `}` takes `}`.
    Current,
    /// Read the following tokens as an object block, dispatching every field with `S`,
    /// without checking that the value was `{`.
    Object(S),
    /// Read an object using the caller's stored dynamic-token key.
    ObjectAs(S, Vec<u8>),
    /// Read an object only for value `{`; otherwise take one value.
    ObjectIfOpen(S),
    /// Read an object only for value `{`; otherwise report and use [`Shape::Skip`]
    /// recovery.
    ObjectOrSkip(S),
    /// Read the object twice, as required for a newly created default adjuster.
    Twice(S),
    /// Read every token up to `}` or the end, resolving each into the value slot.
    List,
    /// Read a fixed-size raw span until close, reporting only its first excess token
    /// and skipping its block if present. Keep tokens for caller conversion.
    BoundedList(usize),
    /// Read a list only for value `{`; otherwise take one value.
    ListIfOpen,
    /// Read tokens after the value until brace depth, initially 1, reaches zero or EOF.
    /// The value itself is not counted. Resolve into the lexer rather than the value
    /// slot; keep nonbrace tokens as anonymous values.
    Tokens,
    /// Read a list only for value `{`; otherwise report and use [`Shape::Skip`]
    /// recovery.
    ListOrSkip,
    /// For value `{`, read one entry with dispatch `S`. Report a nonclosing token after
    /// it, skipping its block if present, then skip to the value block's closing `}`
    /// without definitions. Other values report and use [`Shape::Skip`].
    Pair(S),
    /// For value `{`, read two raw tokens as strings without keys or operators. At `}`
    /// or EOF, report if fewer than two were read and consume the closing token. Other
    /// tokens, including `{`, resolve into the value slot before conversion through
    /// [`super::Value::read_string`]. Report a third token, skip its block if present,
    /// then skip to the pair's `}` without definitions. Other values use
    /// [`Shape::Skip`].
    Strings,
    /// Report, then skip the whole block only when the current lexer token is `{`,
    /// defining nothing. Resolve each token during the skip, so a reference retaining
    /// `{` opens a nested skip.
    Skip,
    /// Reporting skip without a stored definition.
    Discard,
    /// Without reporting, skip literal tokens to the matching `}` only when the value
    /// token is `{`, defining nothing.
    SkipIfOpen,
    /// Read a known nonassign trigger body. For `{`, peek until `}` or the end and
    /// consume the closing token. Dispatch each key with `S`, including keys without
    /// operators whose slots remain stale ([`Field::fresh`] is false). Anonymous tokens
    /// become keys rather than list entries. Other values report and use
    /// [`Shape::Skip`].
    Body(S),
    /// A trigger [`Shape::Body`] used by content fields such as an idea's
    /// `allowed`, through the shared trigger dispatch
    /// ([`crate::combat_tactics::Shaper`]) against the content names the
    /// read was given ([`read`]).
    Trigger,
    /// Mean-time-to-happen modifiers treat `factor` and `add` as scoped variables;
    /// other keys use shared trigger dispatch.
    MtthModifier,
    /// Read an effect script through the shared effect dispatch.
    Effect,
    /// The effect caller's parse kind.
    EffectIn(u32),
    /// An effect value reader's key loop, dispatched through
    /// `effect_keys::value_readers` outside the default value paths.
    EffectValueKeys(u64),
    /// An effect script with a dedicated key reader or inline value loop,
    /// selected by effect token.
    EffectKeys(u32),
    /// An effect body whose custom key grammar is deferred. Its shape is
    /// decided by the effect dispatch.
    EffectByKey { trigger_limits: bool },
    /// Report a counted warning, then follow the selected consumption.
    /// Repeated reports in one file keep their first line and total count.
    Reported {
        kind: crate::DiagnosticKind,
        report: String,
        read: Box<Shape<S>>,
    },
    /// A block read by key whose reader the port does not interpret: an
    /// effect, or an unported nested value reader (see
    /// [`Reader::generic`]).
    ByKey,
    /// End the reader's loop after taking this field's value. The closing read consumes
    /// the next token regardless of kind, then the enclosing reader continues. At file
    /// top level this is [`Shape::One`].
    Break,
    /// Report and skip its block, then end this loop and consume its closing token.
    SkipBreak,
    /// Report and skip its block, then return from this file's reader.
    StopFile,
    /// Read color from the current lexer token. A number is one packed value. After
    /// `{`, read three components and a fourth token: if it is not `}`, it is alpha and
    /// one more token follows. Resolve each token, reporting nonnumeric components or a
    /// final token other than `}`. Report any other initial token without reading
    /// further.
    Color,
    /// Read custom statements. Anonymous values and blocks clear the key slot;
    /// assignments resolve all three tokens without defining variables. The caller
    /// supplies field dispatch.
    Statements(S),
    /// Read raw lexer tokens, resolving into the lexer without changing the value slot,
    /// unlike string-list reads.
    RawList,
    /// Read a sub-unit type array with one raw entry before testing its delimiter.
    RawListFirst,
    /// Read a division-template point as ordered raw coordinates.
    TemplatePoint,
    /// Read a country-token array, resolving raw entries into the value slot while
    /// leaving that slot intact for the closing token.
    ValueList,
    /// Read optional equipment assets in an unconditional raw loop. Each nonclosing
    /// entry resolves into both lexer and value slot before an object read with `S`.
    /// Entries need not be `{`; their object reads following fields until `}`. The
    /// final list closing token changes only the lexer.
    ObjectList(S),
    /// Read validated token arrays. An opening token cannot name a database item, so
    /// reporting skips its nested block.
    CheckedList { stores_value: bool },
    /// Skip raw tokens unconditionally, without reporting.
    RawSkip,
    /// Read raw characters after the current lexer read up to its matching brace,
    /// including braces inside comments and quotes.
    RawText,
    /// Read a math expression's initial `value`, recursive operands, operator tail and
    /// reporting skip.
    Expression,
    /// For an on-research-complete body, peek after the value and read a bare `limit`
    /// as a trigger ([`Shape::Trigger`]). Then read effects with `S` only if the
    /// resulting value slot holds `{`; otherwise report without further consumption. A
    /// read limit becomes the block's first item.
    LimitedBody(S),
    /// Read a scripted key as a scalar C string or case entries. Case names resolve as
    /// tokens without defining variables; their values are triggers.
    ScriptedKey,
    /// Read enabled equipment entries with their inline limit trigger. Conditional
    /// tails beyond one token are refused because the trigger result would change
    /// subsequent consumption.
    TechnologyEquipment,
    /// Whether the reader reads this field at all depends on what the port
    /// cannot tell here (why): the rest of the file is [`Read::unsettled`].
    Unsettled(String),
}

impl<S> Shape<S> {
    /// The same shape with its reader argument mapped, for a dispatch that
    /// embeds another reader's (a modifier block inside an idea).
    pub fn map<T>(self, node: impl FnOnce(S) -> T) -> Shape<T> {
        match self {
            Self::One => Shape::One,
            Self::Current => Shape::Current,
            Self::Object(s) => Shape::Object(node(s)),
            Self::ObjectAs(s, key) => Shape::ObjectAs(node(s), key),
            Self::ObjectIfOpen(s) => Shape::ObjectIfOpen(node(s)),
            Self::ObjectOrSkip(s) => Shape::ObjectOrSkip(node(s)),
            Self::Twice(s) => Shape::Twice(node(s)),
            Self::List => Shape::List,
            Self::BoundedList(n) => Shape::BoundedList(n),
            Self::ListIfOpen => Shape::ListIfOpen,
            Self::Tokens => Shape::Tokens,
            Self::ListOrSkip => Shape::ListOrSkip,
            Self::Pair(s) => Shape::Pair(node(s)),
            Self::Strings => Shape::Strings,
            Self::Skip => Shape::Skip,
            Self::Discard => Shape::Discard,
            Self::SkipIfOpen => Shape::SkipIfOpen,
            Self::Body(s) => Shape::Body(node(s)),
            Self::Trigger => Shape::Trigger,
            Self::MtthModifier => Shape::MtthModifier,
            Self::Effect => Shape::Effect,
            Self::EffectIn(scope) => Shape::EffectIn(scope),
            Self::EffectValueKeys(reader) => Shape::EffectValueKeys(reader),
            Self::EffectKeys(token) => Shape::EffectKeys(token),
            Self::EffectByKey { trigger_limits } => Shape::EffectByKey { trigger_limits },
            Self::Reported { kind, report, read } => Shape::Reported {
                kind,
                report,
                read: Box::new((*read).map(node)),
            },
            Self::ByKey => Shape::ByKey,
            Self::Break => Shape::Break,
            Self::SkipBreak => Shape::SkipBreak,
            Self::StopFile => Shape::StopFile,
            Self::Color => Shape::Color,
            Self::Statements(s) => Shape::Statements(node(s)),
            Self::RawList => Shape::RawList,
            Self::RawListFirst => Shape::RawListFirst,
            Self::TemplatePoint => Shape::TemplatePoint,
            Self::ValueList => Shape::ValueList,
            Self::ObjectList(s) => Shape::ObjectList(node(s)),
            Self::CheckedList { stores_value } => Shape::CheckedList { stores_value },
            Self::RawSkip => Shape::RawSkip,
            Self::RawText => Shape::RawText,
            Self::Expression => Shape::Expression,
            Self::ScriptedKey => Shape::ScriptedKey,
            Self::LimitedBody(s) => Shape::LimitedBody(node(s)),
            Self::TechnologyEquipment => Shape::TechnologyEquipment,
            Self::Unsettled(why) => Shape::Unsettled(why),
        }
    }
}

/// Mean-time-to-happen fields `factor`, `base`, `days`, `months` and `years` take one
/// value. `modifier` reads a modifier trigger body by key. Other keys are logged, with
/// their value token read and nothing skipped.
pub fn mean_time_to_happen<S>(field: &Field<'_>) -> Shape<S> {
    match field.keyword().as_deref() {
        Some(b"modifier") => Shape::MtthModifier,
        _ => Shape::One,
    }
}

/// Classify an uninterpreted trigger or effect body field as documented by
/// [`Reader::generic`]. Every key dispatches with the current value slot, including
/// keys without operators. A punctuation key cannot name a trigger, effect or scope and
/// uses a reporting skip on the current lexer token. Otherwise a fresh `{` reads by key
/// ([`Shape::ByKey`]); a stale block is unsettled. A block after a missing operator
/// could be enclosing fields for a one-value reader or be skipped by reporting, which
/// cannot be distinguished here. Other values consume one token either way.
pub fn untraced<S>(field: &Field<'_>) -> Shape<S> {
    if !matches!(field.key.kind, Kind::Bare | Kind::Quoted) {
        Shape::Skip
    } else if field.opens() {
        Shape::ByKey
    } else if field.skips_block() {
        Shape::Unsettled(format!(
            "key {} has no operator and a block follows it: a one-value reader reads the block as the enclosing fields, a reporting skip passes over it, and the field reader is unknown",
            field.key.shown()
        ))
    } else {
        Shape::One
    }
}

/// A field as a reader's dispatch sees it.
#[derive(Debug)]
pub struct Field<'a> {
    /// The key token.
    pub key: &'a Token,
    /// The value token, resolved; stale when `fresh` is false.
    pub value: &'a Token,
    /// An operator was found and this field's own value was read.
    pub fresh: bool,
    /// The operator slot, including a previous operator left stale.
    pub operator: Operator,
    /// The current lexer token: the value, `}` for an empty value, or the token
    /// consumed as a missing operator. Reporting skips test this token rather than the
    /// value slot.
    pub current: &'a Token,
    /// Native effect parse kind; None means the caller did not supply it.
    pub effect_scope: Option<u32>,
}

impl Field<'_> {
    /// The key as keyword dispatch sees it: a folded bare key. Quoted keys, punctuation
    /// and bare keys starting with a digit or `-` return `None` ([`super::looked_up`]).
    pub fn keyword(&self) -> Option<Vec<u8>> {
        super::looked_up(&self.key.text, self.key.kind == Kind::Quoted)
            .then(|| self.key.text.to_ascii_lowercase())
    }

    /// Whether the key is keyword `name` (see [`Self::keyword`]).
    pub fn keyword_is(&self, name: &str) -> bool {
        self.keyword()
            .is_some_and(|keyword| keyword == name.as_bytes())
    }

    /// The value slot's token as a reader that dispatches on its id sees
    /// it, as [`super::Item::value_keyword`]: the folded text of a bare
    /// token the lexer looked up; `None` for any other kind.
    pub fn value_keyword(&self) -> Option<Vec<u8>> {
        (self.value.kind == Kind::Bare && super::looked_up(&self.value.text, false))
            .then(|| self.value.text.to_ascii_lowercase())
    }

    /// Whether the value token is `{`.
    pub fn opens(&self) -> bool {
        self.value.kind == Kind::Open
    }

    /// Whether the current token is `{`, whose block a reporting skip passes over.
    pub fn skips_block(&self) -> bool {
        self.current.kind == Kind::Open
    }
}

/// A native reading note or counted port warning, with its first line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misread {
    pub line: u32,
    pub what: String,
    pub kind: crate::DiagnosticKind,
    /// A repeated warning's occurrences in this file; ordinary native
    /// reading notes are not aggregated.
    pub count: Option<usize>,
}

impl std::fmt::Display for Misread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.what)?;
        if let Some(count) = self.count {
            write!(f, " (count: {count})")?;
        }
        Ok(())
    }
}

/// What [`read`] gives: the items as the readers saw them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub items: Vec<Item>,
    pub misreads: Vec<Misread>,
    /// Diagnostics for each top-level item, in `items` order and with the
    /// same length. Direct persistent reads keep empty entries: their
    /// diagnostic ownership is unavailable.
    /// Counted reports retain the occurrences in that item, even when the
    /// file-wide `misreads` combines the same report from several items.
    pub item_misreads: Vec<Vec<Misread>>,
    /// Diagnostics from file fields that kept no item, variable definitions
    /// at any nesting depth, or a direct persistent read. Their ownership is
    /// file-wide or unavailable.
    pub unattributed_misreads: Vec<Misread>,
    /// A block read by key whose reading the port cannot settle, and why:
    /// everything after it in the file is unknown and not in `items`.
    pub unsettled: Option<Misread>,
}

/// The outcome of one key/operator/value read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    /// A key, an operator and the value.
    Fresh,
    /// A key and a token that is no operator, consumed in its place.
    NoOperator,
    /// Nothing: the text ended after definitions.
    Ended,
}

/// Reader state: the token stream with one pending peek, key/operator/value slots and
/// the variable table.
pub struct Reader<'n> {
    tokens: Vec<Token>,
    source: Option<TextSource<'n>>,
    /// The positions of the [`Kind::End`] tokens, in order, so a look ahead
    /// finds the next one without walking the text.
    ends: Vec<usize>,
    next: usize,
    /// The current lexer token. A failed advance at EOF leaves it unchanged; a failed
    /// peek changes its kind to End and retains its text. Resolved tokens are written
    /// back here.
    pub current: Token,
    /// A peek read `current` ahead (+100); the next advance returns its
    /// result again.
    pending: bool,
    pending_result: bool,
    /// The key slot.
    pub key: Token,
    /// The operator slot, which each [`Item::operator`] keeps.
    operator: Operator,
    /// The value slot, resolved.
    pub value: Token,
    /// `@name` definitions: FNV-1a of the name after `@`, plus the whole defining token
    /// including kind, quoted flag and text.
    variables: Vec<(u32, Token)>,
    misreads: Vec<Misread>,
    reading_notes: Vec<Misread>,
    item_misreads: Vec<Vec<Misread>>,
    unattributed_misreads: Vec<Misread>,
    unsettled: Option<Misread>,
    /// A field's reader ended its loop ([`Shape::Break`]).
    broken: bool,
    /// The content names a trigger key is read against ([`Shape::Trigger`]).
    names: Option<&'n KeyNames>,
    stopped: bool,
    effect_scope: Option<u32>,
}

/// Select a file loop or a direct object-block read.
#[derive(Clone, Copy, Debug)]
pub enum Top<S> {
    /// A database's custom file loop: only the end of the text stops it.
    File(S),
    /// An object loop: the first closing brace also stops it.
    Persistent(S),
}

/// Read one file with its loader's lexer mode and field dispatch. [`Top::File`] selects
/// a database file loop; [`Top::Persistent`] selects a block loop. Trigger readers
/// supply names from their load point; `None` makes a content key explicitly unknown.
pub fn read<S: Copy>(
    bytes: &[u8],
    mode: Mode,
    top: Top<S>,
    names: Option<&KeyNames>,
    dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
) -> Read {
    read_scoped(bytes, mode, top, names, None, dispatch)
}

/// Effect scope supplied by direct native effect dispatch callers.
pub(crate) fn read_in_effect_scope<S: Copy>(
    bytes: &[u8],
    mode: Mode,
    top: Top<S>,
    names: Option<&KeyNames>,
    scope: u32,
    dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
) -> Read {
    read_scoped(bytes, mode, top, names, Some(scope), dispatch)
}

fn read_scoped<S: Copy>(
    bytes: &[u8],
    mode: Mode,
    top: Top<S>,
    names: Option<&KeyNames>,
    scope: Option<u32>,
    dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
) -> Read {
    let mut reader = Reader::new(lex(bytes, mode));
    reader.names = names;
    reader.effect_scope = scope;
    let items = match top {
        Top::File(node) => reader.file(node, dispatch),
        Top::Persistent(node) => {
            let items = reader.object(node, dispatch);
            reader.item_misreads.resize_with(items.len(), Vec::new);
            items
        }
    };
    // A refused file must still name counted warnings encountered before
    // its stopping point; callers that return only the refusal keep them.
    if let Some(cause) = &mut reader.unsettled {
        for note in reader.misreads.iter().filter(|note| note.count.is_some()) {
            cause.what.push_str(&format!("; {note}"));
        }
    }
    Read {
        items,
        misreads: reader.misreads,
        item_misreads: reader.item_misreads,
        unattributed_misreads: reader
            .unattributed_misreads
            .into_iter()
            .chain(reader.reading_notes)
            .collect(),
        unsettled: reader.unsettled,
    }
}

/// Shared reader for GUI, sprite, font, effect and settings consumers that walk tokens
/// directly rather than dispatching [`Shape`]. It uses the same lexer, slots, variable
/// table, loop steps and skips as [`read`].
impl<'n> Reader<'n> {
    /// Construct a reader over a file's tokens with an empty variable table.
    pub fn new(lexed: Lexed<'n>) -> Self {
        Self {
            ends: (lexed.tokens.iter().enumerate())
                .filter_map(|(at, token)| (token.kind == Kind::End).then_some(at))
                .collect(),
            tokens: lexed.tokens,
            source: lexed.source,
            next: 0,
            current: Token::end(0),
            pending: false,
            pending_result: false,
            // Key and value slots start as token 0 with empty text.
            key: Token::slot(),
            operator: Operator::Equals,
            value: Token::slot(),
            variables: Vec::new(),
            misreads: Vec::new(),
            reading_notes: Vec::new(),
            item_misreads: Vec::new(),
            unattributed_misreads: Vec::new(),
            unsettled: None,
            broken: false,
            names: None,
            stopped: false,
            effect_scope: None,
        }
    }

    fn line(&self) -> u32 {
        if self.current.line != 0 {
            return self.current.line;
        }
        self.tokens
            .get(self.next)
            .or(self.tokens.last())
            .map_or(0, |token| token.line)
    }

    /// A native subreader's diagnostic, without adding a skip.
    pub(crate) fn note_native_error(&mut self, line: u32, what: String) {
        self.misread(line, what);
    }

    /// A native subreader whose next token consumption cannot be settled.
    pub(crate) fn refuse_native(&mut self, line: u32, what: String) {
        self.unsettled = Some(Misread {
            kind: crate::DiagnosticKind::Unsettled,
            count: None,
            line,
            what,
        });
    }

    fn misread(&mut self, line: u32, what: String) {
        self.misread_at(line, what, false);
    }

    /// Definitions update the file's shared variable table, including inside nested
    /// objects. All later references use that table, so these notes are file-scoped.
    fn variable_misread(&mut self, line: u32, what: String) {
        self.misread_at(line, what, true);
    }

    fn misread_at(&mut self, line: u32, what: String, file_scoped: bool) {
        let note = Misread {
            kind: crate::DiagnosticKind::Misread,
            count: None,
            line,
            what,
        };
        let notes = if file_scoped {
            &mut self.unattributed_misreads
        } else {
            &mut self.reading_notes
        };
        notes.push(note.clone());
        self.misreads.push(note);
    }

    /// Advance the lexer: the pending peek's result, else the
    /// next token. `false` past the end and for a read that returned 0 (a
    /// [`Kind::End`] token): the token is left as it was, save the text of a
    /// bare token that read ended, which becomes the lexer's token.
    pub fn advance(&mut self) -> bool {
        if self.pending {
            self.pending = false;
            return self.pending_result;
        }
        let Some(token) = self.tokens.get(self.next) else {
            return false;
        };
        self.next += 1;
        if token.kind != Kind::End {
            self.current = token.clone();
            return true;
        }
        if !token.text.is_empty() {
            // A comment marker (token 9) or a bare token's text (token 0):
            // no bare token is a lone `!`, `#` or `;`, which end one.
            let kind = match token.text.as_slice() {
                b"!" | b"#" | b";" => Kind::Comment,
                _ => Kind::Bare,
            };
            self.current = Token {
                kind,
                ..token.clone()
            };
        }
        false
    }

    /// Peek advances only when none is pending and keeps the result. A failed read
    /// changes the token to End while retaining text. Loops stop on `}` or the end.
    pub fn peek(&mut self) -> &Token {
        if !self.pending {
            let result = self.advance();
            if !result {
                self.current.kind = Kind::End;
            }
            self.pending = true;
            self.pending_result = result;
        }
        &self.current
    }

    fn current(&self) -> Token {
        self.current.clone()
    }

    /// A quoted token stays literal. A bare `@name` takes its defining token, or
    /// retains its own text if unknown.
    pub fn resolve(&self, token: Token) -> Token {
        if token.quoted || token.kind == Kind::Code || !token.text.starts_with(b"@") {
            return token;
        }
        let hash = super::variable_hash(&token.text[1..]);
        match self.variables.iter().find(|(defined, _)| *defined == hash) {
            Some((_, defined)) => Token {
                line: token.line,
                ..defined.clone()
            },
            None => token,
        }
    }

    /// Read a definition while the lexer stands on `@name`.
    fn define(&mut self) {
        let name = self.current();
        let hash = super::variable_hash(&name.text[1..]);
        if self.variables.iter().any(|(defined, _)| *defined == hash) {
            // "Variable name %s is already taken": nothing more is read.
            self.variable_misread(
                name.line,
                format!(
                    "variable {} is already taken, so its `=` and value are read as what follows",
                    name.shown()
                ),
            );
            return;
        }
        if !self.advance() {
            return;
        }
        if self.current().kind != Kind::Equals {
            let token = self.current();
            self.variable_misread(
                name.line,
                format!(
                    "definition {} has no `=`: the reader consumes {} and defines nothing",
                    name.shown(),
                    token.shown()
                ),
            );
            return;
        }
        // The peeked token's text decides, whatever its kind (a failed
        // peek's end token keeps the text).
        let peeked = self.peek().clone();
        let value = match peeked {
            // Resolved where it stands, without consuming it: the next key
            // read takes it again.
            token if token.text.starts_with(b"@") => {
                let resolved = self.resolve(token.clone());
                self.variable_misread(
                    name.line,
                    format!(
                        "definition {} takes {}, which the variable reader leaves unread, so the next key read starts from it",
                        name.shown(),
                        token.shown()
                    ),
                );
                resolved
            }
            _ => {
                self.advance();
                let token = self.current();
                if token.kind == Kind::Open {
                    self.variable_misread(
                        name.line,
                        format!(
                            "definition {} keeps only the `{{` of its block value; the block's contents are read as what follows",
                            name.shown()
                        ),
                    );
                }
                token
            }
        };
        self.variables.push((hash, value));
    }

    /// Read a key, operator and value into the slots.
    pub fn key_operator_value(&mut self) -> Slots {
        loop {
            if !self.advance() {
                // Only definitions were left: the loop ends with no key read
                // and every slot as it was.
                return Slots::Ended;
            }
            if self.current().text.starts_with(b"@") {
                self.define();
                continue;
            }
            self.key = self.current();
            if !self.advance() || !self.current().operator() {
                return Slots::NoOperator;
            }
            self.operator = self.current().as_operator();
            let peeked = self.peek();
            let value = if peeked.kind == Kind::Close {
                // A `}` value is left for the loop: the value is empty.
                Token::end(peeked.line)
            } else {
                // Any other, the end a failed peek left included, is taken
                // with its text.
                self.advance();
                self.resolve(self.current())
            };
            self.value = value;
            return Slots::Fresh;
        }
    }

    /// A file's top level: fields until the end of the text only.
    fn file<S: Copy>(
        &mut self,
        top: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while !self.stopped && self.unsettled.is_none() && self.peek().kind != Kind::End {
            let item = self.field(top, dispatch);
            let notes = std::mem::take(&mut self.reading_notes);
            if let Some(item) = item {
                items.push(item);
                self.item_misreads.push(notes);
            } else {
                self.unattributed_misreads.extend(notes);
            }
            self.broken = false;
        }
        items
    }

    /// Read fields until `}` or the end, consuming the closing read. Any retained `@`
    /// token resolves and is written back, so the enclosing reader continues after
    /// 0xFF.
    fn object<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            if self.at_loop_end() {
                break;
            }
            if let Some(item) = self.field(schema, dispatch) {
                items.push(item);
            }
            if std::mem::take(&mut self.broken) {
                let line = self.line();
                if self.advance() {
                    let consumed = self.current();
                    self.misread(
                        line,
                        format!(
                            "the loop ends here, and its closing read consumes {}",
                            consumed.shown()
                        ),
                    );
                }
                break;
            }
        }
        items
    }

    /// Peek before each field. A `}` or End closes the loop and is consumed; any
    /// retained `@` token resolves and is written back.
    fn at_loop_end(&mut self) -> bool {
        if !self.peek().ends_block() {
            return false;
        }
        self.advance();
        self.current = self.resolve(self.current());
        true
    }

    /// Take one block-loop step for a consumer with its own dispatch. Return `false` at
    /// `}` or End ([`Self::at_loop_end`]); otherwise fill the key/operator/value slots.
    pub fn next_key(&mut self) -> bool {
        if self.at_loop_end() {
            return false;
        }
        self.key_operator_value();
        true
    }

    /// Whether every read from here returns 0: a peeked token that read returned 0, or none but
    /// the ends left after it.
    pub fn at_end(&self) -> bool {
        if self.pending && self.pending_result {
            return false;
        }
        self.tokens[self.next.min(self.tokens.len())..]
            .iter()
            .all(|token| token.kind == Kind::End)
    }

    /// When the value slot holds `{`, skip to its closing literal brace without
    /// resolving tokens.
    pub fn skip_current_block(&mut self) {
        if self.value.kind == Kind::Open {
            self.skip_block();
        }
    }

    /// A reported key skips a block only when the current lexer token is `{`, resolving
    /// tokens during that skip.
    pub fn skip_reported(&mut self) {
        if self.current.kind == Kind::Open {
            self.skip();
        }
    }

    /// One key/operator/value read and its field dispatch.
    fn field<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Option<Item> {
        let slots = self.key_operator_value();
        if self.unsettled.is_some() {
            return None;
        }
        let fresh = slots == Slots::Fresh;
        let key = self.key.clone();
        match slots {
            Slots::Fresh => {}
            // The key and the token after it were consumed; the operator and
            // value slots keep what the last read left.
            Slots::NoOperator => {
                let consumed = self.current();
                self.misread(
                    key.line,
                    format!(
                        "key {} has no operator: the reader consumes {} as its operator and hands the key the previous value {}",
                        key.shown(),
                        consumed.shown(),
                        self.value.shown()
                    ),
                );
            }
            // Block and file loops dispatch the key slot regardless of what the field
            // read consumed. A definition at EOF leaves the previous slots, or the
            // initial empty key when none was read.
            Slots::Ended => {
                let line = self.line();
                self.misread(
                    line,
                    format!(
                        "the text ends after a definition, so no key is read and the reader takes the key slot {} again, with the value {}",
                        key.shown(),
                        self.value.shown()
                    ),
                );
            }
        }
        let value = self.value.clone();
        let current = self.current();
        let shape = dispatch(
            schema,
            &Field {
                key: &key,
                value: &value,
                fresh,
                operator: self.operator,
                current: &current,
                effect_scope: self.effect_scope,
            },
        );
        let (shape, stored_key) = match shape {
            Shape::ObjectAs(schema, key) => (Shape::Object(schema), key),
            Shape::Discard => {
                self.skip_reported();
                return None;
            }
            shape => (shape, key.text.clone()),
        };
        // Capture this field's operator before reading its block: fields inside
        // overwrite the same slot with their own `<`, `>` or `=`.
        let operator = self.operator;
        // Take the source token from the current lexer token for current-token readers,
        // otherwise from the value slot, preserving its kind even after a failed
        // advance leaves End.
        let kind = match shape {
            Shape::Current | Shape::Color => current.kind,
            _ => value.kind,
        };
        let value = self.apply(shape, &key, &value, dispatch);
        Some(Item {
            key: Some(stored_key),
            key_quoted: key.kind == Kind::Quoted,
            operator,
            value,
            kind,
        })
    }

    fn apply<S: Copy>(
        &mut self,
        shape: Shape<S>,
        key: &Token,
        value: &Token,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Value {
        let opens = value.kind == Kind::Open;
        match shape {
            Shape::ObjectAs(..) | Shape::Discard => unreachable!("handled by field"),
            Shape::One | Shape::Current => {
                if opens {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is a block where its reader takes one value: the reader reads `{{` and the block's contents are read as the enclosing fields, its `}}` ending the enclosing block",
                            key.shown()
                        ),
                    );
                }
                Value::Scalar(if matches!(shape, Shape::Current) {
                    self.current.text.clone()
                } else {
                    value.text.clone()
                })
            }
            Shape::ObjectOrSkip(schema) if opens => Value::Block(self.object(schema, dispatch)),
            Shape::ListOrSkip if opens => Value::Block(self.list()),
            Shape::ObjectOrSkip(_) | Shape::ListOrSkip => {
                self.misread(
                    key.line,
                    format!(
                        "field {} is {} where its reader reads a block: it is reported",
                        key.shown(),
                        value.shown()
                    ),
                );
                self.apply(Shape::Skip, key, value, dispatch)
            }
            Shape::Pair(schema) => self.pair(schema, key, value, dispatch),
            Shape::Strings if opens => Value::Block(self.strings(key)),
            Shape::Strings => {
                self.misread(
                    key.line,
                    format!(
                        "field {} is {} where its reader reads `{{ string string }}`: it is reported",
                        key.shown(),
                        value.shown()
                    ),
                );
                self.apply(Shape::Skip, key, value, dispatch)
            }
            Shape::Object(schema) | Shape::ObjectIfOpen(schema) | Shape::Twice(schema)
                if opens || matches!(shape, Shape::Object(_) | Shape::Twice(_)) =>
            {
                if !opens {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is {} where its reader reads a block: it reads the tokens after it as its fields, up to the next `}}`",
                            key.shown(),
                            value.shown()
                        ),
                    );
                }
                let mut items = self.object(schema, dispatch);
                if let Shape::Twice(schema) = shape {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is read twice: the second read takes the tokens after its block as its fields",
                            key.shown()
                        ),
                    );
                    items.extend(self.object(schema, dispatch));
                }
                Value::Block(items)
            }
            Shape::ObjectIfOpen(_) => Value::Scalar(value.text.clone()),
            Shape::Object(_) | Shape::Twice(_) => unreachable!("handled above"),
            Shape::ListIfOpen if !opens => Value::Scalar(value.text.clone()),
            Shape::List | Shape::ListIfOpen => {
                if !opens {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is {} where its reader reads a list: it reads the tokens after it as list entries, up to the next `}}`",
                            key.shown(),
                            value.shown()
                        ),
                    );
                }
                Value::Block(self.list())
            }
            Shape::BoundedList(n) => Value::Block(self.bounded_list(n)),
            Shape::Tokens => {
                if !opens {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is {} where its reader reads a block: it reads the tokens after it, braces counted, up to the next unmatched `}}`",
                            key.shown(),
                            value.shown()
                        ),
                    );
                }
                Value::Block(self.tokens())
            }
            Shape::SkipIfOpen => {
                if opens {
                    self.skip_current_block();
                    Value::Block(Vec::new())
                } else {
                    Value::Scalar(value.text.clone())
                }
            }
            Shape::Skip => {
                if self.current.kind == Kind::Open {
                    self.skip_reported();
                    Value::Block(Vec::new())
                } else {
                    Value::Scalar(value.text.clone())
                }
            }
            Shape::Body(schema) if opens => Value::Block(self.object(schema, dispatch)),
            Shape::Trigger | Shape::MtthModifier if opens => {
                let mut shaper = crate::combat_tactics::Shaper::new(self.names);
                let node = if matches!(shape, Shape::MtthModifier) {
                    crate::combat_tactics::Node::Modifier
                } else {
                    crate::combat_tactics::Node::Trigger
                };
                Value::Block(self.object(node, &mut |node, field: &Field<'_>| {
                    shaper.shape(node, field)
                }))
            }
            Shape::Body(_) | Shape::Trigger | Shape::MtthModifier => {
                self.misread(
                    key.line,
                    format!(
                        "field {} is {} where its trigger reads a block: the missing block is reported and nothing is read",
                        key.shown(),
                        value.shown()
                    ),
                );
                self.apply(Shape::Skip, key, value, dispatch)
            }
            Shape::Effect => self.effect_body(key, value),
            Shape::EffectIn(scope) => self.effect_body_in(key, value, Some(scope)),
            Shape::EffectValueKeys(address) => {
                let mut shaper =
                    crate::effect_keys::value_readers::Shaper::new(address, self.names, key);
                Value::Block(self.object(
                    crate::effect_keys::value_readers::Node::Root,
                    &mut |node, field| shaper.shape(node, field),
                ))
            }
            Shape::EffectKeys(token)
                if opens || !crate::effect_keys::key_readers::requires_block(token) =>
            {
                let mut shaper = crate::effect_keys::key_readers::Shaper::new(self.names, token);
                let saved = self.effect_scope;
                self.effect_scope =
                    crate::effect_keys::value_readers::read_script_scope(token, saved);
                let body = Value::Block(self.object(
                    crate::effect_keys::key_readers::Node::Keys,
                    &mut |node, field| shaper.shape(node, field),
                ));
                self.effect_scope = saved;
                body
            }
            Shape::EffectKeys(_) => self.effect_body(key, value),
            Shape::EffectByKey { trigger_limits } => {
                if !opens {
                    self.effect_body(key, value)
                } else if self.current.kind == Kind::Open {
                    Value::Block(self.generic_context(false, trigger_limits))
                } else {
                    self.apply(Shape::ByKey, key, value, dispatch)
                }
            }
            Shape::Reported { kind, report, read } => {
                for notes in [&mut self.reading_notes, &mut self.misreads] {
                    if let Some(previous) = notes.iter_mut().find(|note| {
                        note.count.is_some() && note.what == report && note.kind == kind
                    }) {
                        *previous.count.as_mut().unwrap() += 1;
                    } else {
                        notes.push(Misread {
                            line: key.line,
                            what: report.clone(),
                            kind,
                            count: Some(1),
                        });
                    }
                }
                self.apply(*read, key, value, dispatch)
            }
            Shape::ByKey => {
                if opens && self.current.kind != Kind::Open {
                    // A key without an operator receives the stale `{`; its dispatch
                    // decides what to consume.
                    self.unsettled = Some(Misread {
                        kind: crate::DiagnosticKind::Unsettled,
                        count: None,
                        line: key.line,
                        what: format!(
                            "key {} has no operator and its reader is handed the previous value `{{`: a body reader reads the tokens after {} up to the enclosing `}}`, a one-value reader or an invalid key reads nothing more, and the field reader is unknown",
                            key.shown(),
                            self.current.shown()
                        ),
                    });
                    Value::Block(Vec::new())
                } else if opens {
                    // `meta_effect` and `meta_trigger` copy their `text` block raw.
                    let meta = super::looked_up(&key.text, key.kind == Kind::Quoted)
                        && matches!(
                            key.text.to_ascii_lowercase().as_slice(),
                            b"meta_effect" | b"meta_trigger"
                        );
                    Value::Block(self.generic(meta))
                } else {
                    Value::Scalar(value.text.clone())
                }
            }
            Shape::Statements(node) => Value::Block(self.statements(node, dispatch)),
            Shape::RawList => Value::Block(self.raw_list(false, false, false)),
            Shape::RawListFirst => Value::Block(self.raw_list_first()),
            Shape::TemplatePoint => crate::effect_keys::value_readers::template_point(self),
            Shape::ValueList => Value::Block(self.raw_list(false, true, false)),
            Shape::ObjectList(schema) => Value::Block(self.object_list(schema, dispatch)),
            Shape::CheckedList { stores_value } => {
                Value::Block(self.raw_list(false, stores_value, true))
            }
            Shape::RawSkip => {
                self.skip_block();
                Value::Block(Vec::new())
            }
            Shape::RawText => Value::Scalar(self.raw_text()),
            Shape::Expression => self.math_expression(),
            Shape::LimitedBody(schema) => Value::Block(self.limited_body(schema, key, dispatch)),
            Shape::ScriptedKey if opens => Value::Block(self.scripted_key()),
            Shape::ScriptedKey => Value::Scalar(value.text.clone()),
            Shape::TechnologyEquipment => Value::Block(self.raw_list(true, false, false)),
            Shape::Color => self.color(key),
            Shape::StopFile => {
                self.misread(
                    key.line,
                    format!(
                        "unexpected top-level key {}; the file reader stops here",
                        key.shown(),
                    ),
                );
                if self.current.kind == Kind::Open {
                    self.skip_reported();
                }
                self.stopped = true;
                Value::Scalar(value.text.clone())
            }
            Shape::SkipBreak => {
                if self.current.kind == Kind::Open {
                    self.skip();
                }
                self.broken = true;
                Value::Scalar(value.text.clone())
            }
            Shape::Unsettled(why) => {
                self.unsettled = Some(Misread {
                    kind: crate::DiagnosticKind::Unsettled,
                    count: None,
                    line: key.line,
                    what: format!("field {}: {why}", key.shown()),
                });
                Value::Block(Vec::new())
            }
            Shape::Break => {
                self.misread(
                    key.line,
                    format!(
                        "field {} ends the loop reading it, its value {} read and nothing more",
                        key.shown(),
                        value.shown()
                    ),
                );
                self.broken = true;
                Value::Scalar(value.text.clone())
            }
        }
    }

    fn statements<S: Copy>(
        &mut self,
        node: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            if matches!(self.peek().kind, Kind::Close | Kind::End) {
                self.advance();
                break;
            }
            if !self.advance() {
                break;
            }
            let first = self.resolve(self.current());
            self.current = first.clone();
            let assigned = first.kind != Kind::Open && self.peek().operator();
            let (key, value) = if assigned {
                self.key = first.clone();
                self.advance();
                self.operator = self.resolve(self.current()).as_operator();
                self.advance();
                let value = self.resolve(self.current());
                self.current = value.clone();
                (first, value)
            } else {
                self.key = Token::slot();
                self.operator = Operator::Equals;
                (self.key.clone(), first)
            };
            self.value = value.clone();
            let operator = self.operator;
            let shape = dispatch(
                node,
                &Field {
                    key: &key,
                    value: &value,
                    current: &self.current,
                    effect_scope: self.effect_scope,
                    fresh: assigned,
                    operator,
                },
            );
            let contents = self.apply(shape, &key, &value, dispatch);
            items.push(Item {
                key: assigned.then_some(key.text),
                key_quoted: key.kind == Kind::Quoted,
                operator,
                kind: value.kind,
                value: contents,
            });
        }
        items
    }

    fn raw_text(&mut self) -> Vec<u8> {
        let Some(source) = self.source.as_mut() else {
            self.unsettled = Some(Misread {
                kind: crate::DiagnosticKind::Unsettled,
                count: None,
                line: self.key.line,
                what: "raw meta text cannot be read: token stream has no byte source".into(),
            });
            return Vec::new();
        };
        let mut at = self
            .next
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map(|token| token.byte_end)
            .unwrap_or(0);
        let start = at;
        let mut depth = 1_usize;
        while let Some(byte) = source.bytes.get(at) {
            if *byte == 0 {
                self.unsettled = Some(Misread {
                    kind: crate::DiagnosticKind::Unsettled,
                    count: None,
                    line: self.key.line,
                    what: "raw meta text cannot be read: embedded NUL".into(),
                });
                return Vec::new();
            }
            at += 1;
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        let mut parameter = false;
        let mut valid = depth == 0;
        for byte in &source.bytes[start..at.saturating_sub(usize::from(depth == 0))] {
            match byte {
                b'[' if parameter => {
                    valid = false;
                    break;
                }
                b'[' => parameter = true,
                b']' if !parameter => {
                    valid = false;
                    break;
                }
                b']' => parameter = false,
                _ => {}
            }
        }
        valid &= !parameter;
        let text = source.bytes[start..at.saturating_sub(usize::from(depth == 0))].to_vec();
        // A raw read can end inside a quoted token or a comment. Resume
        // the same lexer at that byte; no token or reader slot was read.
        let mut tail = lex_with(&source.bytes[at..], Mode::KeepMark, source.code);
        let line_offset = source.bytes[..at]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u32;
        for token in &mut tail.tokens {
            token.line += line_offset;
            token.byte_end += at;
        }
        self.tokens.truncate(self.next);
        self.tokens.extend(tail.tokens);
        self.ends = self
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(i, token)| (token.kind == Kind::End).then_some(i))
            .collect();
        if !valid {
            // The raw read leaves the lexer's token intact; reporting tests that token
            // for `{`.
            self.misread(
                self.key.line,
                "raw meta text has unmatched braces or brackets and is reported".into(),
            );
            self.skip_reported();
        }
        text
    }

    /// Copy the raw entry into the value slot before reading its child object. Both
    /// closing reads leave that slot intact.
    fn object_list<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            let advanced = self.advance();
            self.current = self.resolve(self.current());
            if stop || !advanced {
                break;
            }
            let token = self.current();
            self.value = token.clone();
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Block(self.object(schema, dispatch)),
            });
        }
        items
    }

    // Read sub-unit types with an unconditional first raw entry.
    fn raw_list_first(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            self.advance();
            self.current = self.resolve(self.current());
            let token = self.current();
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text),
            });
            if self.peek().ends_block() {
                self.advance();
                self.current = self.resolve(self.current());
                return items;
            }
        }
    }

    fn raw_list(&mut self, equipment: bool, stores_value: bool, skip_open: bool) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            let advanced = self.advance();
            self.current = self.resolve(self.current());
            if stop || !advanced {
                break;
            }
            let mut token = self.current();
            if stores_value {
                self.value = token.clone();
            }
            if skip_open && token.kind == Kind::Open {
                self.skip_reported();
                continue;
            }
            if equipment && token.kind == Kind::Bare && token.text.eq_ignore_ascii_case(b"limit") {
                self.advance();
                self.current = self.resolve(self.current());
                self.advance();
                self.current = self.resolve(self.current());
                let value = self.current();
                if value.kind != Kind::Open {
                    // Trigger-body admission tests the value slot, which this loop
                    // leaves at the list's `{`. It would read the following tokens as
                    // trigger fields; that consumption is not followed.
                    self.unsettled = Some(Misread {
                        kind: crate::DiagnosticKind::Unsettled,
                        count: None,
                        line: token.line,
                        what: format!(
                            "technology enable_equipments.limit: unsupported value {} (the trigger body reader reads the list's retained `{{`)",
                            value.shown()
                        ),
                    });
                    break;
                }
                let body = self.apply(Shape::Trigger, &token, &value, &mut |_: bool, field| {
                    untraced(field)
                });
                items.push(Item {
                    key: Some(token.text.clone()),
                    key_quoted: false,
                    operator: Operator::Equals,
                    kind: value.kind,
                    value: body,
                });
                self.advance();
                self.current = self.resolve(self.current());
                token = self.current();
                // Both evaluated branches consume the same delimiter only
                // for this tail. Keep the conditional link unavailable.
                if matches!(token.kind, Kind::Open | Kind::Close | Kind::End)
                    || self.peek().kind != Kind::Close
                {
                    self.unsettled = Some(Misread { kind: crate::DiagnosticKind::Unsettled, count: None, line: token.line, what: "technology enable_equipments.limit: unsupported conditional tail (requires one equipment token)".into() });
                    break;
                }
            }
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text),
            });
        }
        items
    }

    /// Read an effect script using the default effect-key dispatch.
    fn effect_body(&mut self, key: &Token, value: &Token) -> Value {
        let scope =
            crate::effect_keys::value_readers::body_scope(self.names, key, self.effect_scope);
        self.effect_body_in(key, value, scope)
    }

    fn effect_body_in(&mut self, key: &Token, value: &Token, scope: Option<u32>) -> Value {
        let saved = self.effect_scope;
        self.effect_scope = scope;
        let body = if value.kind == Kind::Open {
            let names = self.names;
            Value::Block(self.object((), &mut |(), field| crate::effect_keys::shape(names, field)))
        } else {
            self.misread(
                key.line,
                crate::effect_keys::body_error(&key.shown(), &value.shown()),
            );
            self.apply(Shape::Skip, key, value, &mut |(): (), _| Shape::Skip)
        };
        self.effect_scope = saved;
        body
    }

    /// [`Shape::LimitedBody`].
    fn limited_body<S: Copy>(
        &mut self,
        schema: S,
        key: &Token,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let next = self.peek();
        if next.kind == Kind::Bare && next.text.eq_ignore_ascii_case(b"limit") {
            items.extend(self.field(schema, &mut |_, _| Shape::Trigger));
        }
        if self.value.kind == Kind::Open {
            items.extend(self.object(schema, dispatch));
        } else if self.current.kind == Kind::Open {
            // Reporting skips a block at the current token, here the peeked token; that
            // recovery is not followed.
            self.unsettled = Some(Misread {
                kind: crate::DiagnosticKind::Unsettled,
                count: None,
                line: key.line,
                what: format!(
                    "field {} has the value {} and a block after it: the effect's recovery skip over the peeked `{{` is not followed; native effect recovery is part of the named effect-key validity follow-up",
                    key.shown(),
                    self.value.shown()
                ),
            });
        } else {
            self.misread(
                key.line,
                format!(
                    "field {} has the value {} where its effect reads a block: it is reported and nothing more is read",
                    key.shown(),
                    self.value.shown()
                ),
            );
        }
        items
    }

    fn scripted_key(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            if matches!(self.peek().kind, Kind::Close | Kind::End) {
                self.advance();
                break;
            }
            if !self.advance() {
                break;
            }
            let key = self.resolve(self.current());
            self.current = key.clone();
            if key.kind == Kind::Open || !self.peek().operator() {
                // Case entries reject anonymous blocks and values. Their callback
                // receives cleared key and operator slots.
                self.key = Token::slot();
                self.operator = Operator::Equals;
                self.value = key.clone();
                if self.current.kind == Kind::Open {
                    self.skip();
                }
                continue;
            }
            self.key = key.clone();
            self.advance();
            let operator = self.resolve(self.current()).as_operator();
            self.operator = operator;
            self.advance();
            let value = self.resolve(self.current());
            self.current = value.clone();
            self.value = value.clone();
            // Read an exact boolean token; false or failed reads instead construct and
            // read an AND trigger.
            let shape = if super::read_bool(&value.text) == Some(true) {
                Shape::One
            } else {
                Shape::Trigger
            };
            let contents = self.apply(shape, &key, &value, &mut |_: bool, field| untraced(field));
            items.push(Item {
                key: Some(key.text),
                key_quoted: key.kind == Kind::Quoted,
                operator,
                kind: value.kind,
                value: contents,
            });
        }
        items
    }

    /// [`Shape::Pair`].
    fn pair<S: Copy>(
        &mut self,
        schema: S,
        key: &Token,
        value: &Token,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Value {
        if value.kind != Kind::Open {
            self.misread(
                key.line,
                format!(
                    "field {} is {} where its reader reads `{{ key = value }}`: it is reported",
                    key.shown(),
                    value.shown()
                ),
            );
            return self.apply(Shape::Skip, key, value, dispatch);
        }
        let items = self.field(schema, dispatch).into_iter().collect();
        if !matches!(self.peek().kind, Kind::Close | Kind::End) {
            self.misread(
                key.line,
                format!(
                    "field {} holds more than one entry: the reader reports it and skips the rest",
                    key.shown()
                ),
            );
            if self.current.kind == Kind::Open {
                self.skip();
            }
        }
        self.skip_block();
        Value::Block(items)
    }

    /// [`Shape::Strings`] after its `{`: the strings read, each a value
    /// without a key.
    fn strings(&mut self, key: &Token) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            if matches!(self.peek().kind, Kind::Close | Kind::End) {
                if items.len() != 2 {
                    self.misread(
                        key.line,
                        format!(
                            "field {} holds {} string(s) where its reader reads two: it is reported",
                            key.shown(),
                            items.len()
                        ),
                    );
                }
                self.advance();
                return items;
            }
            self.advance();
            let token = self.resolve(self.current());
            self.value = token.clone();
            if items.len() == 2 {
                self.misread(
                    key.line,
                    format!(
                        "field {} holds {} after its two strings: it is reported and skipped to the pair's `}}`",
                        key.shown(),
                        token.shown()
                    ),
                );
                if self.current.kind == Kind::Open {
                    self.skip();
                }
                self.skip_block();
                return items;
            }
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text),
            });
        }
    }

    /// Advance with literal brace counting to the block's closing `}` or EOF, resolving
    /// and defining nothing.
    pub fn skip_block(&mut self) {
        let mut depth = 0_u32;
        while self.advance() {
            match self.current.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return,
                Kind::Close => depth -= 1,
                _ => {}
            }
        }
    }

    /// [`Shape::Color`]: its components as values without keys.
    fn color(&mut self, key: &Token) -> Value {
        let current = self.current();
        if current.kind == Kind::Bare && super::number_token(&current.text, false) {
            return Value::Scalar(current.text);
        }
        if current.kind != Kind::Open {
            self.misread(
                key.line,
                format!(
                    "field {} is {} where the colour reader reads a number or `{{`: it is reported and nothing more is read",
                    key.shown(),
                    current.shown()
                ),
            );
            return Value::Scalar(current.text);
        }
        let mut components = Vec::new();
        let next = |reader: &mut Self| {
            reader.advance();
            let token = reader.resolve(reader.current());
            reader.current = token.clone();
            token
        };
        for index in 0..5 {
            let token = next(self);
            if index == 3 && token.kind == Kind::Close {
                return Value::Block(components);
            }
            if index == 4 {
                if token.kind != Kind::Close {
                    self.misread(
                        key.line,
                        format!(
                            "field {} is a colour that does not close after four components: the colour reader reads {} as its end",
                            key.shown(),
                            token.shown()
                        ),
                    );
                }
                return Value::Block(components);
            }
            if !(token.kind == Kind::Bare && super::number_token(&token.text, false)) {
                self.misread(
                    key.line,
                    format!(
                        "field {} has {} where the colour reader reads a number: it is reported and read as a component",
                        key.shown(),
                        token.shown()
                    ),
                );
            }
            components.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text),
            });
        }
        Value::Block(components)
    }

    /// Read tokens until `}` or End and consume that closing token. Resolve every entry
    /// into the value slot.
    pub fn list(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            if stop {
                return items;
            }
            let token = self.resolve(self.current());
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text.clone()),
            });
            self.value = token;
        }
    }

    fn bounded_list(&mut self, n: usize) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            if stop {
                return items;
            }
            // Write the resolved token back to the lexer and value slot before
            // reporting.
            let token = self.resolve(self.current());
            self.current = token.clone();
            self.value = token.clone();
            if items.len() == n && self.current.kind == Kind::Open {
                self.skip();
            }
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text),
            });
        }
    }

    /// [`Shape::Tokens`] peeks for EOF first, then stops at depth zero; otherwise it
    /// takes the next token.
    fn tokens(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        let mut depth = 1;
        while self.peek().kind != Kind::End && depth >= 1 {
            self.advance();
            let token = self.resolve(self.current());
            match token.kind {
                Kind::Open => depth += 1,
                Kind::Close => depth -= 1,
                _ => items.push(Item {
                    key: None,
                    key_quoted: false,
                    operator: Operator::Equals,
                    kind: token.kind,
                    value: Value::Scalar(token.text),
                }),
            }
        }
        items
    }

    /// With the lexer on `{`, skip tokens through a peeked `}` or End, recursively
    /// skipping nested blocks and defining nothing. Resolve each token including the
    /// closing token and write it back. A resolved `{`, including a reference retaining
    /// a brace, starts a nested skip. Peeks test the token before resolution.
    pub fn skip(&mut self) {
        loop {
            let end = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            self.current = self.resolve(self.current());
            if end {
                return;
            }
            if self.current.kind == Kind::Open {
                self.skip();
            }
        }
    }

    /// A block read by key that holds a value without a key, read as a
    /// list: [`Self::balanced`], or unsettled when it also holds a
    /// definition (see [`Self::generic`]). `None` when the block holds keys
    /// only.
    fn keyless_list(&mut self) -> Option<Vec<Item>> {
        let line = self.line();
        let (keyless, definition) = self.scan();
        if !keyless {
            return None;
        }
        if let Some(variable) = definition {
            self.unsettled = Some(Misread {
                kind: crate::DiagnosticKind::Unsettled,
                count: None,
                line,
                what: format!(
                    "variable {variable} is in a trigger or effect block that also holds anonymous values: a list reader does not define it, while a key reader consumes it out of step, and the field reader is unknown; effect keys remain read by key"
                ),
            });
            return Some(Vec::new());
        }
        Some(self.balanced())
    }

    /// Read an uninterpreted block by key: fresh `{` values recurse and other values
    /// take one token. Definitions register in the file's shared variable table.
    /// `meta_effect` and `meta_trigger` instead copy `text` raw without defining
    /// variables.
    ///
    /// Unknown dispatch cannot distinguish a list reader from a key reader. A key the
    /// game skips may be read by key here and register definitions, so this is lenient.
    /// A block containing an anonymous value reads as a list without definitions; if it
    /// also contains a definition, consumption is ambiguous and the remainder of the
    /// file is unsettled. Known trigger bodies use their own value/key dispatch
    /// ([`Shape::Trigger`]) against the content names.
    fn generic(&mut self, meta: bool) -> Vec<Item> {
        self.generic_context(meta, false)
    }

    fn generic_context(&mut self, meta: bool, trigger_limits: bool) -> Vec<Item> {
        if !meta && let Some(list) = self.keyless_list() {
            return list;
        }
        let mut dispatch = |meta: bool, field: &Field<'_>| -> Shape<bool> {
            if meta && field.fresh && field.keyword_is("text") {
                return Shape::Skip;
            }
            // Read the limit as an AND trigger using the enclosing reader's names.
            // Other conditional-effect keys remain a counted deferred gap.
            if trigger_limits && field.keyword_is("limit") {
                return Shape::Trigger;
            }
            untraced(field)
        };
        self.object(meta, &mut dispatch)
    }

    /// The first level of the block ahead (after its `{`): whether it holds
    /// a value without a key, and its first definition.
    fn scan(&self) -> (bool, Option<String>) {
        // A read that returns 0 is the end to the peeks.
        let start = self.next.min(self.tokens.len());
        let end = self
            .ends
            .get(self.ends.partition_point(|&at| at < start))
            .map_or(self.tokens.len(), |&at| at);
        let tokens = &self.tokens[start..end];
        let mut at = 0;
        let mut keyless = false;
        let mut definition = None;
        while let Some(token) = tokens.get(at) {
            match token.kind {
                Kind::Close => break,
                Kind::Open => {
                    keyless = true;
                    at = skip_block(tokens, at);
                    continue;
                }
                _ => {}
            }
            let operator = tokens.get(at + 1).is_some_and(Token::operator);
            if !operator {
                keyless = true;
                at += 1;
                continue;
            }
            if token.text.starts_with(b"@") && definition.is_none() {
                definition = Some(String::from_utf8_lossy(&token.text).into_owned());
            }
            at += 2;
            if tokens.get(at).is_some_and(|value| value.kind == Kind::Open) {
                at = skip_block(tokens, at);
            } else {
                at += 1;
            }
        }
        (keyless, definition)
    }

    /// A list-like block read to its matching `}`, kept as the tree reads
    /// it; references resolve, nothing defines.
    fn balanced(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            let token = self.peek().clone();
            self.advance();
            if token.kind == Kind::End {
                return items;
            }
            match token.kind {
                Kind::Close => return items,
                Kind::Open => items.push(Item {
                    key: None,
                    key_quoted: false,
                    operator: Operator::Equals,
                    kind: Kind::Open,
                    value: Value::Block(self.balanced()),
                }),
                _ => {
                    let token = self.resolve(token);
                    let operator = self.peek().operator();
                    if operator {
                        self.advance();
                        let operator = self.current().as_operator();
                        self.advance();
                        let value = self.resolve(self.current());
                        let kind = value.kind;
                        let value = if kind == Kind::Open {
                            Value::Block(self.balanced())
                        } else {
                            Value::Scalar(value.text)
                        };
                        items.push(Item {
                            key: Some(token.text),
                            key_quoted: token.kind == Kind::Quoted,
                            operator,
                            kind,
                            value,
                        });
                    } else {
                        items.push(Item {
                            key: None,
                            key_quoted: false,
                            operator: Operator::Equals,
                            kind: token.kind,
                            value: Value::Scalar(token.text.clone()),
                        });
                        self.value = token;
                    }
                }
            }
        }
    }
}

/// The index after the block opening at `at`.
fn skip_block(tokens: &[Token], mut at: usize) -> usize {
    let mut depth = 0_usize;
    while let Some(token) = tokens.get(at) {
        at += 1;
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    at
}

#[cfg(test)]
mod tests;
