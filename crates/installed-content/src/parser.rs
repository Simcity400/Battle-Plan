//! The shared, byte-preserving Clausewitz text model, and its two readers.
//!
//! Script content (the files the game reads through a CReader) is read by
//! the reader replay, [`native`]: the game's lexer (1424A7A90) and each
//! reader's own field dispatch, so key dispatch, token kinds, `@name`
//! variables, comparisons, the end of the text and text read out of step
//! with its braces follow the game in one place.
//!
//! [`parse`], the tree, reads what the game's script readers do not replay:
//! saves, whose serializer writes balanced text without variables and every
//! field of which its own consumer reads, so a brace mismatch there is
//! corruption to refuse, not a script form to replay; and the `.gfx`,
//! `.gui`, `.asset`, `.font` and `.mod` formats and user settings, whose
//! native readers are not traced. Both return [`Item`]s, so consumers do not
//! care which read them.
//!
//! Numbers remain lexical bytes; callers choose their engine representation.
//! Quoted strings decode `\"` and `\\`, while other backslash sequences remain
//! literal. This also preserves paths and game formatting bytes without UTF-8
//! replacement or an invented numeric precision.

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

    /// The text CReader::ReadStringInto (1424ABB50) stores with its check on
    /// (third argument 0), the form every script reader of names, keys and
    /// pictures uses: read_string_token (1424B05E0) copies the value token's
    /// text up to its first NUL, and a text holding a line feed is reported
    /// (1424ACEE0) and stored as `Unreadable String` instead. `None` for a
    /// block, which no reader of one value is given (the replay reads `{`).
    pub fn read_string(&self) -> Option<&[u8]> {
        Some(read_string(self.scalar()?))
    }

    /// [`Self::read_string`] as the port's text. The game stores the bytes
    /// whatever they are (1424ABB50, 1424B05E0 filter nothing): text that is
    /// not UTF-8 is [`NotText::NotUtf8`], never absent, so a reader refuses
    /// by name what it cannot hold.
    pub fn read_str(&self) -> Result<&str, NotText> {
        text(self.read_string().ok_or(NotText::Block)?)
    }

    /// The value token's text as a reader that takes it as a C string reads
    /// it (hash_name 140BBA1B0 on the value slot's text, +0xC8): up to its
    /// first NUL, without ReadStringInto's line-feed check, as
    /// [`Self::read_str`] converts it.
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

/// What CReader::ReadStringInto (1424ABB50) stores for a string holding a
/// line feed.
pub const UNREADABLE_STRING: &[u8] = b"Unreadable String";

/// CReader::ReadStringInto (1424ABB50) with its check on, of a token's text:
/// read_string_token (1424B05E0) copies it up to its first NUL, and a text
/// holding a line feed is stored as [`UNREADABLE_STRING`].
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
    /// The key is a quoted token. CTextLexer::read_token (1424A7A90) gives a
    /// quoted token the id 0xF without looking it up in the keyword registry,
    /// so a reader that dispatches on the key's token id never sees it as a
    /// keyword. Ask [`Item::keyword`], never the key bytes.
    pub key_quoted: bool,
    /// The operator between a key and its value. CTextLexer::read_token
    /// (1424A7A90) ends a bare token at `<` and `>` and gives each its own
    /// token (0x1D4, 0x1D3), which 1424AC5E0 accepts beside `=` (1), so
    /// `skill>@x` is a key, an operator and a value. [`Operator::Equals`]
    /// for an anonymous item.
    pub operator: Operator,
    pub value: Value,
    /// The kind CTextLexer::read_token (1424A7A90) gave the token the reader
    /// took the value from, as the reader's slot keeps it: the value slot
    /// (+0xC0) after 1424AC5E0 and 1424AB540 (a reference takes its
    /// definition's kind), or the lexer's current token (+0x18) for a reader
    /// of it ([`native::Shape::Current`]). [`Kind::End`] is the end of the
    /// text a failed advance left there, its text kept, or the empty value
    /// before a `}`; neither is a keyword, a number or a quoted string,
    /// whatever its text. Ask [`Item::quoted`], [`Item::value_keyword`] and
    /// [`Item::number`], never the bytes. The tree ([`parse`]) gives
    /// [`Kind::Bare`], [`Kind::Quoted`] or, for a block, [`Kind::Open`].
    pub kind: Kind,
}

