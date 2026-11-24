
use std::io;

pub mod decimal;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Bare,
    Quoted,
    Equals,
    Less,
    Greater,
    Open,
    Close,
    Other,
    End,
    Comment,
    Code,
}

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

    pub fn scalar_str(&self) -> Option<&str> {
        std::str::from_utf8(self.scalar()?).ok()
    }

    pub fn items(&self) -> Option<&[Item]> {
        match self {
            Self::Scalar(_) => None,
            Self::Block(items) | Self::Tagged { items, .. } => Some(items),
        }
    }

    pub fn read_string(&self) -> Option<&[u8]> {
        Some(read_string(self.scalar()?))
    }

    pub fn read_str(&self) -> Result<&str, NotText> {
        text(self.read_string().ok_or(NotText::Block)?)
    }

    pub fn c_str(&self) -> Result<&str, NotText> {
        text(c_string(self.scalar().ok_or(NotText::Block)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotText {
    Block,
    NotUtf8(Vec<u8>),
    Unkeyed,
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
            Self::Unkeyed => f.write_str("absent: the entry has no key"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unheld(Option<String>);

impl Unheld {
    pub fn keep<'a>(&mut self, field: &str, text: Result<&'a str, NotText>) -> Option<&'a str> {
        text.inspect_err(|refused| {
            self.0
                .get_or_insert_with(|| format!("{field} is {refused}"));
        })
        .ok()
    }

    pub fn cause(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub fn absorb(&mut self, other: &Self) {
        if self.0.is_none() {
            self.0.clone_from(&other.0);
        }
    }
}

pub fn text(bytes: &[u8]) -> Result<&str, NotText> {
    std::str::from_utf8(bytes).map_err(|_| NotText::NotUtf8(bytes.to_vec()))
}

pub fn c_string(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}

pub fn read_bool(text: &[u8]) -> Option<bool> {
    match c_string(text) {
        b"yes" => Some(true),
        b"no" => Some(false),
        _ => None,
    }
}

pub fn read_bool_field(items: &[Item], key: &str, default: bool) -> bool {
    items
        .iter()
        .filter(|item| item.keyword_is(key))
        .fold(default, |state, item| {
            item.value.scalar().and_then(read_bool).unwrap_or(state)
        })
}

pub const UNREADABLE_STRING: &[u8] = b"Unreadable String";

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
    pub key: Option<Vec<u8>>,
    pub key_quoted: bool,
    pub operator: Operator,
    pub value: Value,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operator {
    #[default]
    Equals,
    Less,
    Greater,
}

impl Operator {
    pub fn text(self) -> &'static [u8] {
        match self {
            Self::Equals => b"=",
            Self::Less => b"<",
            Self::Greater => b">",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyword<'a>(&'a [u8]);

pub fn fold_token(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl<'a> Keyword<'a> {
    pub fn is(self, name: &str) -> bool {
        self.0.eq_ignore_ascii_case(name.as_bytes())
    }

    pub fn folded(self) -> Vec<u8> {
        self.0.to_ascii_lowercase()
    }

    pub fn text(self) -> &'a [u8] {
        self.0
    }
}

impl Item {
    pub fn keyword(&self) -> Option<Keyword<'_>> {
        let key = c_string(self.key.as_deref()?);
        looked_up(key, self.key_quoted).then_some(Keyword(key))
    }

    pub fn key_str(&self) -> Result<&str, NotText> {
        text(self.key.as_deref().ok_or(NotText::Unkeyed)?)
    }

    pub fn keyword_is(&self, name: &str) -> bool {
        self.keyword().is_some_and(|keyword| keyword.is(name))
    }

    pub fn value_keyword(&self) -> Option<String> {
        let text = c_string(self.value.scalar()?);
        (self.kind == Kind::Bare && looked_up(text, false))
            .then(|| String::from_utf8_lossy(&text.to_ascii_lowercase()).into_owned())
    }

    pub fn quoted(&self) -> bool {
        self.kind == Kind::Quoted && matches!(self.value, Value::Scalar(_))
    }

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

pub fn variable_hash(name: &[u8]) -> u32 {
    name.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn looked_up(key: &[u8], quoted: bool) -> bool {
    !quoted
        && key.first().is_some_and(|first| {
            !matches!(
                first,
                b'0'..=b'9' | b'-' | b'=' | b'<' | b'>' | b'{' | b'}' | b'(' | b')' | b',' | b'?'
            )
        })
}

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

pub fn parse(bytes: &[u8]) -> io::Result<Document> {
    crate::parse_counts::parse("balanced grammar", bytes);
    Parser::new(bytes, false, false).document()
}

pub fn parse_save(bytes: &[u8]) -> io::Result<Document> {
    crate::parse_counts::parse("save grammar", bytes);
    let mut parser = Parser::new(bytes, false, false);
    parser.save_variables = true;
    parser.document()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockLines {
    pub open: u32,
    pub first_token: u32,
    pub close: u32,
}

pub fn parse_unclosed_lines(input: &[u8]) -> io::Result<(Document, Vec<BlockLines>, bool)> {
    crate::parse_counts::parse("unclosed grammar with lines", input);
    let mut parser = Parser::new(input, true, true);
    let document = parser.document()?;
    let closed = parser.lexer.at_eof.closed;
    Ok((
        document,
        parser.lines.map(|lines| lines.blocks).unwrap_or_default(),
        closed,
    ))
}

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
    at_eof: EndOfFile,
    lenient: bool,
}

#[derive(Clone, Copy, Default)]
struct EndOfFile {
    closes: bool,
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
                    _ => bytes.push(byte),
                },
                _ => bytes.push(byte),
            }
        }
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
pub fn parse_unclosed(input: &[u8]) -> io::Result<(Document, bool)> {
    crate::parse_counts::parse("unclosed grammar", input);
    let mut parser = Parser::new(input, true, false);
    let document = parser.document()?;
    Ok((document, parser.lexer.at_eof.closed))
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token>,
    lines: Option<Lines>,
    closed_at: usize,
    save_variables: bool,
}

struct Lines {
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
                    let decimal = decimal::Decimal::parse(&bytes).map_err(|error| {
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
        let first_offset = if self.lines.is_some() {
            self.peek()?.map(|token| token.offset)
        } else {
            None
        };
        let index = self.lines.as_mut().map(|lines| {
            let open = lines.line(offset);
            lines.blocks.push(BlockLines {
                open,
                first_token: lines.line(first_offset.unwrap_or(self.lexer.position)),
                close: open,
            });
            lines.blocks.len() - 1
        });
        let items = self.items(depth + 1, true, variable_map)?;
        if let (Some(index), Some(lines)) = (index, self.lines.as_mut()) {
            lines.blocks[index].close = lines.line(self.closed_at);
        }
        Ok(items)
    }
}

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
