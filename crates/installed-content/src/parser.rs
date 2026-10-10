//! Shared byte-preserving Clausewitz text model with two readers.
//!
//! Script content uses [`native`] to follow lexer and field-dispatch consumption. Key
//! dispatch, token kinds, `@name` variables, comparisons, EOF and text consumed out of
//! step with braces share that behavior.
//!
//! [`parse`] builds a balanced tree for saves and descriptor consumers. Save brace
//! mismatches are corruption and refuse the read. Both readers return [`Item`]s with
//! the same value model.
//!
//! Numbers remain lexical bytes until callers choose their representation. Quoted
//! strings decode escaped quotes and backslashes while preserving other escape
//! sequences, paths and formatting bytes without UTF-8 replacement or invented numeric
//! precision.

use std::io;

pub mod native;
pub use native::Kind;

/// Maximum number of nested ordinary or color blocks.
pub const MAX_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Scalar(Vec<u8>),
    Block(Vec<Item>),
    Tagged { tag: Vec<u8>, items: Vec<Item> },
}

impl Value {
    pub fn scalar(&self) -> Option<&[u8]> {
        match self {
            Self::Scalar(bytes) => Some(bytes),
            Self::Block(_) | Self::Tagged { .. } => None,
        }
    }

    /// Returns `None` for a block or a scalar that is not valid UTF-8.
    pub fn scalar_str(&self) -> Option<&str> {
        std::str::from_utf8(self.scalar()?).ok()
    }

    pub fn items(&self) -> Option<&[Item]> {
        match self {
            Self::Scalar(_) => None,
            Self::Block(items) | Self::Tagged { items, .. } => Some(items),
        }
    }

    /// Stored script strings take value-token text up to its first NUL. A line feed is
    /// reported and replaces the text with [`UNREADABLE_STRING`]. A block returns
    /// `None`; one-value script replay stores its opening brace as a scalar instead.
    pub fn read_string(&self) -> Option<&[u8]> {
        Some(read_string(self.scalar()?))
    }

    /// [`Self::read_string`] converted to text. Stored bytes are not filtered; invalid
    /// UTF-8 gives [`NotText::NotUtf8`] rather than absence, so readers refuse by name
    /// what they cannot hold.
    pub fn read_str(&self) -> Result<&str, NotText> {
        text(self.read_string().ok_or(NotText::Block)?)
    }

    /// Value-token text through its first NUL, without the stored-string line-feed
    /// check, converted as in [`Self::read_str`].
    pub fn c_str(&self) -> Result<&str, NotText> {
        text(c_string(self.scalar().ok_or(NotText::Block)?))
    }
}

/// Why a stored text is not the port's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotText {
    /// A block where the reader stores one value.
    Block,
    /// Bytes the game stores that are not UTF-8, which the port's text
    /// cannot hold.
    NotUtf8(Vec<u8>),
}

impl std::fmt::Display for NotText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block => f.write_str("a block where one value is stored"),
            Self::NotUtf8(bytes) => write!(
                f,
                "the stored text `{}` is not UTF-8, which the port's text cannot hold",
                bytes.escape_ascii()
            ),
        }
    }
}

/// The first stored text a reader's projection could not hold, named by
/// its field: the projection goes on without it, and the reader that owns
/// the definition refuses it (a file gap, [`crate::coverage`]) instead of
/// answering without the text or from an earlier one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unheld(Option<String>);

impl Unheld {
    /// The text, or `None` with the first failure kept.
    pub fn keep<'a>(&mut self, field: &str, text: Result<&'a str, NotText>) -> Option<&'a str> {
        text.inspect_err(|refused| {
            self.0
                .get_or_insert_with(|| format!("{field} is {refused}"));
        })
        .ok()
    }

    /// The first failure, if any.
    pub fn cause(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Keeps `other`'s failure unless this one has an earlier one.
    pub fn absorb(&mut self, other: &Self) {
        if self.0.is_none() {
            self.0.clone_from(&other.0);
        }
    }
}

/// Stored bytes as the port's text, or why they are not.
pub fn text(bytes: &[u8]) -> Result<&str, NotText> {
    std::str::from_utf8(bytes).map_err(|_| NotText::NotUtf8(bytes.to_vec()))
}

/// A token's text as a C string: up to its first NUL.
pub fn c_string(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}

/// Read exact C-string `yes` or `no`; failure stores nothing.
pub fn read_bool(text: &[u8]) -> Option<bool> {
    match c_string(text) {
        b"yes" => Some(true),
        b"no" => Some(false),
        _ => None,
    }
}

