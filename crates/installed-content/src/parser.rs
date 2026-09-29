//! The shared, byte-preserving Clausewitz text grammar.
//!
//! Numbers remain lexical bytes; callers choose their engine representation.
//! Quoted strings decode `\"` and `\\`, while other backslash sequences remain
//! literal. This also preserves paths and game formatting bytes without UTF-8
//! replacement or an invented numeric precision.

use std::io;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// `None` represents an anonymous list element, block, or file header.
    pub key: Option<Vec<u8>>,
    pub value: Value,
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

/// Parses a complete text document, including an optional leading UTF-8 BOM.
///
/// Errors use zero-based byte offsets in the original input. `HOI4txt` is an
/// ordinary anonymous scalar, so save-header validation belongs to the caller.
pub fn parse(bytes: &[u8]) -> io::Result<Document> {
    Ok(parse_spanned(bytes, false)?.0)
}

/// The 1-based lines of one block's `{` and `}`, counting `\n` only, as the
/// game's reader counts them (`CFile::CFile@1424DC040` starts at 1 and
/// `CMemoryFile::ReadNextCharacter@1424D92C0` adds one on the character after
/// a `\n`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockLines {
    pub open: u32,
    pub close: u32,
}

/// [`parse_unclosed`] with the lines of every block and tagged block, in the
/// order their `{` appear: a pre-order walk of the document's
/// [`Value::Block`] and [`Value::Tagged`] values meets them in this order.
pub fn parse_unclosed_lines(input: &[u8]) -> io::Result<(Document, Vec<BlockLines>, bool)> {
    let first = match parse_spanned(input, true) {
        Ok((document, lines)) => return Ok((document, lines, false)),
        Err(error) => error,
    };
    for missing in 1..=MAX_DEPTH {
        let mut repaired = input.to_vec();
        repaired.extend(std::iter::repeat_n(b'}', missing));
        if let Ok((document, lines)) = parse_spanned(&repaired, true) {
            return Ok((document, lines, true));
        }
    }
    Err(first)
}

fn parse_spanned(bytes: &[u8], lines: bool) -> io::Result<(Document, Vec<BlockLines>)> {
    let mut parser = Parser {
        lexer: Lexer {
            bytes,
            position: usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3,
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
    };
    let items = parser.items(0, false)?;
    Ok((
        Document { items },
        parser.lines.map(|lines| lines.blocks).unwrap_or_default(),
    ))
}

#[derive(Debug)]
enum TokenKind {
    Scalar { bytes: Vec<u8>, quoted: bool },
    Open,
    Close,
    Equals,
}

#[derive(Debug)]
struct Token {
    offset: usize,
    kind: TokenKind,
}

struct Lexer<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Lexer<'_> {
    fn next(&mut self) -> io::Result<Option<Token>> {
        while let Some(&byte) = self.bytes.get(self.position) {
            if byte.is_ascii_whitespace() {
                self.position += 1;
            } else if byte == b'#' {
                while let Some(&comment) = self.bytes.get(self.position) {
                    if comment == b'\n' || comment == b'\r' {
                        break;
                    }
                    if comment == 0 {
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
            b'=' => TokenKind::Equals,
            b'"' => TokenKind::Scalar {
                bytes: self.quoted(offset)?,
                quoted: true,
            },
            _ => {
                while let Some(&next) = self.bytes.get(self.position) {
                    if next.is_ascii_whitespace()
                        || matches!(next, b'{' | b'}' | b'=' | b'"' | b'#')
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
        Err(invalid(start, "unterminated quoted string"))
    }
}
/// Parses a game script file, closing blocks the file leaves open at its end,
/// as the game's own loader does.
///
/// Shipped files do this: `interface/powerbalanceview.gfx` stops inside its
/// `spriteTypes` block, and `common/ideas/SOV.txt` ends before its last brace,
/// and the game loads both. Returns whether braces had to be supplied so a
/// caller can report the file. A truncated save is still an error: only content
/// readers recover, never [`parse`].
pub fn parse_unclosed(input: &[u8]) -> io::Result<(Document, bool)> {
    let first = match parse(input) {
        Ok(document) => return Ok((document, false)),
        Err(error) => error,
    };
    for missing in 1..=MAX_DEPTH {
        let mut repaired = input.to_vec();
        repaired.extend(std::iter::repeat_n(b'}', missing));
        if let Ok(document) = parse(&repaired) {
            return Ok((document, true));
        }
    }
    Err(first)
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

impl Parser<'_> {
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
                TokenKind::Equals => {
                    return Err(invalid(token.offset, "expected an item before '='"));
                }
                TokenKind::Open => items.push(Item {
                    key: None,
                    value: Value::Block(self.block(depth, token.offset)?),
                }),
                TokenKind::Scalar { bytes, quoted } => {
                    if matches!(
                        self.peek()?.map(|token| &token.kind),
                        Some(TokenKind::Equals)
                    ) {
                        self.next()?;
                        items.push(Item {
                            key: Some(bytes),
                            value: self.value(depth)?,
                        });
                    } else {
                        items.push(Item {
                            key: None,
                            value: self.scalar_or_tag(bytes, quoted, depth)?,
                        });
                    }
                }
            }
        }
        if in_block {
            Err(invalid(
                self.lexer.position,
                "expected closing brace before end of input",
            ))
        } else {
            Ok(items)
        }
    }

    fn value(&mut self, depth: usize) -> io::Result<Value> {
        let token = self
            .next()?
            .ok_or_else(|| invalid(self.lexer.position, "expected a value before end of input"))?;
        match token.kind {
            TokenKind::Open => Ok(Value::Block(self.block(depth, token.offset)?)),
            TokenKind::Scalar { bytes, quoted } => self.scalar_or_tag(bytes, quoted, depth),
            TokenKind::Close | TokenKind::Equals => Err(invalid(token.offset, "expected a value")),
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

fn invalid(offset: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Clausewitz text at byte {offset}: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
            (b"a == b".as_slice(), 3),
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
