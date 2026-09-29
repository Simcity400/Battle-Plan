//! The game's CReader over its own lexer: what a reader reads, token by
//! token, including where the text leaves the shape the tree grammar gives
//! it.
//!
//! The tree ([`super::parse`]) pairs braces as written. The game's readers
//! do not: each field's reader consumes what it consumes, and the next one
//! goes on from there. A block given where a reader takes one value is not
//! skipped, a value without a key makes the key reader consume the next
//! token as its operator, a definition whose name is taken leaves its `=`
//! for the next key. [`read`] replays that consumption exactly:
//!
//! - [`lex`] is CTextLexer::read_token (1424A7A90);
//! - [`Reader`] holds the reader's key (+0x30), operator (+0x78) and value
//!   (+0xC0) slots, which a later read that finds no operator leaves as they
//!   were, and its `@name` table (+0x130);
//! - every block loop is CPersistent::Read (1424A9730): peek, stop at `}` or
//!   the end, else ReadKeyOperatorValue (1424AC5E0) and the reader's field
//!   dispatch, which the caller gives as a [`Shape`] per field;
//! - a file's top level stops only at the end (140AD6030, 1406B3690 and
//!   140AAD450 loop until token 0x13).
//!
//! The result is the tree as the readers saw it: every item a key, its value
//! token (resolved) or the block its reader read, and [`Read::misreads`],
//! each place the reading left the text, by line.

use super::{Item, Operator, Value};

/// A token kind of 1424A7A90 that the readers tell apart.
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
    /// The end of the text (token 0x13), or the empty value 1424AC5E0 sets
    /// before a `}`.
    End,
}

/// One token: its kind, its text (a single-character token's text is that
/// character) and the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub text: Vec<u8>,
    pub line: u32,
}

impl Token {
    fn end(line: u32) -> Self {
        Self {
            kind: Kind::End,
            text: Vec::new(),
            line,
        }
    }

    /// A reader slot as CReader::CReader (1424A9D30) leaves it: token 0,
    /// not quoted, no text, which no keyword lookup matches.
    fn slot() -> Self {
        Self {
            kind: Kind::Bare,
            text: Vec::new(),
            line: 0,
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
}

/// `isspace` in the C locale, which 1424A7A90 skips (142556630).
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// What [`lex`] gives: the tokens, and whether reading the last one failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    /// The last token is a bare one that ran into the end of the text:
    /// 1424A7A90 fills the token and returns `cVar5 != -1`, false, so the
    /// advance that reads it fails while the lexer's token keeps its text.
    /// A quoted string or a one-character token read at the end succeeds.
    pub last_fails: bool,
}

/// CTextLexer::read_token (1424A7A90) over the whole text, after a UTF-8
/// byte-order mark. A byte 0xFF reads as the end of the input: the lexer's
/// `char` compares equal to -1. `!`, `#` and `;` start a comment to the end
/// of the line; `"` a quoted string, in which `\"` and `\\` are one
/// character and any other backslash stays; `(` `)` `,` `<` `=` `>` `{` `}`
/// are one-character tokens and `?=` one token (a lone `?` is dropped);
/// anything else is a bare token up to white space or one of those
/// characters.
pub fn lex(bytes: &[u8]) -> Lexed {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let end = bytes.iter().position(|b| *b == 0xFF).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    let mut tokens = Vec::new();
    let mut last_fails = false;
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
    while at < bytes.len() {
        let byte = bytes[at];
        if space(byte) {
            if byte == b'\n' {
                line += 1;
            }
            at += 1;
            continue;
        }
        match byte {
            b'!' | b'#' | b';' => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'"' => {
                let start_line = line;
                let mut text = Vec::new();
                at += 1;
                let mut previous = 0_u8;
                while at < bytes.len() {
                    let byte = bytes[at];
                    at += 1;
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
                });
            }
            b'?' => {
                if bytes.get(at + 1) == Some(&b'=') {
                    tokens.push(Token {
                        kind: Kind::Other,
                        text: b"?=".to_vec(),
                        line,
                    });
                    at += 2;
                } else {
                    at += 1;
                }
            }
            _ => {
                if let Some(kind) = single(byte) {
                    tokens.push(Token {
                        kind,
                        text: vec![byte],
                        line,
                    });
                    at += 1;
                    continue;
                }
                let start = at;
                while at < bytes.len()
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
                tokens.push(Token {
                    kind: Kind::Bare,
                    text: bytes[start..at].to_vec(),
                    line,
                });
                last_fails = at == bytes.len();
            }
        }
    }
    Lexed { tokens, last_fails }
}