/// Apply boolean assignments in order; a failed conversion keeps the previous value.
pub fn read_bool_field(items: &[Item], key: &str, default: bool) -> bool {
    items
        .iter()
        .filter(|item| item.keyword_is(key))
        .fold(default, |state, item| {
            item.value.scalar().and_then(read_bool).unwrap_or(state)
        })
}

/// Stored replacement for a script string containing a line feed.
pub const UNREADABLE_STRING: &[u8] = b"Unreadable String";

/// Copy token text up to its first NUL. A line feed replaces the stored text with
/// [`UNREADABLE_STRING`].
pub fn read_string(text: &[u8]) -> &[u8] {
    let text = c_string(text);
    if text.contains(&b'\n') {
        UNREADABLE_STRING
    } else {
        text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// `None` represents an anonymous list element, block, or file header.
    pub key: Option<Vec<u8>>,
    /// The key is quoted, so it bypasses keyword lookup and cannot reach keyword
    /// dispatch. Ask [`Item::keyword`] rather than comparing key bytes.
    pub key_quoted: bool,
    /// The operator between a key and value. `<` and `>` end bare tokens and each form
    /// their own operator beside `=`, so `skill>@x` has a key, operator and value.
    /// Anonymous items use [`Operator::Equals`].
    pub operator: Operator,
    pub value: Value,
    /// The source value token's kind after variable resolution, or the current lexer
    /// token for [`native::Shape::Current`]. References inherit their definition's
    /// kind. [`Kind::End`] keeps the text from a failed advance or an empty value
    /// before `}`; that text cannot make it a keyword, number or quoted string. Use
    /// [`Item::quoted`], [`Item::value_keyword`] and [`Item::number`]. The tree
    /// ([`parse`]) supplies [`Kind::Bare`], [`Kind::Quoted`] or [`Kind::Open`] for
    /// blocks.
    pub kind: Kind,
}

/// Operators are single characters: `<=` is `<` followed by `=`, `!` starts a comment,
/// and `?=` is not a field operator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operator {
    /// `=`, token 1.
    #[default]
    Equals,
    /// `<`, token 0x1D4.
    Less,
    /// `>`, token 0x1D3.
    Greater,
}

impl Operator {
    /// The operator's one character.
    pub fn text(self) -> &'static [u8] {
        match self {
            Self::Equals => b"=",
            Self::Less => b"<",
            Self::Greater => b">",
        }
    }
}

/// A bare key eligible for keyword lookup uses an ASCII-lowercased copy. Dispatch
/// compares the resulting token identity without case; the registry alone determines
/// whether the key is a keyword, and each reader selects the keywords it handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyword<'a>(&'a [u8]);

/// A text token's name under the lexer's ASCII case fold.
pub fn fold_token(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl<'a> Keyword<'a> {
    /// Whether this is the keyword `name` (ASCII, compared without case).
    pub fn is(self, name: &str) -> bool {
        self.0.eq_ignore_ascii_case(name.as_bytes())
    }

    /// The lexer's ASCII-lowercased copy, for matching reader keywords. Non-ASCII bytes
    /// cannot become keyword bytes.
    pub fn folded(self) -> Vec<u8> {
        self.0.to_ascii_lowercase()
    }

    /// The key exactly as authored, for a reader that uses its text.
    pub fn text(self) -> &'a [u8] {
        self.0
    }
}

impl Item {
    /// The key seen by keyword dispatch. Anonymous items, quoted keys, bare keys
    /// starting with a digit or `-`, and punctuation tokens (`=` `<` `>` `{` `}` `(`
    /// `)` `,` `?=`) return `None`. These all take the reader's nonkeyword branch.
    pub fn keyword(&self) -> Option<Keyword<'_>> {
        let key = self.key.as_deref()?;
        looked_up(key, self.key_quoted).then_some(Keyword(key))
    }

    /// The key's text as a reader that takes it as a name stores it (an
    /// idea's or a group's token, a definition's name), converted as
    /// [`Value::read_str`] converts a value; an anonymous item's is empty.
    pub fn key_str(&self) -> Result<&str, NotText> {
        text(self.key.as_deref().unwrap_or_default())
    }

    /// Whether the key is the keyword `name` (see [`Self::keyword`]).
    pub fn keyword_is(&self, name: &str) -> bool {
        self.keyword().is_some_and(|keyword| keyword.is(name))
    }

    /// The ASCII-folded bare value seen by keyword dispatch. Quoted values, numbers,
    /// punctuation, End tokens regardless of retained text, and blocks return `None`
    /// and take the nonkeyword branch.
    pub fn value_keyword(&self) -> Option<String> {
        let text = self.value.scalar()?;
        (self.kind == Kind::Bare && looked_up(text, false))
            .then(|| String::from_utf8_lossy(&text.to_ascii_lowercase()).into_owned())
    }

    /// The value is a quoted string, which variable resolution always leaves literal.
    pub fn quoted(&self) -> bool {
        self.kind == Kind::Quoted && matches!(self.value, Value::Scalar(_))
    }

    /// The value is a bare number token as recognized by [`number_token`].
    pub fn number(&self) -> bool {
        self.kind == Kind::Bare
            && self
                .value
                .scalar()
                .is_some_and(|text| number_token(text, false))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub items: Vec<Item>,
}

