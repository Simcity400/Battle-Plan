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

/// read_bool_token@1424B06E0: exact C-string yes/no; failure stores nothing.
pub fn read_bool(text: &[u8]) -> Option<bool> {
    match c_string(text) {
        b"yes" => Some(true),
        b"no" => Some(false),
        _ => None,
    }
}

/// read_bool_field@1424ABCA0 -> 1424B06E0: ordered assignments; failure keeps the value.
pub fn read_bool_field(items: &[Item], key: &str, default: bool) -> bool {
    items
        .iter()
        .filter(|item| item.keyword_is(key))
        .fold(default, |state, item| {
            item.value.scalar().and_then(read_bool).unwrap_or(state)
        })
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

/// A text token's name under the lexer's ASCII fold (1424A7A90, 1424B8790).
pub fn fold_token(name: &str) -> String {
    name.to_ascii_lowercase()
}

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

    /// Every value of a keyword as the engine's script reader matches it: a key is a
    /// lexer token, found through `1424A5EE0`'s hash folding only ASCII A–Z and
    /// `_strnicmp@142549958` (`AddDynamicToken@1424A65A0`; [`fold_token`]), so
    /// `ENABLE_EQUIPMENTS` is `enable_equipments`.
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

/// Projects native save writer output. CVariables::vf2@140BB9D60 ->
/// 1424ADC10 -> 1424ADB70 emits a raw empty name as `=fixed5` inside
/// `variables`. Native text loading (1424A7A90 -> 1424AC5E0) instead takes
/// `=` as a key with a retained value; that recovery is not the saved state.
pub(crate) fn parse_save(bytes: &[u8]) -> io::Result<Document> {
    let mut parser = Parser::new(bytes, false, false);
    parser.save_variables = true;
    parser.document()
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
                    // 1424AF9A0 -> 1424B89F0 writes signed fixed5 with at
                    // most five fractional digits and no leading plus.
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