/// What a reader's field dispatch does with a field, after 1424AC5E0 has
/// read its key, operator and value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape<S> {
    /// A reader of the value token alone (ReadFixed5 1424ABB10,
    /// ReadScriptSignedInt32 1424AB970, ReadStringInto 1424ABB50,
    /// read_bool_field 1424ABCA0, a token compare), or a key only logged
    /// without RecordError: it consumes nothing more. Given a `{`, the
    /// block's contents are read as the enclosing fields and its `}` ends
    /// the enclosing block. A key reported through ReportUnexpectedToken
    /// (1424AD100 -> RecordError 1424ACF90) is [`Shape::Skip`]: RecordError
    /// skips a block (1424AD270).
    One,
    /// ReadParserStringWithCaseInsensitiveHash (140606100): as [`Shape::One`],
    /// but the text is the lexer's current token's (+0x18), not the value
    /// slot's: an `@name` value keeps its own text (1424AC5E0 resolves only
    /// into the value slot), and an empty value before `}` is `}`.
    Current,
    /// ReadObjectFromParser (1424ABB40) or a reader's own loop: the
    /// CPersistent::Read loop (1424A9730) over the tokens that follow, each
    /// field dispatched with `S`. It does not check the value was `{`.
    Object(S),
    /// An object only when the value token is `{`, else one value its
    /// reader takes (141389AC0's portrait types, ReadStringInto).
    ObjectIfOpen(S),
    /// An object only when the value token is `{` (1406031A0's
    /// `hidden_modifier`, 1413CA970's `research_bonus` test token 3), else
    /// RecordError (1424ACF90), whose skip is [`Shape::Skip`]'s.
    ObjectOrSkip(S),
    /// Read as an object twice (140ADCB60's new adjuster on the default
    /// token: 141001790 reads it, then 140ADDA6F again).
    Twice(S),
    /// ReadStringList (1401426E0): every token up to the next `}` or the
    /// end, each through 1424AB540 into the value slot.
    List,
    /// A list only when the value token is `{` (140ADE670), else one value.
    ListIfOpen,
    /// CStateTemplate's `provinces` loop (140AAF580): the tokens after the
    /// value up to the `}` that brings its depth, starting at 1, to 0 (a `{`
    /// adds one, a `}` takes one; the value itself is not counted), or to
    /// the end; each resolved (1424AB540) and read from the lexer, not the
    /// value slot. Every token but a brace is kept as a value without a key.
    Tokens,
    /// A list only when the value token is `{` (1403A8D80, 1413CA970's
    /// `traits`), else ReportUnexpectedToken or RecordError, whose skip is
    /// [`Shape::Skip`]'s.
    ListOrSkip,
    /// One entry, then SkipBlock (1424AD210), when the value token is `{`
    /// (140C89020, 140C8CCF0, 140C88A60): 1424AC5E0 reads the entry, which
    /// the caller's dispatch reads with `S`; a token after it that is no `}`
    /// is RecordError (1424ACF90, skipping a `{` it stands on); SkipBlock
    /// then passes over every token to the `}` that closes the value's
    /// block, defining nothing. Any other value is RecordError, as
    /// [`Shape::Skip`].
    Pair(S),
    /// std_pair_parser (140C88A60) of two strings, when the value token is
    /// `{`: a raw token loop with no key or operator. It peeks; at `}` or
    /// the end it reports (1424ACF90) fewer than two strings read, with no
    /// skip (the peeked token is the current one), and consumes it. Any
    /// other token, a `{` included, is advanced to, resolved (1424AB540)
    /// and copied into the value slot: the first two are the strings
    /// (ReadStringInto 1424ABB50, the consumer's [`super::Value::read_string`]),
    /// a third is reported, whose skip passes a block it opens, and
    /// SkipBlock (1424AD210) passes to the `}` that closes the pair,
    /// defining nothing. Any other value is RecordError, as [`Shape::Skip`].
    Strings,
    /// Reported (1424AD100 -> RecordError 1424ACF90), then
    /// SkipBlockResolvingVariables (1424AD270): a block is skipped whole,
    /// defining nothing, when the reader's current token is `{`.
    Skip,
    /// SkipCurrentBlock (1424AD1A0), unreported: the tokens to the matching
    /// `}` when the value token (+0xC0) is `{`, defining nothing.
    SkipIfOpen,
    /// CTrigger::LoadBody (1405431C0) of a trigger without the assign flag
    /// (0x800 at +0x24 clear), a reader the caller knows: given `{`, the
    /// loop peeks, stops at `}` or the end (advancing past it), else
    /// ReadKeyOperatorValue (1424AC5E0) and LoadKey (vtable +0x30) on the
    /// key slot whatever 1424AC5E0 read, so every field is dispatched with
    /// `S`, a key read without an operator with the slots it left
    /// ([`Field::fresh`] false). A token without a key is a key, never a
    /// list entry. Any other value is RecordError ("Non assign trigger is
    /// not enclosed in {}", 1424ACF90) with nothing read: [`Shape::Skip`]'s
    /// skip.
    Body(S),
    /// [`Shape::Body`] of a trigger whose fields the port does not
    /// interpret (a CAndTrigger such as an idea's `allowed`, a
    /// CMTTHModifier): each field through [`untraced`].
    Trigger,
    /// A block read by key whose reader the port does not interpret: a
    /// trigger or effect of the content whose name the port cannot look up
    /// (see [`Reader::generic`]).
    ByKey,
    /// The reader's own loop ends at this field, its value read and
    /// nothing more (1409B9A20's `ordered` loop on an index below 1): the
    /// read that closes the loop (the lexer's vtable +8 after it) consumes
    /// the next token, whatever it is, and the enclosing reader goes on
    /// from there. At a file's top level it is [`Shape::One`].
    Break,
    /// CColor's Read (142237160) through ReadObjectFromParser (1424ABB40),
    /// on the lexer's current token: a number token (0xC) is one packed
    /// value (1424B01A0); after `{` it reads the next three tokens as
    /// components and a fourth, which unless it is `}` is the alpha and
    /// followed by one more token, each through 1424AB540, and reports
    /// (1424ACEE0) a component that is no number token and a last token
    /// that is no `}`; any other token is reported and nothing is read.
    Color,
}

impl<S> Shape<S> {
    /// The same shape with its reader argument mapped, for a dispatch that
    /// embeds another reader's (a modifier block inside an idea).
    pub fn map<T>(self, node: impl FnOnce(S) -> T) -> Shape<T> {
        match self {
            Self::One => Shape::One,
            Self::Current => Shape::Current,
            Self::Object(s) => Shape::Object(node(s)),
            Self::ObjectIfOpen(s) => Shape::ObjectIfOpen(node(s)),
            Self::ObjectOrSkip(s) => Shape::ObjectOrSkip(node(s)),
            Self::Twice(s) => Shape::Twice(node(s)),
            Self::List => Shape::List,
            Self::ListIfOpen => Shape::ListIfOpen,
            Self::Tokens => Shape::Tokens,
            Self::ListOrSkip => Shape::ListOrSkip,
            Self::Pair(s) => Shape::Pair(node(s)),
            Self::Strings => Shape::Strings,
            Self::Skip => Shape::Skip,
            Self::SkipIfOpen => Shape::SkipIfOpen,
            Self::Body(s) => Shape::Body(node(s)),
            Self::Trigger => Shape::Trigger,
            Self::ByKey => Shape::ByKey,
            Self::Break => Shape::Break,
            Self::Color => Shape::Color,
        }
    }
}

/// CMeanTimeToHappen::vf4 (140545850): `factor` (ReadFixed5), `base`
/// (1424AFF10), `days`, `months` and `years` (ParseTokenInt32) take one
/// value; `modifier` is a CMTTHModifier, a CAndTrigger whose body
/// CTrigger::LoadBody (1405431C0) reads by key; any other key is only
/// logged ("unknown command"), its value token read and nothing skipped.
/// Keyword names from register_authored_keys (140082CB0): factor 0x296B,
/// months 0x296C, days 0x296D, years 0x296E, base 0x2C86, modifier 0x2965.
pub fn mean_time_to_happen<S>(field: &Field<'_>) -> Shape<S> {
    match field.keyword().as_deref() {
        Some(b"modifier") => Shape::Trigger,
        _ => Shape::One,
    }
}