impl Document {
    /// Finds the first occurrence. Duplicate keys remain in source order.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<&Value> {
        self.items
            .iter()
            .find(|item| item.key.as_deref() == Some(key.as_ref()))
            .map(|item| &item.value)
    }

    pub fn get_all<'a>(
        &'a self,
        key: impl AsRef<[u8]> + 'a,
    ) -> impl Iterator<Item = &'a Value> + 'a {
        self.items
            .iter()
            .filter(move |item| item.key.as_deref() == Some(key.as_ref()))
            .map(|item| &item.value)
    }

    /// All values whose keys match under the shared ASCII case fold ([`fold_token`]),
    /// so `ENABLE_EQUIPMENTS` matches `enable_equipments`.
    pub fn get_all_keyword<'a>(&'a self, keyword: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.items
            .iter()
            .filter(move |item| is_keyword(item, keyword))
            .map(|item| &item.value)
    }
}

/// Whether an item's key is `keyword` as the lexer identifies tokens: only ASCII A–Z
/// fold (see [`Document::get_all_keyword`]).
pub fn is_keyword(item: &Item, keyword: &str) -> bool {
    item.key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case(keyword.as_bytes()))
}

/// FNV-1a over a variable name after its `@`.
fn variable_hash(name: &[u8]) -> u32 {
    name.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

/// Whether a key is eligible for keyword lookup: a bare token that starts with neither
/// a digit nor `-` and is not punctuation (`=` `<` `>` `{` `}` `(` `)` `,` `?=`).
/// Quoted tokens are ineligible. Shared by [`Item::keyword`] and the script reader's
/// `Field::keyword`.
pub(crate) fn looked_up(key: &[u8], quoted: bool) -> bool {
    !quoted
        && key.first().is_some_and(|first| {
            !matches!(
                first,
                b'0'..=b'9' | b'-' | b'=' | b'<' | b'>' | b'{' | b'}' | b'(' | b')' | b',' | b'?'
            )
        })
}

/// Whether a bare token is numeric: starts with a digit or `-`, or its text through the
/// first NUL contains only digits, `-` and `.`. A leading NUL therefore gives a number
/// token. Quoted tokens stay quoted, and a slot with no byte retains its initial empty
/// kind, including the value before `}`.
pub fn number_token(text: &[u8], quoted: bool) -> bool {
    let string = c_string(text);
    !quoted
        && match text.first() {
            Some(b'0'..=b'9' | b'-') => true,
            Some(_) => string
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'-' | b'.')),
            None => false,
        }
}

/// Parses a complete text document, including an optional leading UTF-8 BOM:
/// a save or one of the formats the tree reads (see the module).
///
/// Errors use zero-based byte offsets in the original input. `HOI4txt` is an
/// ordinary anonymous scalar, so save-header validation belongs to the caller.
pub fn parse(bytes: &[u8]) -> io::Result<Document> {
    Parser::new(bytes, false, false).document()
}

/// Project save writer output, including an empty name emitted as `=fixed5` inside
/// `variables`. Script loading instead takes `=` as a key with a retained value; that
/// recovery does not represent the saved state.
pub(crate) fn parse_save(bytes: &[u8]) -> io::Result<Document> {
    let mut parser = Parser::new(bytes, false, false);
    parser.save_variables = true;
    parser.document()
}

/// The 1-based lines of a block's `{` and `}`, counting only line feeds. A block closed
/// by EOF ([`parse_unclosed`]) closes on the input's last line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockLines {
    pub open: u32,
    pub close: u32,
}