/// The operators 1424AC5E0 reads between a key and its value. The lexer
/// (1424A7A90) makes every one a single character: `<=` is `<` and then `=`,
/// `!` starts a comment, and `?=` (0x318) is no operator to 1424AC5E0.
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

/// A bare key as CTextLexer::read_token (1424A7A90) looks it up in the
/// keyword registry: through a lowercased copy (String_ToLowerInPlace
/// 1424B8790, compared by StringView_CompareNoCase 1424B6750). Readers
/// dispatch on the token id that lookup gives, so a keyword matches whatever
/// its case. Only the registry says whether the key is a keyword at all; a
/// reader compares it with the keywords it dispatches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyword<'a>(&'a [u8]);

impl<'a> Keyword<'a> {
    /// Whether this is the keyword `name` (ASCII, compared without case).
    pub fn is(self, name: &str) -> bool {
        self.0.eq_ignore_ascii_case(name.as_bytes())
    }

    /// The lexer's lowercased copy, for a `match` over a reader's keywords.
    /// ASCII only, as 1424B8790 folds: a non-ASCII byte is never a keyword's.
    pub fn folded(self) -> Vec<u8> {
        self.0.to_ascii_lowercase()
    }

    /// The key exactly as authored, for a reader that uses its text.
    pub fn text(self) -> &'a [u8] {
        self.0
    }
}

impl Item {
    /// The key as the reader's keyword dispatch sees it, the only way a
    /// reader asks whether a key is one of its keywords. `None` for an
    /// anonymous item, a quoted key (token 0xF, 1424A7A90), a bare key
    /// starting with a digit or `-`, which 1424A7A90 never looks up, and a
    /// punctuation token the reader took as a key (`=` `<` `>` `{` `}` `(`
    /// `)` `,` `?=`, which end a bare token there): each reaches the
    /// reader's branch for a name that is no keyword.
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

    /// The value token as a reader that dispatches on its token id sees it
    /// (a category's `type` 140FC0730, a `ledger` 141515DD0, a division
    /// names `type` 1409B9A20): the lexer's folded copy of a bare value it
    /// looked up (1424A7A90); `None` for a quoted value (token 0xF), a
    /// number, punctuation, the end of the text (0x13, whatever text it
    /// kept) and a block, which reach the reader's branch for a value that
    /// is no keyword.
    pub fn value_keyword(&self) -> Option<String> {
        let text = self.value.scalar()?;
        (self.kind == Kind::Bare && looked_up(text, false))
            .then(|| String::from_utf8_lossy(&text.to_ascii_lowercase()).into_owned())
    }

    /// The value is a quoted string (token 0xF, the lexer's flag +4), which
    /// 1424AB540 never reads as a variable.
    pub fn quoted(&self) -> bool {
        self.kind == Kind::Quoted && matches!(self.value, Value::Scalar(_))
    }

    /// The value is a number token (0xC, 1424A7A90): a bare token
    /// [`number_token`] names one.
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
}