/// A field of a known trigger or effect body whose own reader the port
/// does not interpret, the key-validity gap [`Reader::generic`] documents:
/// a block read by key ([`Shape::ByKey`]), else one value. LoadBody
/// (1405431C0) hands LoadKey (vtable +0x30) every key 1424AC5E0 reads, one
/// read without an operator included, with the value slot as the last read
/// left it: a stale `{` is [`Shape::ByKey`] (a `{` consumed as the
/// operator read by key as a fresh block is, anything else unsettled); a
/// stale value that opens no block goes to RecordError (1424ACF90, a body
/// reader's "Non assign trigger" and LoadKey's unknown key alike), whose
/// skip passes a `{` consumed as the operator. A punctuation key (a taken
/// name's `=`, 1424AC830) names no trigger, effect or scope, so LoadKey
/// (1405434F0) reaches RecordError whatever the stale value.
pub fn untraced<S>(field: &Field<'_>) -> Shape<S> {
    let punctuation = !matches!(field.key.kind, Kind::Bare | Kind::Quoted);
    if !field.fresh && punctuation {
        Shape::Skip
    } else if field.opens() {
        Shape::ByKey
    } else if !field.fresh {
        Shape::Skip
    } else {
        Shape::One
    }
}

/// A field as a reader's dispatch sees it.
#[derive(Debug)]
pub struct Field<'a> {
    /// The key token (+0x30).
    pub key: &'a Token,
    /// The value token (+0xC0), resolved; stale when `fresh` is false.
    pub value: &'a Token,
    /// 1424AC5E0 found an operator and read this field's own value.
    pub fresh: bool,
    /// The lexer's current token (+0x18) after 1424AC5E0: the value token,
    /// the `}` it left for an empty value, or, for a key read without an
    /// operator, the token it consumed in the operator's place. A skip
    /// after RecordError (1424AD270) tests this token, not the value slot.
    pub current: &'a Token,
}

impl Field<'_> {
    /// The key as the reader's keyword dispatch sees it: the lexer's folded
    /// copy of a bare key (1424A7A90); `None` for a quoted key (token 0xF),
    /// punctuation, and a bare key starting with a digit or `-`, which the
    /// lexer never looks up ([`super::looked_up`]).
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

    /// Whether the current token is `{`: a skip (1424AD270) passes over its
    /// block.
    pub fn skips_block(&self) -> bool {
        self.current.kind == Kind::Open
    }
}

/// One place the game's reading leaves the text, and the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misread {
    pub line: u32,
    pub what: String,
}

impl std::fmt::Display for Misread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.what)
    }
}

/// What [`read`] gives: the items as the readers saw them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub items: Vec<Item>,
    pub misreads: Vec<Misread>,
    /// A block read by key whose reading the port cannot settle, and why:
    /// everything after it in the file is unknown and not in `items`.
    pub unsettled: Option<Misread>,
}

/// What one ReadKeyOperatorValue (1424AC5E0) read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slots {
    /// A key, an operator and the value.
    Fresh,
    /// A key and a token that is no operator, consumed in its place.
    NoOperator,
    /// Nothing: the text ended after definitions.
    Ended,
}

/// The reader's state (CReader): the token stream with its one pending
/// peek, the key/operator/value slots and the variable table.
pub struct Reader<'t> {
    tokens: &'t [Token],
    /// The last token's advance fails ([`Lexed::last_fails`]).
    last_fails: bool,
    next: usize,
    /// The lexer's current token (+0x18). An advance at the end of the text
    /// returns false and leaves it as it was (1424A7A90 returns 0 before it
    /// touches the token); a peek that fails sets its kind to the end (0x13)
    /// and keeps its text.
    current: Token,
    /// A peek read `current` ahead (+100); the next advance returns its
    /// result (+0x10) again.
    pending: bool,
    pending_result: bool,
    key: Token,
    /// The operator slot (+0x78), which each [`Item::operator`] keeps.
    operator: Operator,
    value: Token,
    /// `@name` definitions (+0x130): the FNV-1a of the name after `@` and
    /// the whole defining token (1424AC830 stores kind, quoted flag and
    /// text, +0x28..+0x30).
    variables: Vec<(u32, Token)>,
    misreads: Vec<Misread>,
    unsettled: Option<Misread>,
    /// A field's reader ended its loop ([`Shape::Break`]).
    broken: bool,
}

/// Reads `bytes` as the file's reader does, with `dispatch` giving each
/// field's shape. `top` is the file level's dispatch argument.
pub fn read<S: Copy>(
    bytes: &[u8],
    top: S,
    dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
) -> Read {
    let lexed = lex(bytes);
    let mut reader = Reader::new(&lexed);
    let items = reader.file(top, dispatch);
    Read {
        items,
        misreads: reader.misreads,
        unsettled: reader.unsettled,
    }
}