/// [`parse_unclosed`] with the lines of every block and tagged block, in the
/// order their `{` appear: a pre-order walk of the document's
/// [`Value::Block`] and [`Value::Tagged`] values meets them in this order.
pub fn parse_unclosed_lines(input: &[u8]) -> io::Result<(Document, Vec<BlockLines>, bool)> {
    let mut parser = Parser::new(input, true, true);
    let document = parser.document()?;
    let closed = parser.lexer.at_eof.closed;
    Ok((
        document,
        parser.lines.map(|lines| lines.blocks).unwrap_or_default(),
        closed,
    ))
}

/// Every scalar of `bytes` in order, as the parser's own lexer reads and
/// decodes it (comment boundaries, quoting and escapes), for text the
/// parser refuses. No error stops the scan: a structural error does not
/// concern the lexer, a NUL byte separates words and an unterminated quote
/// runs to the end of the text.
pub fn scalars(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut lexer = Lexer {
        bytes,
        position: usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3,
        at_eof: EndOfFile::default(),
        lenient: true,
    };
    let mut scalars = Vec::new();
    while let Ok(Some(token)) = lexer.next() {
        if let TokenKind::Scalar { bytes, .. } = token.kind {
            scalars.push(bytes);
        }
    }
    scalars
}

#[derive(Debug)]
enum TokenKind {
    Scalar { bytes: Vec<u8>, quoted: bool },
    Open,
    Close,
    Operator(Operator),
}

#[derive(Debug)]
struct Token {
    offset: usize,
    kind: TokenKind,
}

struct Lexer<'a> {
    bytes: &'a [u8],
    position: usize,
    /// The end of the input ends an open quoted string ([`parse_unclosed`]).
    at_eof: EndOfFile,
    /// [`scalars`]' mode: NUL bytes separate words and an unterminated
    /// quote ends the text, instead of either failing.
    lenient: bool,
}

/// What the end of the input does to what the text leaves open.
#[derive(Clone, Copy, Default)]
struct EndOfFile {
    /// Close it, as the game's readers do; otherwise it is an error.
    closes: bool,
    /// Something was closed.
    closed: bool,
}

impl Lexer<'_> {
    fn next(&mut self) -> io::Result<Option<Token>> {
        while let Some(&byte) = self.bytes.get(self.position) {
            if byte.is_ascii_whitespace() || (self.lenient && byte == 0) {
                self.position += 1;
            } else if byte == b'#' {
                while let Some(&comment) = self.bytes.get(self.position) {
                    if comment == b'\n' || comment == b'\r' {
                        break;
                    }
                    if comment == 0 && !self.lenient {
                        return Err(invalid(self.position, "NUL byte in text"));
                    }
                    self.position += 1;
                }
            } else {
                break;
            }
        }
        let offset = self.position;
        let Some(&byte) = self.bytes.get(offset) else {
            return Ok(None);
        };
        self.position += 1;
        let kind = match byte {
            0 => return Err(invalid(offset, "NUL byte in text")),
            b'{' => TokenKind::Open,
            b'}' => TokenKind::Close,
            b'=' => TokenKind::Operator(Operator::Equals),
            b'<' => TokenKind::Operator(Operator::Less),
            b'>' => TokenKind::Operator(Operator::Greater),
            b'"' => TokenKind::Scalar {
                bytes: self.quoted(offset)?,
                quoted: true,
            },
            _ => {
                while let Some(&next) = self.bytes.get(self.position) {
                    if next.is_ascii_whitespace()
                        || matches!(next, b'{' | b'}' | b'=' | b'<' | b'>' | b'"' | b'#')
                        || (self.lenient && next == 0)
                    {
                        break;
                    }
                    if next == 0 {
                        return Err(invalid(self.position, "NUL byte in text"));
                    }
                    self.position += 1;
                }
                TokenKind::Scalar {
                    bytes: self.bytes[offset..self.position].to_vec(),
                    quoted: false,
                }
            }
        };
        Ok(Some(Token { offset, kind }))
    }

    fn quoted(&mut self, start: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        while let Some(&byte) = self.bytes.get(self.position) {
            let offset = self.position;
            self.position += 1;
            match byte {
                0 if self.lenient => {}
                0 => return Err(invalid(offset, "NUL byte in quoted string")),
                b'"' => return Ok(bytes),
                b'\\' => match self.bytes.get(self.position).copied() {
                    Some(escaped @ (b'"' | b'\\')) => {
                        bytes.push(escaped);
                        self.position += 1;
                    }
                    // Unknown escape sequences are literal bytes, including
                    // the backslash. A trailing slash still cannot close a quote.
                    _ => bytes.push(byte),
                },
                _ => bytes.push(byte),
            }
        }
        // A quoted token ends at EOF and retains the text read so far.
        if self.at_eof.closes {
            self.at_eof.closed = true;
            return Ok(bytes);
        }
        if self.lenient {
            return Ok(bytes);
        }
        Err(invalid(start, "unterminated quoted string"))
    }
}
/// Parse a `.gfx`, `.font` or particle file, closing open blocks at EOF as its reader
/// loops do. EOF also ends quoted strings and closes blocks inside comments. Script
/// content uses [`native`], whose loops stop there directly. Return whether EOF closed
/// anything for caller reporting. A truncated save remains an error: [`parse`] never
/// recovers.
pub fn parse_unclosed(input: &[u8]) -> io::Result<(Document, bool)> {
    let mut parser = Parser::new(input, true, false);
    let document = parser.document()?;
    Ok((document, parser.lexer.at_eof.closed))
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token>,
    lines: Option<Lines>,
    /// The offset of the last closing brace a block ended on.
    closed_at: usize,
    save_variables: bool,
}