/// FNV-1a over a variable name after its `@` (1424AC830, 1424AB540).
fn variable_hash(name: &[u8]) -> u32 {
    name.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

/// Whether CTextLexer::read_token (1424A7A90) looks a key token up in the
/// keyword registry: a bare token that does not start with a digit or `-`
/// (the number id 0xC) and is no punctuation the lexer makes a token of its
/// own (`=` `<` `>` `{` `}` `(` `)` `,` `?=`). A quoted token is 0xF. The one
/// key rule of [`Item::keyword`] and the replay's `Field::keyword`.
pub(crate) fn looked_up(key: &[u8], quoted: bool) -> bool {
    !quoted
        && key.first().is_some_and(|first| {
            !matches!(
                first,
                b'0'..=b'9' | b'-' | b'=' | b'<' | b'>' | b'{' | b'}' | b'(' | b')' | b',' | b'?'
            )
        })
}

/// Whether CTextLexer::read_token (1424A7A90) gives a token the number id
/// 0xC: a bare token starting with a digit or `-`, or one whose text up to
/// its first NUL (the lookup's `strlen` copy, and the loop `while (c != 0)`
/// after it) is made only of digits, `-` and `.` and is no keyword. So a
/// token starting with a NUL, an empty C string, is a number. A quoted token
/// is 0xF. Text with no byte is no token the lexer read: the zeroed slot
/// CReader::CReader (1424A9D30) leaves and the empty value 1424AC5E0 stores
/// before a `}` (1424B0110) both have the id 0.
pub fn number_token(text: &[u8], quoted: bool) -> bool {
    let string = &text[..text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len())];
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

/// The 1-based lines of one block's `{` and `}`, counting `\n` only, as the
/// game's reader counts them (`CFile::CFile@1424DC040` starts at 1 and
/// `CMemoryFile::ReadNextCharacter@1424D92C0` adds one on the character after
/// a `\n`). A block the end of the input closes ([`parse_unclosed`]) closes
/// on the input's last line.
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
        // 1424A7A90 ends a quoted token at the end of the input and hands
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
/// Parses a `.gfx`, `.font` or particle file, closing what the file leaves
/// open at its end as the game's readers do: CPersistent::Read (1424A9730)
/// and CReader::ReadStringList (1401426E0) each stop at the end-of-file
/// token (0x13) as at `}`, and the lexer (1424A7A90) ends a quoted string
/// there. The end closes them wherever it falls, inside a comment included.
/// Script content is read by [`native`], whose loops stop there themselves.
///
/// Shipped files do this: `interface/powerbalanceview.gfx` stops inside its
/// `spriteTypes` block, and the game loads it. Returns whether the end closed
/// anything, so a caller can report the file. A truncated save is still an
/// error: [`parse`] never recovers.
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
        }
    }

    fn document(&mut self) -> io::Result<Document> {
        Ok(Document {
            items: self.items(0, false)?,
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

    fn items(&mut self, depth: usize, in_block: bool) -> io::Result<Vec<Item>> {
        let mut items = Vec::new();
        while let Some(token) = self.next()? {
            match token.kind {
                TokenKind::Close if in_block => {
                    self.closed_at = token.offset;
                    return Ok(items);
                }
                TokenKind::Close => return Err(invalid(token.offset, "unexpected closing brace")),
                TokenKind::Operator(Operator::Equals) => {
                    return Err(invalid(token.offset, "expected an item before '='"));
                }
                // 1424AC5E0 takes a stray `<` or `>` as a key, out of step
                // with the text; it stays a value without a key here.
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
                    value: Value::Block(self.block(depth, token.offset)?),
                    kind: Kind::Open,
                }),
                TokenKind::Scalar { bytes, quoted } => {
                    if let Some(TokenKind::Operator(operator)) =
                        self.peek()?.map(|token| &token.kind)
                    {
                        let operator = *operator;
                        self.next()?;
                        let key_quoted = quoted;
                        let (value, kind) = self.value(depth)?;
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
    fn value(&mut self, depth: usize) -> io::Result<(Value, Kind)> {
        let token = self
            .next()?
            .ok_or_else(|| invalid(self.lexer.position, "expected a value before end of input"))?;
        match token.kind {
            TokenKind::Open => Ok((Value::Block(self.block(depth, token.offset)?), Kind::Open)),
            TokenKind::Scalar { bytes, quoted } => {
                let value = self.scalar_or_tag(bytes, quoted, depth)?;
                let kind = tree_kind(&value, quoted);
                Ok((value, kind))
            }
            // 1424AC5E0 takes the token after the operator as the value
            // whatever it is: `a >= 5` has the value `=`.
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
                items: self.block(depth, offset)?,
            })
        } else {
            Ok(Value::Scalar(bytes))
        }
    }

    fn block(&mut self, depth: usize, offset: usize) -> io::Result<Vec<Item>> {
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
        let items = self.items(depth + 1, true)?;
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
mod tests {
    use super::*;

    /// The word scan reads what the parser's lexer reads, past any error.
    #[test]
    fn scalars_follow_the_lexer_past_errors() {
        let words = |text: &[u8]| {
            scalars(text)
                .into_iter()
                .map(|word| String::from_utf8(word).unwrap())
                .collect::<Vec<_>>()
        };
        // A bare CR ends a comment; escapes are decoded; a structural error
        // and a NUL byte do not stop the scan; an unterminated quote runs
        // to the end.
        assert_eq!(
            words(b"# note\rentity = { name = \"a\\\"b\\\\c\" } } = x\0y # z\n\"open"),
            ["entity", "name", "a\"b\\c", "x", "y", "open"]
        );
        assert!(parse(b"a = \"open").is_err());
        assert!(parse(b"a = x\0y").is_err());
    }

    #[test]
    fn block_lines_follow_the_braces_in_preorder() {
        let (document, lines, repaired) = parse_unclosed_lines(
            b"a = {\n  b = { c = 1 }\n  color = rgb { 1 2 3 }\n}\r\nd = {\n\n  e = {\n}",
        )
        .unwrap();
        assert!(repaired);
        assert_eq!(document.items.len(), 2);
        let spans: Vec<_> = lines
            .iter()
            .map(|lines| (lines.open, lines.close))
            .collect();
        // a, b, the tagged colour, d, then e; the supplied brace closes d.
        assert_eq!(spans, [(1, 4), (2, 2), (3, 3), (5, 8), (7, 8)]);
        assert_eq!(
            parse(b"x = { }").unwrap(),
            parse_unclosed_lines(b"x = { }").unwrap().0
        );
    }

    #[test]
    fn document_keeps_save_header_duplicate_keys_and_mixed_lists() {
        let document = parse(
            b"\xef\xbb\xbf# save\r\nHOI4txt\nreplace_path=map\nreplace_path=\"history/states\"\n\
              data={ 42 tag=GER { 7 8 } empty={} # a comment\n 9 }",
        )
        .unwrap();
        assert_eq!(document.items[0].key, None);
        assert_eq!(
            document.items[0].value.scalar(),
            Some(b"HOI4txt".as_slice())
        );
        assert_eq!(
            document.get("replace_path").unwrap().scalar_str(),
            Some("map")
        );
        assert_eq!(
            document
                .get_all(b"replace_path")
                .map(|value| value.scalar().unwrap())
                .collect::<Vec<_>>(),
            vec![b"map".as_slice(), b"history/states".as_slice()]
        );
        let items = document.get("data").unwrap().items().unwrap();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].value.scalar_str(), Some("42"));
        assert_eq!(items[1].key.as_deref(), Some(b"tag".as_slice()));
        assert_eq!(items[1].value.scalar_str(), Some("GER"));
        assert_eq!(items[2].value.items().unwrap().len(), 2);
        assert_eq!(items[3].value.items(), Some([].as_slice()));
        assert_eq!(items[4].value.scalar_str(), Some("9"));
        assert!(document.get("missing").is_none());
    }

    #[test]
    fn color_tags_preserve_case_and_do_not_consume_ordinary_list_elements() {
        let document =
            parse(b"a=HSV { 0.1 0.15 0.4 } b=rgb{1 2 3} c={ rgb plain { 4 } \"rgb\" { 5 } }")
                .unwrap();
        assert!(
            matches!(document.get("a"), Some(Value::Tagged { tag, items }) if tag == b"HSV" && items.len() == 3)
        );
        assert!(
            matches!(document.get("b"), Some(Value::Tagged { tag, items }) if tag == b"rgb" && items.len() == 3)
        );
        let items = document.get("c").unwrap().items().unwrap();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].value.scalar_str(), Some("rgb"));
        assert_eq!(items[1].value.scalar_str(), Some("plain"));
        assert_eq!(items[3].value.scalar_str(), Some("rgb"));
    }

    #[test]
    fn quoted_strings_preserve_non_utf8_formatting_and_literal_unknown_escapes() {
        let document = parse(
            b"\"quoted key\"=\"a\\\"b\\\\c\\n\\q # { = }\" name=\"\x11HSpy\xff\x11!\" empty=\"\"",
        )
        .unwrap();
        assert_eq!(
            document.get("quoted key").unwrap().scalar(),
            Some(b"a\"b\\c\\n\\q # { = }".as_slice())
        );
        assert_eq!(
            document.get("name").unwrap().scalar(),
            Some(b"\x11HSpy\xff\x11!".as_slice())
        );
        assert_eq!(document.get("name").unwrap().scalar_str(), None);
        assert_eq!(document.get("empty").unwrap().scalar_str(), Some(""));
        assert!(Value::Scalar(Vec::new()).items().is_none());
        assert!(Value::Block(Vec::new()).scalar().is_none());
    }

    #[test]
    fn items_record_whether_their_value_is_quoted() {
        let document = parse(b"a = \"@x\" b = @x \"@y\" @y c = { } d = rgb { 1 }").unwrap();
        let quoted: Vec<_> = document.items.iter().map(Item::quoted).collect();
        assert_eq!(quoted, [true, false, true, false, false, false]);
    }

    /// 1424A7A90 gives a quoted key token 0xF without the keyword lookup and
    /// folds a bare one; a key starting with a digit or `-` is never looked up.
    #[test]
    fn keys_keep_their_token_kind_and_only_bare_keys_are_keywords() {
        let document =
            parse(b"Fallback_NAME = a \"fallback_name\" = b 1x = c -x = d { e } \"@q\" = 1")
                .unwrap();
        let items = &document.items;
        let quoted: Vec<_> = items.iter().map(|item| item.key_quoted).collect();
        assert_eq!(quoted, [false, true, false, false, false, true]);
        assert!(items[0].keyword_is("fallback_name"));
        assert_eq!(items[0].keyword().unwrap().folded(), b"fallback_name");
        assert_eq!(items[0].keyword().unwrap().text(), b"Fallback_NAME");
        for item in &items[1..5] {
            assert_eq!(item.keyword(), None, "{item:?}");
        }
    }

    /// The content readers ask [`Item::keyword`] whether a key is a keyword
    /// (1424A7A90: a quoted key is token 0xF, a bare one folds). A raw key
    /// comparison in their code outside the tests fails here unless it is
    /// allowed below with the address that makes it a text compare.
    #[test]
    fn content_readers_ask_the_keyword_helper() {
        const READERS: [(&str, &str); 10] = [
            ("division_names.rs", include_str!("division_names.rs")),
            ("ideas.rs", include_str!("ideas.rs")),
            ("leader_traits.rs", include_str!("leader_traits.rs")),
            ("modifiers.rs", include_str!("modifiers.rs")),
            ("terrain.rs", include_str!("terrain.rs")),
            ("script_files.rs", include_str!("script_files.rs")),
            ("characters.rs", include_str!("characters.rs")),
            ("ideologies.rs", include_str!("ideologies.rs")),
            (
                "modifier_definitions.rs",
                include_str!("modifier_definitions.rs"),
            ),
            (
                "scripted_localisation.rs",
                include_str!("scripted_localisation.rs"),
            ),
        ];
        const RAW: [&str; 6] = [
            "key.as_deref() ==",
            "key.as_deref() !=",
            "eq_ignore_ascii_case(",
            "to_ascii_lowercase(",
            ".get(b\"",
            "get_all(",
        ];
        // (file, line text) with why it compares text, not a keyword.
        const ALLOWED: [(&str, &str); 13] = [
            // A group name: 1409B4370 finds it by the folded hash 140BBA1B0
            // and `_stricmp` (14253D9F8).
            ("division_names.rs", "key.to_ascii_lowercase()"),
            // A trait name: 140710200 compares its hash and `_stricmp`
            // (14253D9F8), selecting the first match in load order.
            ("leader_traits.rs", "let name = token.to_ascii_lowercase();"),
            (
                "leader_traits.rs",
                "let name = found.token.to_ascii_lowercase();",
            ),
            // An idea group's name as the token registry keys it
            // (add_dynamic_token 1424A65A0 -> 1424A5EE0 folds A-Z).
            (
                "ideas.rs",
                "(!matches!(name.first(), Some(b'0'..=b'9' | b'-'))).then(|| name.to_ascii_lowercase())",
            ),
            // An idea a repeated group names again: 140FC0E90's `_stricmp`.
            (
                "ideas.rs",
                "self.ideas[*index].token.eq_ignore_ascii_case(&idea.token)",
            ),
            // A modifier definition's name in the token registry (1405FF750
            // -> 1424A65A0, folded, StringView_CompareNoCase 1424B6750).
            (
                "modifier_definitions.rs",
                "self.by_name.get(&key.as_bytes().to_ascii_lowercase())",
            ),
            (
                "modifier_definitions.rs",
                "let name = definition.token.as_bytes().to_ascii_lowercase();",
            ),
            // A category name: FindTerrainByHashedName (1403BCA00).
            (
                "terrain.rs",
                ".position(|category| category.name.eq_ignore_ascii_case(name))",
            ),
            // A file path's extension, not a key.
            ("script_files.rs", "let lower = path.to_ascii_lowercase();"),
            ("characters.rs", "let lower = path.to_ascii_lowercase();"),
            // A recorded template's tag, a dynamic token (1424A65A0 ->
            // 1424A5EE0 folds A-Z).
            (
                "characters.rs",
                ".filter(|(tag, _)| tag.eq_ignore_ascii_case(&built.tag))",
            ),
            // The modifier a key names (1406042A0): the lexer folds a bare
            // key (1424A7A90), FindTokenNoCase a quoted one (1424A7060).
            ("modifiers.rs", "name: key.to_ascii_lowercase(),"),
            // An ideology group's number by its token, a dynamic token
            // (140A38190 -> 1424A6500 -> 1424A65A0 folds A-Z).
            ("ideologies.rs", ".get(&token.to_ascii_lowercase())"),
        ];
        let mut raw = Vec::new();
        for (file, source) in READERS {
            let code = source.split("#[cfg(test)]").next().unwrap_or_default();
            for line in code.lines().map(str::trim) {
                if !line.starts_with("//")
                    && RAW.iter().any(|pattern| line.contains(pattern))
                    && !ALLOWED.contains(&(file, line))
                {
                    raw.push(format!("{file}: {line}"));
                }
            }
        }
        assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
    }

    /// 1424A7A90 makes `<`, `>` and `=` single-character tokens that end a
    /// bare token, and 1424AC5E0 accepts each as the operator.
    #[test]
    fn comparisons_are_keys_with_their_operator_and_value_token() {
        let document = parse(b"t = { skill>@x phase > 99 a<\"010\" b >= 5 c = d } < 3").unwrap();
        let items = document.get("t").unwrap().items().unwrap();
        let shown: Vec<_> = items
            .iter()
            .map(|item| {
                (
                    item.key
                        .as_deref()
                        .map(|key| String::from_utf8_lossy(key).into_owned()),
                    item.operator,
                    String::from_utf8_lossy(item.value.scalar().unwrap()).into_owned(),
                    item.quoted(),
                )
            })
            .collect();
        let keyed = |key: &str, operator, value: &str, quoted| {
            (Some(key.to_owned()), operator, value.to_owned(), quoted)
        };
        assert_eq!(
            shown,
            [
                keyed("skill", Operator::Greater, "@x", false),
                keyed("phase", Operator::Greater, "99", false),
                keyed("a", Operator::Less, "010", true),
                // `>=` is `>` and then `=`: the game reads `=` as the value.
                (
                    Some("b".to_owned()),
                    Operator::Greater,
                    "=".to_owned(),
                    false
                ),
                (None, Operator::Equals, "5".to_owned(), false),
                keyed("c", Operator::Equals, "d", false),
            ]
        );
        let doubled = parse(b"a == b").unwrap();
        assert_eq!(doubled.items[0].value.scalar(), Some(b"=".as_slice()));
        assert_eq!(doubled.items[1].value.scalar(), Some(b"b".as_slice()));
        // A stray operator stays a value without a key.
        assert_eq!(document.items[1].key, None);
        assert_eq!(document.items[1].value.scalar(), Some(b"<".as_slice()));
    }

    /// 1424A7A90 gives the number id 0xC to a bare token starting with a
    /// digit or `-`, or made of digits, `-` and `.` up to its first NUL, so
    /// one starting with a NUL; text with no byte is the id-0 slot.
    #[test]
    fn number_tokens_follow_the_lexer() {
        for (text, quoted, number) in [
            (b"010".as_slice(), false, true),
            (b"1tail", false, true),
            (b"-x", false, true),
            (b".5", false, true),
            (b"1.5.2", false, true),
            (b"010", true, false),
            (b"x1", false, false),
            (b"yes", false, false),
            (b"", false, false),
            // The loop stops at the NUL: an empty C string, and `.5`.
            (b"\0x", false, true),
            (b".5\0x", false, true),
            (b"x\0", false, false),
        ] {
            assert_eq!(
                number_token(text, quoted),
                number,
                "{}",
                String::from_utf8_lossy(text)
            );
        }
    }

    /// The readers stop at the end-of-file token as at `}` (1424A9730,
    /// 1409B9A20, 1401426E0), wherever the end falls.
    #[test]
    fn the_end_of_a_script_file_closes_what_it_leaves_open() {
        for source in [
            b"G = { ordered = { 1 = { \"First\" } # tail".as_slice(),
            b"G = { ordered = { 1 = { \"First\" }\n# tail\n",
            b"G = { ordered = { 1 = { \"First",
        ] {
            assert!(parse(source).is_err());
            let (document, closed) = parse_unclosed(source).unwrap();
            assert!(closed, "{}", String::from_utf8_lossy(source));
            let ordered = document.get("G").unwrap().items().unwrap()[0]
                .value
                .items()
                .unwrap();
            assert_eq!(
                ordered[0].value.items().unwrap()[0].value.scalar(),
                Some(b"First".as_slice())
            );
        }
        let (_, closed) = parse_unclosed(b"G = { } # done").unwrap();
        assert!(!closed);
        assert!(parse_unclosed(b"G = { } }").is_err());
        assert!(parse_unclosed(b"G = { a = ").is_err());
    }

    #[test]
    fn numeric_scalars_retain_every_lexical_byte() {
        let scalars = [
            "-9223372036854775808",
            "18446744073709551615",
            "99999999999999999999999999999999999",
            "0.0000100",
            "+00042",
            "1.234567890123456789e-20",
            "1936.1.2.17",
        ];
        let document = parse(scalars.join(" ").as_bytes()).unwrap();
        let values: Vec<_> = document
            .items
            .iter()
            .map(|item| item.value.scalar_str().unwrap())
            .collect();
        assert_eq!(values, scalars);
    }

    #[test]
    fn malformed_documents_report_original_byte_offsets() {
        for (input, offset) in [
            (b"a =".as_slice(), 3),
            (b"a = {".as_slice(), 5),
            (b"a = }".as_slice(), 4),
            (b"}".as_slice(), 0),
            (b"\"unfinished".as_slice(), 0),
            (b"\"unfinished\\".as_slice(), 0),
            (b"a={x=y}}".as_slice(), 7),
            (b"\xef\xbb\xbf}".as_slice(), 3),
            (b"a=\"x\0y\"".as_slice(), 4),
            (b"a=x\0y".as_slice(), 3),
            (b"#x\0".as_slice(), 2),
        ] {
            let error = parse(input).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(
                error
                    .to_string()
                    .starts_with(&format!("Clausewitz text at byte {offset}:")),
                "input={input:?}: {error}"
            );
        }
    }

    #[test]
    fn depth_limit_applies_to_anonymous_and_tagged_blocks() {
        let accepted = format!("{}{}", "{".repeat(MAX_DEPTH), "}".repeat(MAX_DEPTH));
        assert!(parse(accepted.as_bytes()).is_ok());
        let rejected = format!("{}{}", "{".repeat(MAX_DEPTH + 1), "}".repeat(MAX_DEPTH + 1));
        assert!(
            parse(rejected.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("depth limit")
        );
        let tagged = format!(
            "{}{}",
            "HSV {".repeat(MAX_DEPTH + 1),
            "}".repeat(MAX_DEPTH + 1)
        );
        assert!(
            parse(tagged.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("depth limit")
        );
        assert_eq!(
            parse(b"\xef\xbb\xbf # comment only").unwrap(),
            Document::default()
        );
    }
}