impl<'t> Reader<'t> {
    fn new(lexed: &'t Lexed) -> Self {
        Self {
            tokens: &lexed.tokens,
            last_fails: lexed.last_fails,
            next: 0,
            current: Token::end(0),
            pending: false,
            pending_result: false,
            // CReader::CReader (1424A9D30) zeroes the key and value slots:
            // token 0 with empty text.
            key: Token::slot(),
            operator: Operator::Equals,
            value: Token::slot(),
            variables: Vec::new(),
            misreads: Vec::new(),
            unsettled: None,
            broken: false,
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

    fn misread(&mut self, line: u32, what: String) {
        self.misreads.push(Misread { line, what });
    }

    /// The lexer's advance (vtable +8): the pending peek's result, else the
    /// next token. `false` at the end, the token left as it was, and for a
    /// last bare token that ran into the end ([`Lexed::last_fails`]), which
    /// it reads.
    fn advance(&mut self) -> bool {
        if self.pending {
            self.pending = false;
            return self.pending_result;
        }
        let Some(token) = self.tokens.get(self.next) else {
            return false;
        };
        self.current = token.clone();
        self.next += 1;
        !(self.last_fails && self.next == self.tokens.len())
    }

    /// The readers' peek (1424AC5E0, 1424A9730, 1401426E0 inline it): unless
    /// one is pending, advance, keep the result (+0x10) and, when it failed,
    /// make the token the end (0x13), its text kept. Loops stop on `}` or
    /// the end.
    fn peek(&mut self) -> &Token {
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

    /// resolve_variable_token (1424AB540): a quoted token stays; a bare
    /// `@name` takes the defining token, else keeps its own text.
    fn resolve(&self, token: Token) -> Token {
        if token.kind == Kind::Quoted || !token.text.starts_with(b"@") {
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

    /// 1424AC830 with the lexer on `@name`.
    fn define(&mut self) {
        let name = self.current();
        let hash = super::variable_hash(&name.text[1..]);
        if self.variables.iter().any(|(defined, _)| *defined == hash) {
            // "Variable name %s is already taken": nothing more is read.
            self.misread(
                name.line,
                format!(
                    "variable {} is already taken (1424AC830), so its `=` and value are read as what follows",
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
            self.misread(
                name.line,
                format!(
                    "definition {} has no `=`: 1424AC830 consumes {} and defines nothing",
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
                self.misread(
                    name.line,
                    format!(
                        "definition {} takes {}, which 1424AC830 leaves unread, so the next key read starts from it",
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
                    self.misread(
                        name.line,
                        format!(
                            "definition {} keeps only the `{{` of its block value (1424AC830); the block's contents are read as what follows",
                            name.shown()
                        ),
                    );
                }
                token
            }
        };
        self.variables.push((hash, value));
    }

    /// CReader::ReadKeyOperatorValue (1424AC5E0): what it read into the
    /// slots.
    fn key_operator_value(&mut self) -> Slots {
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
        while self.unsettled.is_none() && self.peek().kind != Kind::End {
            if let Some(item) = self.field(top, dispatch) {
                items.push(item);
            }
            self.broken = false;
        }
        items
    }

    /// CPersistent::Read (1424A9730): fields until `}` or the end, which
    /// it consumes.
    fn object<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            match self.peek().kind {
                Kind::End => break,
                Kind::Close => {
                    self.advance();
                    break;
                }
                _ => {}
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

    /// One ReadKeyOperatorValue and the field's reader.
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
                        "key {} has no operator: 1424AC5E0 consumes {} as its operator and hands the key the previous value {}",
                        key.shown(),
                        consumed.shown(),
                        self.value.shown()
                    ),
                );
            }
            // CPersistent::Read (1424A9730) and the file loops dispatch the
            // key slot (+0x30) after 1424AC5E0 whatever it read: a text that
            // ends in a definition dispatches the slots as the last read left
            // them, the empty key 1424A9D30 set when no key was read yet.
            Slots::Ended => {
                let line = self.line();
                self.misread(
                    line,
                    format!(
                        "the text ends after a definition, so 1424AC5E0 reads no key and the reader takes the key slot {} again, with the value {}",
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
                current: &current,
            },
        );
        // The slot (+0x78) as 1424AC5E0 left it for this field, taken before
        // its block is read: each field inside writes the operator token
        // 1424A7A90 gave it (`<`, `>`, `=`) into the same slot.
        let operator = self.operator;
        // The token the reader took the value from: the lexer's current
        // token for a reader of it (140606100, CColor 142237160), else the
        // value slot, whose kind a failed advance's end (0x13) keeps.
        let kind = match shape {
            Shape::Current | Shape::Color => current.kind,
            _ => value.kind,
        };
        let value = self.apply(shape, &key, &value, dispatch);
        Some(Item {
            key: Some(key.text.clone()),
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
                        "field {} is {} where its reader reads a block: it is reported (1424ACF90)",
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
                        "field {} is {} where its reader reads `{{ string string }}`: it is reported (140C88A60 -> 1424ACF90)",
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
                    self.skip();
                    Value::Block(Vec::new())
                } else {
                    Value::Scalar(value.text.clone())
                }
            }
            Shape::Skip => {
                if self.current.kind == Kind::Open {
                    self.skip();
                    Value::Block(Vec::new())
                } else {
                    Value::Scalar(value.text.clone())
                }
            }
            Shape::Body(schema) if opens => Value::Block(self.object(schema, dispatch)),
            Shape::Trigger if opens => {
                Value::Block(self.object(false, &mut |_, field: &Field<'_>| untraced(field)))
            }
            Shape::Body(_) | Shape::Trigger => {
                self.misread(
                    key.line,
                    format!(
                        "field {} is {} where its trigger reads a block: \"Non assign trigger is not enclosed in {{}}\" is reported (1405431C0) and nothing is read",
                        key.shown(),
                        value.shown()
                    ),
                );
                self.apply(Shape::Skip, key, value, dispatch)
            }
            Shape::ByKey => {
                if opens && self.current.kind != Kind::Open {
                    // A key read without an operator, handed the stale `{`
                    // (LoadBody 1405431C0 calls LoadKey whatever 1424AC5E0
                    // read): which reader it is decides the reading.
                    self.unsettled = Some(Misread {
                        line: key.line,
                        what: format!(
                            "key {} has no operator and its reader is handed the previous value `{{`: a body reader (1405431C0) reads the tokens after {} up to the enclosing `}}`, a one-value reader or a key LoadKey does not know (RecordError 1424ACF90) reads nothing more, and the port does not know which reader it is",
                            key.shown(),
                            self.current.shown()
                        ),
                    });
                    Value::Block(Vec::new())
                } else if opens {
                    // `meta_effect` and `meta_trigger` copy their `text`
                    // block raw (141408C80).
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
            Shape::Color => self.color(key),
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
                    "field {} is {} where its reader reads `{{ key = value }}`: it is reported (1424ACF90)",
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
                    "field {} holds more than one entry: the reader reports it (1424ACF90) and SkipBlock (1424AD210) passes over the rest",
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
                            "field {} holds {} string(s) where its reader reads two: it is reported (140C88A60 -> 1424ACF90)",
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
                        "field {} holds {} after its two strings: it is reported (140C88A60 -> 1424ACF90) and SkipBlock (1424AD210) passes to the pair's `}}`",
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

    /// CReader::SkipBlock (1424AD210): advance, counting `{` and `}`, to the
    /// `}` that closes the block it stands in, or the end; nothing is
    /// resolved or defined.
    fn skip_block(&mut self) {
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
                    "field {} is {} where CColor reads a number or `{{`: it is reported and nothing more is read",
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
                            "field {} is a colour that does not close after four components: CColor reads {} as its end",
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
                        "field {} has {} where CColor reads a number: it is reported and read as a component",
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

    /// ReadStringList (1401426E0): tokens up to `}` or the end, which it
    /// consumes; each resolved and stored in the value slot.
    fn list(&mut self) -> Vec<Item> {
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

    /// [`Shape::Tokens`]: 140AAF580 peeks for the end first, then stops
    /// once the depth is 0, else takes the next token.
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

    /// SkipBlockResolvingVariables (1424AD270) with the lexer on `{`: every
    /// token to the matching `}`, nested blocks skipped the same way,
    /// defining nothing.
    fn skip(&mut self) {
        loop {
            if matches!(self.peek().kind, Kind::Close | Kind::End) {
                self.advance();
                return;
            }
            self.advance();
            if self.current().kind == Kind::Open {
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
                line,
                what: format!(
                    "variable {variable} is in a block read by a trigger or effect reader that also holds values without keys: a list reader (1401426E0) reads it without defining, a reader by key (1424AC5E0) out of step, and the port does not know which reader it is"
                ),
            });
            return Some(Vec::new());
        }
        Some(self.balanced())
    }

    /// A block read by key whose readers the port does not interpret. Its
    /// fields are read by key, a `{` value as another such block and any
    /// other value as one value, so its definitions register (the file's
    /// variables hold everywhere a reader reads by key). `meta_effect` and
    /// `meta_trigger` copy their `text` block raw (141408C80), defining
    /// nothing.
    ///
    /// Known gap (owner decision 2026-09-28): the port has no set of the
    /// trigger and effect names, so it cannot tell a list reader from a
    /// reader by key, and a key the game does not know is read by key here
    /// where the game skips its block (1424AD100, 1424AD270), its
    /// definitions registered: the lenient side. A block holding a value
    /// without a key is read as a list (1401426E0, which defines nothing);
    /// one that also holds a definition could be either, so the rest of the
    /// file is unsettled. The exact fix is a key-validity set from the
    /// executable's name tables plus the content's names.
    fn generic(&mut self, meta: bool) -> Vec<Item> {
        if !meta && let Some(list) = self.keyless_list() {
            return list;
        }
        let mut dispatch = |meta: bool, field: &Field<'_>| -> Shape<bool> {
            if meta && field.fresh && field.keyword_is("text") {
                return Shape::Skip;
            }
            untraced(field)
        };
        self.object(meta, &mut dispatch)
    }

    /// The first level of the block ahead (after its `{`): whether it holds
    /// a value without a key, and its first definition.
    fn scan(&self) -> (bool, Option<String>) {
        // A last token whose advance fails is the end to the peeks.
        let end = self.tokens.len() - usize::from(self.last_fails);
        let tokens = &self.tokens[self.next.min(end)..end];
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
mod tests {
    use super::*;

    fn texts(tokens: &[Token]) -> Vec<(Kind, String)> {
        tokens
            .iter()
            .map(|token| {
                (
                    token.kind,
                    String::from_utf8_lossy(&token.text).into_owned(),
                )
            })
            .collect()
    }

    /// 1424A7A90: comments on `!`, `#` and `;`, one-character operators and
    /// brackets, `?=`, quoted escapes, and 0xFF as the end.
    #[test]
    fn the_lexer_splits_as_the_game_does() {
        let lexed = lex(b"a=b!c\nd<5 e>=f (g,h) \"x\\\"y\\\\z\\n\" ?= ?q; tail\n{}\xff ignored");
        // `}` ends at the 0xFF: a one-character token succeeds.
        assert!(!lexed.last_fails);
        let tokens = lexed.tokens;
        assert_eq!(
            texts(&tokens),
            [
                (Kind::Bare, "a".into()),
                (Kind::Equals, "=".into()),
                (Kind::Bare, "b".into()),
                (Kind::Bare, "d".into()),
                (Kind::Less, "<".into()),
                (Kind::Bare, "5".into()),
                (Kind::Bare, "e".into()),
                (Kind::Greater, ">".into()),
                (Kind::Equals, "=".into()),
                (Kind::Bare, "f".into()),
                (Kind::Other, "(".into()),
                (Kind::Bare, "g".into()),
                (Kind::Other, ",".into()),
                (Kind::Bare, "h".into()),
                (Kind::Other, ")".into()),
                (Kind::Quoted, "x\"y\\z\\n".into()),
                (Kind::Other, "?=".into()),
                (Kind::Bare, "q".into()),
                (Kind::Open, "{".into()),
                (Kind::Close, "}".into()),
            ]
        );
        assert_eq!(tokens[3].line, 2);
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Test {
        Top,
        Body,
    }

    /// A test reader: `obj` reads an object, `list` a list, `n` one value,
    /// `trigger` by key; anything else in a body is skipped, at the top an
    /// object.
    fn test_read(text: &[u8]) -> Read {
        read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
            schema,
            field.keyword().as_deref(),
        ) {
            (Test::Top, _) => Shape::Object(Test::Body),
            (_, Some(b"obj")) => Shape::Object(Test::Body),
            (_, Some(b"list")) => Shape::List,
            (_, Some(b"n")) => Shape::One,
            (_, Some(b"trigger")) => Shape::ByKey,
            _ => Shape::Skip,
        })
    }

    fn keys(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| String::from_utf8_lossy(item.key.as_deref().unwrap_or(b"-")).into_owned())
            .collect()
    }

    fn block<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
        items
            .iter()
            .find(|item| item.key.as_deref() == Some(key.as_bytes()))
            .and_then(|item| item.value.items())
            .unwrap()
    }

    /// A block given to a one-value reader: `n` reads `{`, the block's
    /// fields are the body's, its `}` ends the body, and the body's own
    /// remaining fields are the file's.
    #[test]
    fn a_block_where_one_value_is_read_shifts_the_rest_up() {
        let read = test_read(b"a = { n = { n = 2 x = 3 } n = 4 } b = { n = 5 }");
        assert_eq!(keys(&read.items), ["a", "n", "b"]);
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["n", "n", "x"]);
        assert_eq!(a[0].value, Value::Scalar(b"{".to_vec()));
        assert_eq!(a[1].value, Value::Scalar(b"2".to_vec()));
        // `n = 4` is read at the top level, whose reader takes it as an
        // object that a's own `}` then ends at once.
        assert!(block(&read.items, "n").is_empty());
        assert_eq!(keys(block(&read.items, "b")), ["n"]);
        assert_eq!(read.misreads.len(), 2, "{:?}", read.misreads);
        assert!(read.misreads[0].what.starts_with("field `n` is a block"));
    }

    /// 1424A7A90 returns false for a bare token that runs into the end of
    /// the text, its text kept. A peek turns it into the end (0x13): a list
    /// (1401426E0) or a block loop stops before it, while 1424AC5E0 takes
    /// it, text and all, as a value. A quoted string at the end succeeds.
    #[test]
    fn a_last_bare_token_fails_its_advance_but_keeps_its_text() {
        let lexed = lex(b"a = { list = { x Final");
        assert!(lexed.last_fails);
        assert!(!lex(b"a = { list = { x Final\n").last_fails);
        assert!(!lex(b"a = \"Final").last_fails);
        let read = test_read(b"a = { list = { x Final");
        let list = block(block(&read.items, "a"), "list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].value, Value::Scalar(b"x".to_vec()));
        let read = test_read(b"a = { list = { x Final\n");
        assert_eq!(block(block(&read.items, "a"), "list").len(), 2);
        let read = test_read(b"a = { n = Final");
        assert_eq!(
            block(&read.items, "a")[0].value,
            Value::Scalar(b"Final".to_vec())
        );
        // A last key is never read: the loop's peek sees the end.
        let read = test_read(b"a = { n = 1 } stray");
        assert_eq!(keys(&read.items), ["a"]);
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    }

    /// 140C89020's pair: one entry, then SkipBlock (1424AD210) to the
    /// block's `}`; a second entry is reported and passed over, defining
    /// nothing; a value that is no `{` is reported.
    #[test]
    fn a_pair_reads_one_entry_and_skips_the_rest_of_its_block() {
        let pair = |text: &[u8]| {
            read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
                schema,
                field.keyword().as_deref(),
            ) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"pair")) => Shape::Pair(Test::Body),
                (_, Some(b"obj")) => Shape::Object(Test::Body),
                _ => Shape::One,
            })
        };
        let read = pair(b"a = { pair = { x = 1 } n = 2 } b = { n = 3 }");
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["pair", "n"]);
        assert_eq!(keys(block(a, "pair")), ["x"]);
        let read = pair(b"a = { pair = { x = 1 @y = 2 obj = { z = 1 } w } n = @y } b = { n = 3 }");
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["pair", "n"]);
        // SkipBlock defines nothing: `@y` keeps its text.
        assert_eq!(a[1].value, Value::Scalar(b"@y".to_vec()));
        assert!(
            read.misreads[0].what.contains("more than one entry"),
            "{:?}",
            read.misreads
        );
        let read = pair(b"a = { pair = 5 n = 2 }");
        assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
        assert!(
            read.misreads[0]
                .what
                .contains("where its reader reads `{ key = value }`")
        );
    }

    /// A reader that tests for `{` and otherwise reports (RecordError
    /// 1424ACF90) skips a block the reporting stands on.
    #[test]
    fn an_object_or_list_given_a_value_is_reported() {
        let read = read(
            b"a = { n = 1 obj { m = 1 } list = 5 n = 2 }",
            Test::Top,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"obj")) => Shape::ObjectOrSkip(Test::Body),
                (_, Some(b"list")) => Shape::ListOrSkip,
                _ => Shape::One,
            },
        );
        let a = block(&read.items, "a");
        // `obj` consumed `{` as its operator (1424AC5E0) and kept the stale
        // value `1`; the report skips the block it stands on.
        assert_eq!(keys(a), ["n", "obj", "list", "n"]);
        assert_eq!(a[3].value, Value::Scalar(b"2".to_vec()));
        assert!(
            read.misreads[1].what.contains("it is reported"),
            "{:?}",
            read.misreads
        );
    }

    /// A text ending in a definition: 1424AC5E0 reads no key, and the loop
    /// dispatches the key slot as the last read left it (1424A9730), the
    /// empty key 1424A9D30 sets when no key was read yet.
    #[test]
    fn a_text_ending_in_a_definition_dispatches_the_key_slot() {
        let read = test_read(b"@x = 1");
        assert_eq!(keys(&read.items), [""]);
        assert!(
            read.misreads[0]
                .what
                .starts_with("the text ends after a definition")
        );
        let read = test_read(b"a = { n = 2 } @y = 3");
        assert_eq!(keys(&read.items), ["a", "n"]);
    }

    /// A value without a key: 1424AC5E0 consumes the next token as its
    /// operator and dispatches the key with the previous value; a `{` after
    /// it is skipped whole for a key the reader does not know.
    #[test]
    fn a_value_without_a_key_consumes_the_next_token() {
        let read = test_read(b"a = { n = 7 stray n = 1 unknown { n = 9 } n = 2 }");
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["n", "stray", "=", "unknown", "n"]);
        // `stray` takes the stale 7; `=` then consumes `1`.
        assert_eq!(a[1].value, Value::Scalar(b"7".to_vec()));
        assert_eq!(a[4].value, Value::Scalar(b"2".to_vec()));
        assert_eq!(read.misreads.len(), 3, "{:?}", read.misreads);
        assert!(
            read.misreads[0]
                .what
                .starts_with("key `stray` has no operator")
        );
    }