struct Lines {
    /// Offsets of every `\n`, ascending.
    newlines: Vec<usize>,
    blocks: Vec<BlockLines>,
}

impl Lines {
    fn line(&self, offset: usize) -> u32 {
        let before = self.newlines.partition_point(|&newline| newline < offset);
        u32::try_from(before).unwrap_or(u32::MAX).saturating_add(1)
    }
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8], closes_at_eof: bool, lines: bool) -> Self {
        Self {
            lexer: Lexer {
                bytes,
                position: usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3,
                at_eof: EndOfFile {
                    closes: closes_at_eof,
                    closed: false,
                },
                lenient: false,
            },
            lookahead: None,
            lines: lines.then(|| Lines {
                newlines: bytes
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == b'\n')
                    .map(|(offset, _)| offset)
                    .collect(),
                blocks: Vec::new(),
            }),
            closed_at: 0,
            save_variables: false,
        }
    }

    fn document(&mut self) -> io::Result<Document> {
        Ok(Document {
            items: self.items(0, false, false)?,
        })
    }

    fn peek(&mut self) -> io::Result<Option<&Token>> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next()?;
        }
        Ok(self.lookahead.as_ref())
    }

    fn next(&mut self) -> io::Result<Option<Token>> {
        match self.lookahead.take() {
            Some(token) => Ok(Some(token)),
            None => self.lexer.next(),
        }
    }

    fn items(&mut self, depth: usize, in_block: bool, variable_map: bool) -> io::Result<Vec<Item>> {
        let mut items = Vec::new();
        while let Some(token) = self.next()? {
            match token.kind {
                TokenKind::Close if in_block => {
                    self.closed_at = token.offset;
                    return Ok(items);
                }
                TokenKind::Close => return Err(invalid(token.offset, "unexpected closing brace")),
                TokenKind::Operator(Operator::Equals) if variable_map => {
                    let value = self.next()?.ok_or_else(|| {
                        invalid(token.offset, "empty saved variable key lacks a value")
                    })?;
                    let TokenKind::Scalar {
                        bytes,
                        quoted: false,
                    } = value.kind
                    else {
                        return Err(invalid(
                            value.offset,
                            "empty saved variable requires fixed5",
                        ));
                    };
                    // The writer emits signed fixed5 with at most five fractional
                    // digits and no leading plus.
                    let decimal = crate::save::Decimal::parse(&bytes).map_err(|error| {
                        invalid(value.offset, &format!("empty saved variable: {error}"))
                    })?;
                    if bytes.starts_with(b"+") || decimal.fractional_digits > 5 {
                        return Err(invalid(
                            value.offset,
                            "empty saved variable requires fixed5",
                        ));
                    }
                    items.push(Item {
                        key: Some(Vec::new()),
                        key_quoted: false,
                        operator: Operator::Equals,
                        value: Value::Scalar(bytes),
                        kind: Kind::Bare,
                    });
                }
                TokenKind::Operator(Operator::Equals) => {
                    return Err(invalid(token.offset, "expected an item before '='"));
                }
                // Script replay takes a stray `<` or `>` as a key, out of step with the
                // text. The tree keeps it as an anonymous value.
                TokenKind::Operator(operator) => items.push(Item {
                    key: None,
                    key_quoted: false,
                    operator: Operator::Equals,
                    value: Value::Scalar(operator.text().to_vec()),
                    kind: Kind::Bare,
                }),
                TokenKind::Open => items.push(Item {
                    key: None,
                    key_quoted: false,
                    operator: Operator::Equals,
                    value: Value::Block(self.block(depth, token.offset, false)?),
                    kind: Kind::Open,
                }),
                TokenKind::Scalar { bytes, quoted } => {
                    if let Some(TokenKind::Operator(operator)) =
                        self.peek()?.map(|token| &token.kind)
                    {
                        let operator = *operator;
                        self.next()?;
                        let key_quoted = quoted;
                        let variable_map = self.save_variables
                            && !quoted
                            && bytes == b"variables"
                            && operator == Operator::Equals;
                        let (value, kind) = self.value(depth, variable_map)?;
                        items.push(Item {
                            key: Some(bytes),
                            key_quoted,
                            operator,
                            value,
                            kind,
                        });
                    } else {
                        let value = self.scalar_or_tag(bytes, quoted, depth)?;
                        items.push(Item {
                            key: None,
                            key_quoted: false,
                            operator: Operator::Equals,
                            kind: tree_kind(&value, quoted),
                            value,
                        });
                    }
                }
            }
        }
        if in_block && !self.lexer.at_eof.closes {
            return Err(invalid(
                self.lexer.position,
                "expected closing brace before end of input",
            ));
        }
        if in_block {
            self.lexer.at_eof.closed = true;
            self.closed_at = self.lexer.bytes.len();
        }
        Ok(items)
    }

    /// A value, and its token's kind.
    fn value(&mut self, depth: usize, variable_map: bool) -> io::Result<(Value, Kind)> {
        let token = self
            .next()?
            .ok_or_else(|| invalid(self.lexer.position, "expected a value before end of input"))?;
        match token.kind {
            TokenKind::Open => Ok((
                Value::Block(self.block(depth, token.offset, variable_map)?),
                Kind::Open,
            )),
            TokenKind::Scalar { bytes, quoted } => {
                let value = self.scalar_or_tag(bytes, quoted, depth)?;
                let kind = tree_kind(&value, quoted);
                Ok((value, kind))
            }
            // The field reader takes any token after the operator as its value: `a >=
            // 5` takes `=`.
            TokenKind::Operator(operator) => Ok((
                Value::Scalar(operator.text().to_vec()),
                match operator {
                    Operator::Equals => Kind::Equals,
                    Operator::Less => Kind::Less,
                    Operator::Greater => Kind::Greater,
                },
            )),
            TokenKind::Close => Err(invalid(token.offset, "expected a value")),
        }
    }

    fn scalar_or_tag(&mut self, bytes: Vec<u8>, quoted: bool, depth: usize) -> io::Result<Value> {
        let is_color =
            !quoted && (bytes.eq_ignore_ascii_case(b"rgb") || bytes.eq_ignore_ascii_case(b"hsv"));
        if is_color && matches!(self.peek()?.map(|token| &token.kind), Some(TokenKind::Open)) {
            let offset = self.next()?.expect("peeked opening brace").offset;
            Ok(Value::Tagged {
                tag: bytes,
                items: self.block(depth, offset, false)?,
            })
        } else {
            Ok(Value::Scalar(bytes))
        }
    }

    fn block(&mut self, depth: usize, offset: usize, variable_map: bool) -> io::Result<Vec<Item>> {
        if depth == MAX_DEPTH {
            return Err(invalid(
                offset,
                "block nesting exceeds the depth limit of 128",
            ));
        }
        let index = self.lines.as_mut().map(|lines| {
            let open = lines.line(offset);
            lines.blocks.push(BlockLines { open, close: open });
            lines.blocks.len() - 1
        });
        let items = self.items(depth + 1, true, variable_map)?;
        if let (Some(index), Some(lines)) = (index, self.lines.as_mut()) {
            lines.blocks[index].close = lines.line(self.closed_at);
        }
        Ok(items)
    }
}

/// The kind the tree gives a scalar or a tagged block it read.
fn tree_kind(value: &Value, quoted: bool) -> Kind {
    match value {
        Value::Scalar(_) if quoted => Kind::Quoted,
        Value::Scalar(_) => Kind::Bare,
        Value::Block(_) | Value::Tagged { .. } => Kind::Open,
    }
}

fn invalid(offset: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Clausewitz text at byte {offset}: {message}"),
    )
}

#[cfg(test)]
mod tests;