    /// Definitions hold from where they are read on, at any depth read by
    /// key; a taken name leaves its `=` to the next key read; a quoted
    /// reference stays text; a definition ending its block takes `}` as the
    /// next key.
    #[test]
    fn definitions_follow_the_readers_consumption() {
        let read = test_read(
            b"@x = 1 a = { n = @x @x = 2 n = \"@x\" trigger = { @y = 3 z = 1 } n = @y } b = { n = @y @z = 4 } c = { n = 5 }",
        );
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["n", "=", "n", "trigger", "n"]);
        assert_eq!(a[0].value, Value::Scalar(b"1".to_vec()));
        assert_eq!(a[2].value, Value::Scalar(b"@x".to_vec()));
        assert!(a[2].quoted());
        assert_eq!(a[4].value, Value::Scalar(b"3".to_vec()));
        // `@z = 4` ends b's block, so `}` is read as a key consuming `c`,
        // and `=` as a key consuming `{`, whose block is skipped.
        let b = block(&read.items, "b");
        assert_eq!(keys(b), ["n", "}", "="]);
        assert!(
            read.misreads[0]
                .what
                .contains("variable `@x` is already taken")
        );
    }

    /// An object reader given a scalar reads the following fields up to
    /// the enclosing `}`; a list given a scalar the same; a skipped key's
    /// block is skipped whole.
    #[test]
    fn a_scalar_where_a_block_is_read_pulls_the_rest_in() {
        let read = test_read(b"a = { obj = 1 n = 2 } b = { n = 3 }");
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["obj", "b"]);
        assert_eq!(keys(block(a, "obj")), ["n"]);
        assert_eq!(read.misreads.len(), 1, "{:?}", read.misreads);
        let read = test_read(b"b = { list = x y } c = { n = 1 }");
        let b = block(&read.items, "b");
        assert_eq!(keys(b), ["list", "c"]);
        assert_eq!(block(b, "list")[0].value, Value::Scalar(b"y".to_vec()));
        let read = test_read(b"c = { other = { n = 3 { } } n = 4 }");
        assert_eq!(keys(block(&read.items, "c")), ["other", "n"]);
        assert!(read.misreads.is_empty());
    }

    /// A trigger block holding values without keys is read as a list; one
    /// that also holds a definition cannot be settled.
    #[test]
    fn by_key_blocks_with_values_without_keys() {
        let read = test_read(b"a = { trigger = { x y = 1 } n = 2 } b = { n = 3 }");
        assert!(read.unsettled.is_none());
        assert_eq!(keys(block(&read.items, "a")), ["trigger", "n"]);
        let read = test_read(b"a = { trigger = { x @y = 1 } n = 2 } b = { n = 3 }");
        assert!(
            read.unsettled
                .unwrap()
                .what
                .starts_with("variable @y is in a block read by a trigger")
        );
        assert_eq!(keys(&read.items), ["a"]);
    }

    /// A known trigger body (CTrigger::LoadBody 1405431C0) is never read as
    /// a list: a token without a key is a key whose missing operator
    /// consumes the next token, LoadKey takes every key with the slots
    /// 1424AC5E0 left, and a definition there is consumed, not a list's
    /// unsettled one. A scalar is reported and nothing is read. The list a
    /// reader the port does not know reads keeps each operand's kind.
    #[test]
    fn a_known_trigger_body_is_read_by_key_never_as_a_list() {
        let body = |text: &[u8]| {
            read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
                schema,
                field.keyword().as_deref(),
            ) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"body")) => Shape::Body(Test::Body),
                (_, Some(b"loose")) => Shape::Trigger,
                (_, Some(b"trigger")) => Shape::ByKey,
                _ => Shape::One,
            })
        };
        let read =
            body(b"a = { body = { x @v = 1 n = \"5\" } loose = { y = 2 } n = @v } b = { n = 3 }");
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["body", "loose", "n"]);
        let inside = block(a, "body");
        assert_eq!(keys(inside), ["x", "=", "n"]);
        // `=` is a key too, whose missing operator consumes `1`.
        assert_eq!(inside[2].kind, Kind::Quoted);
        assert!(inside[2].quoted());
        // `@v` was consumed as `x`'s operator, never defined.
        assert_eq!(a[2].value, Value::Scalar(b"@v".to_vec()));
        assert!(read.misreads[0].what.starts_with("key `x` has no operator"));
        let read = body(b"a = { body = yes n = 1 }");
        assert_eq!(keys(block(&read.items, "a")), ["body", "n"]);
        assert!(
            read.misreads[0]
                .what
                .contains("\"Non assign trigger is not enclosed in {}\" is reported"),
            "{:?}",
            read.misreads
        );
        // The same text under a reader the port does not know is a list.
        let read = body(b"a = { trigger = { x y = \"5\" } }");
        let list = block(block(&read.items, "a"), "trigger");
        assert_eq!(list[1].kind, Kind::Quoted);
        assert_eq!(list[1].value, Value::Scalar(b"5".to_vec()));
    }

    /// std_pair_parser (140C88A60): two tokens of any kind are the strings;
    /// a third is reported, a block it opens skipped, and SkipBlock passes
    /// to the pair's `}`; fewer than two are reported; nothing defines.
    #[test]
    fn a_string_pair_reads_two_tokens_and_skips_to_its_end() {
        let pair = |text: &[u8]| {
            read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
                schema,
                field.keyword().as_deref(),
            ) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"pair")) => Shape::Strings,
                _ => Shape::One,
            })
        };
        let strings = |read: &Read| {
            block(block(&read.items, "a"), "pair")
                .iter()
                .map(|item| String::from_utf8_lossy(item.value.scalar().unwrap()).into_owned())
                .collect::<Vec<_>>()
        };
        let read = pair(b"a = { pair = { \"x y\" 5 } n = 1 }");
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        assert_eq!(strings(&read), ["x y", "5"]);
        assert_eq!(block(block(&read.items, "a"), "pair")[0].kind, Kind::Quoted);
        assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);

        let read = pair(b"a = { pair = { x y { n = 9 } extra { z } } n = 1 }");
        assert_eq!(strings(&read), ["x", "y"]);
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["pair", "n"]);
        assert_eq!(a[1].value, Value::Scalar(b"1".to_vec()));
        assert!(
            read.misreads[0]
                .what
                .contains("holds `{` after its two strings")
        );

        // `{` is a string; the `}` after two strings ends the pair.
        let read = pair(b"a = { pair = { { x } n = 2 }");
        assert_eq!(strings(&read), ["{", "x"]);
        assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);

        // `k = v`: `k` and `=` are the strings; `v` is the third, and a
        // definition's name is only resolved.
        let read = pair(b"a = { pair = { @v = 1 } n = @v }");
        assert_eq!(strings(&read), ["@v", "="]);
        assert_eq!(
            block(&read.items, "a")[1].value,
            Value::Scalar(b"@v".to_vec())
        );

        let read = pair(b"a = { pair = { x } n = 1 }");
        assert_eq!(strings(&read), ["x"]);
        assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
        assert!(read.misreads[0].what.contains("holds 1 string(s)"));

        let read = pair(b"a = { pair = x n = 1 }");
        assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
        assert!(read.misreads[0].what.contains("reads `{ string string }`"));
    }

    /// LoadBody (1405431C0) hands LoadKey a key read without an operator
    /// with the value slot the last read left: a `{` consumed as the
    /// operator under a stale `{` is read by key as its block; a stale
    /// value that opens no block is RecordError's skip; a stale `{` with
    /// no `{` consumed depends on the key's reader, so it is unsettled.
    #[test]
    fn an_untraced_key_without_an_operator_follows_the_stale_value() {
        let trigger = |text: &[u8]| {
            read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
                schema,
                field.keyword().as_deref(),
            ) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"loose")) => Shape::Trigger,
                _ => Shape::One,
            })
        };
        let read = trigger(b"a = { loose = { Or { x = 1 } y = 2 } n = 3 }");
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let a = block(&read.items, "a");
        assert_eq!(keys(a), ["loose", "n"]);
        let loose = block(a, "loose");
        assert_eq!(keys(loose), ["Or", "y"]);
        assert_eq!(keys(block(loose, "Or")), ["x"]);
        assert!(
            read.misreads[0]
                .what
                .starts_with("key `Or` has no operator")
        );

        let read = trigger(b"a = { loose = { y = 2 Or { x = 1 } z = 1 } n = 3 }");
        assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
        let loose = block(block(&read.items, "a"), "loose");
        assert_eq!(keys(loose), ["y", "Or", "z"]);
        assert_eq!(loose[1].value, Value::Block(Vec::new()));

        let read = trigger(b"a = { loose = { always yes } n = 3 }");
        let unsettled = read.unsettled.unwrap();
        assert_eq!(unsettled.line, 1);
        assert!(
            unsettled.what.starts_with(
                "key `always` has no operator and its reader is handed the previous value `{`"
            ),
            "{unsettled}"
        );
    }

    /// The value slot's token kind reaches the item: the end of the text a
    /// failed advance left (0x13) keeps its text but is no keyword, and a
    /// reader of the current token (140606100) takes that token's kind.
    #[test]
    fn items_keep_the_value_tokens_kind() {
        let kinds = |text: &[u8], name: &str| {
            let read = read(text, Test::Top, &mut |schema, field: &Field<'_>| match (
                schema,
                field.keyword().as_deref(),
            ) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"current")) => Shape::Current,
                _ => Shape::One,
            });
            block(&read.items, name)
                .iter()
                .map(|item| (item.kind, item.value_keyword()))
                .collect::<Vec<_>>()
        };
        let text = b"a = { type = Ship type = \"ship\" type = 3 current = } b = { type = ship";
        assert_eq!(
            kinds(text, "a"),
            [
                (Kind::Bare, Some("ship".to_owned())),
                (Kind::Quoted, None),
                (Kind::Bare, None),
                (Kind::Close, None),
            ]
        );
        assert_eq!(kinds(text, "b"), [(Kind::End, None)]);
        assert_eq!(
            read(text, Test::Top, &mut |schema, _| match schema {
                Test::Top => Shape::Object(Test::Body),
                Test::Body => Shape::One,
            })
            .items[1]
                .value
                .items()
                .unwrap()[0]
                .value,
            Value::Scalar(b"ship".to_vec())
        );
    }

    /// 1424AC5E0 keeps the operator it read (+0x78), `<` 0x1D4 and `>`
    /// 0x1D3 beside `=`; a key read without one keeps the slot's last.
    #[test]
    fn items_keep_the_operator_slot() {
        let read = test_read(b"a = { n < 5 stray n n > 3 n = 1 } b = { n >= 2 }");
        let a = block(&read.items, "a");
        let operators: Vec<_> = a.iter().map(|item| item.operator).collect();
        assert_eq!(keys(a), ["n", "stray", "n", "n"]);
        assert_eq!(
            operators,
            [
                Operator::Less,
                Operator::Less,
                Operator::Greater,
                Operator::Equals
            ]
        );
        // `>=` is `>` and then the value `=`.
        let b = block(&read.items, "b");
        assert_eq!(b[0].operator, Operator::Greater);
        assert_eq!(b[0].value, Value::Scalar(b"=".to_vec()));
    }

    /// 1424A7A90 makes `>` and `<` tokens of their own, and 1424AC5E0 writes
    /// each field's operator into the one slot (+0x78). A block's item keeps
    /// the `=` read with its key, not the last comparison inside it, whether
    /// the block is read as an object or by key unread ([`Shape::ByKey`]).
    #[test]
    fn a_block_keeps_its_own_operator_after_comparisons_inside() {
        let read = read(
            b"t = { Or = { phase > 99 } n = 1 AND = { x < 2 obj = { y > 3 } } trigger = { OR = { phase > 99 } } }",
            Test::Top,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) | (_, Some(b"obj")) => Shape::Object(Test::Body),
                (_, Some(b"or" | b"and")) => Shape::Object(Test::Body),
                (_, Some(b"trigger")) => Shape::ByKey,
                _ => Shape::One,
            },
        );
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        let t = block(&read.items, "t");
        let operators = |items: &[Item]| -> Vec<_> {
            items
                .iter()
                .map(|item| (keys(std::slice::from_ref(item)).remove(0), item.operator))
                .collect()
        };
        let keyed = |key: &str, operator| (key.to_owned(), operator);
        assert_eq!(
            operators(t),
            [
                keyed("Or", Operator::Equals),
                keyed("n", Operator::Equals),
                keyed("AND", Operator::Equals),
                keyed("trigger", Operator::Equals),
            ]
        );
        assert_eq!(
            operators(block(t, "Or")),
            [keyed("phase", Operator::Greater)]
        );
        let and = block(t, "AND");
        assert_eq!(
            operators(and),
            [keyed("x", Operator::Less), keyed("obj", Operator::Equals)]
        );
        assert_eq!(
            operators(block(and, "obj")),
            [keyed("y", Operator::Greater)]
        );
        let trigger = block(t, "trigger");
        assert_eq!(operators(trigger), [keyed("OR", Operator::Equals)]);
        assert_eq!(
            operators(block(trigger, "OR")),
            [keyed("phase", Operator::Greater)]
        );
    }

    /// 1424AC830 stores the whole defining token and 1424AB540 copies it
    /// back, so a reference keeps its definition's kind (a quoted one is no
    /// keyword); a chain takes the value it names, whose token is then read
    /// as the next key, a taken name; names compare exactly (`@N` is not
    /// `@n`); a quoted token or an unknown name stays its own text.
    #[test]
    fn a_reference_keeps_its_definitions_token_kind() {
        let read = test_read(
            b"@quoted = \"center\" @bare = center @chain = @quoted @N = 3 \
              a = { n = @quoted n = @bare n = @chain n = \"@bare\" n = @none n = @n }",
        );
        let a = block(&read.items, "a");
        let values: Vec<_> = a
            .iter()
            .map(|item| (item.value.scalar().unwrap().to_vec(), item.quoted()))
            .collect();
        assert_eq!(
            values,
            [
                (b"center".to_vec(), true),
                (b"center".to_vec(), false),
                (b"center".to_vec(), true),
                (b"@bare".to_vec(), true),
                (b"@none".to_vec(), false),
                (b"@n".to_vec(), false),
            ]
        );
        assert!(
            read.misreads[0]
                .what
                .contains("definition `@chain` takes `@quoted`")
        );
        assert!(
            read.misreads[1]
                .what
                .contains("variable `@quoted` is already taken")
        );
    }
}
