
use content_reader_parser::parser::{Item, Operator, Value};
pub use content_reader_parser::parser::Kind;
use crate::key_names::KeyNames;

mod math;
mod recover;
#[cfg(test)]
use recover::ExploredNames;
pub use recover::{NamePolicy, RecoveryShape, recover_declared_names};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub text: Vec<u8>,
    pub line: u32,
    pub quoted: bool,
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

    fn shown_reference(&self) -> String {
        if self.kind == Kind::End && !self.text.is_empty() {
            format!("`{}`", String::from_utf8_lossy(&self.text))
        } else {
            self.shown()
        }
    }

    pub fn quoted(&self) -> bool {
        self.quoted
    }

    pub fn is_end(&self) -> bool {
        self.kind == Kind::End
    }

    pub fn ends_block(&self) -> bool {
        matches!(self.kind, Kind::Close | Kind::End)
    }

    pub fn starts_with_at(&self) -> bool {
        self.text.first() == Some(&b'@')
    }

    pub fn c_str(&self) -> &[u8] {
        content_reader_parser::parser::c_string(&self.text)
    }

    pub fn is_number(&self) -> bool {
        self.kind == Kind::Bare && content_reader_parser::parser::number_token(&self.text, false)
    }

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

    pub fn is_keyword(&self, word: &str) -> bool {
        self.lookup()
            .is_some_and(|text| text.eq_ignore_ascii_case(word.as_bytes()))
    }
}

fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    KeepMark,
    SkipMark,
}

fn next(bytes: &[u8], at: &mut usize) -> Option<u8> {
    let byte = bytes.get(*at).copied();
    *at += 1;
    byte.filter(|byte| *byte != 0xFF)
}

pub fn lex(bytes: &[u8], mode: Mode) -> Lexed<'_> {
    lex_with(bytes, mode, false)
}

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
    let mut kept = Vec::new();
    loop {
        if at >= bytes.len() {
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
                        text: content_reader_parser::parser::c_string(&bytes[from..end]).to_vec(),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape<S> {
    One,
    Current,
    Object(S),
    ObjectAs(S, Vec<u8>),
    ObjectIfOpen(S),
    ObjectOrSkip(S),
    Twice(S),
    List,
    BoundedList(usize),
    ListIfOpen,
    Tokens,
    ListOrSkip,
    Pair(S),
    Strings,
    Skip,
    Discard,
    SkipIfOpen,
    Body(S),
    Trigger,
    MtthModifier,
    Effect,
    EffectIn(u32),
    EffectValueKeys(u64),
    EffectKeys(u32),
    EffectByKey { trigger_limits: bool },
    Reported {
        kind: content_reader_parser::DiagnosticKind,
        report: String,
        read: Box<Shape<S>>,
    },
    ByKey,
    Break,
    SkipBreak,
    StopFile,
    Color,
    Statements(S),
    RawList,
    RawListFirst,
    TemplatePoint,
    ValueList,
    ObjectList(S),
    CheckedList { stores_value: bool },
    RawSkip,
    RawText,
    Expression,
    LimitedBody(S),
    ScriptedKey,
    TechnologyEquipment,
    Unsettled(String),
}

impl<S> Shape<S> {
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

pub fn mean_time_to_happen<S>(field: &Field<'_>) -> Shape<S> {
    match field.keyword().as_deref() {
        Some(b"modifier") => Shape::MtthModifier,
        _ => Shape::One,
    }
}

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

#[derive(Debug)]
pub struct Field<'a> {
    pub key: &'a Token,
    pub value: &'a Token,
    pub fresh: bool,
    pub operator: Operator,
    pub current: &'a Token,
    pub effect_scope: Option<u32>,
}

impl Field<'_> {
    pub fn keyword(&self) -> Option<Vec<u8>> {
        let key = self.key.c_str();
        content_reader_parser::parser::looked_up(key, self.key.kind == Kind::Quoted).then(|| key.to_ascii_lowercase())
    }

    pub fn keyword_is(&self, name: &str) -> bool {
        self.keyword()
            .is_some_and(|keyword| keyword == name.as_bytes())
    }

    pub fn value_keyword(&self) -> Option<Vec<u8>> {
        let value = self.value.c_str();
        (self.value.kind == Kind::Bare && content_reader_parser::parser::looked_up(value, false))
            .then(|| value.to_ascii_lowercase())
    }

    pub fn opens(&self) -> bool {
        self.value.kind == Kind::Open
    }

    pub fn skips_block(&self) -> bool {
        self.current.kind == Kind::Open
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misread {
    pub line: u32,
    pub what: String,
    pub kind: content_reader_parser::DiagnosticKind,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub items: Vec<Item>,
    pub misreads: Vec<Misread>,
    pub item_misreads: Vec<Vec<Misread>>,
    pub unattributed_misreads: Vec<Misread>,
    pub unsettled: Option<Misread>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    Fresh,
    NoOperator,
    Ended,
}

struct Variable {
    hash: u32,
    value: Token,
    unknown: Option<(usize, u32, Vec<u8>)>,
}

pub struct Reader<'n> {
    tokens: Vec<Token>,
    source: Option<TextSource<'n>>,
    ends: Vec<usize>,
    next: usize,
    pub current: Token,
    pending: bool,
    pending_result: bool,
    pub key: Token,
    operator: Operator,
    pub value: Token,
    variables: Vec<Variable>,
    unknown_scopes: Vec<(usize, Vec<u8>)>,
    next_unknown_scope: usize,
    misreads: Vec<Misread>,
    reading_notes: Vec<Misread>,
    item_misreads: Vec<Vec<Misread>>,
    unattributed_misreads: Vec<Misread>,
    unsettled: Option<Misread>,
    broken: bool,
    names: Option<&'n KeyNames>,
    stopped: bool,
    effect_scope: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub enum Top<S> {
    File(S),
    Persistent(S),
}

pub fn read<S: Copy>(
    bytes: &[u8],
    mode: Mode,
    top: Top<S>,
    names: Option<&KeyNames>,
    dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
) -> Read {
    read_scoped(bytes, mode, top, names, None, dispatch)
}

pub fn read_in_effect_scope<S: Copy>(
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
    content_reader_parser::parse_counts::parse(std::any::type_name::<S>(), bytes);
    let mut reader = Reader::new(lex(bytes, mode));
    reader.names = names;
    reader.effect_scope = scope;
    let items = match top {
        Top::File(node) => reader.file(node, dispatch),
        Top::Persistent(node) => reader.object_with_notes(node, dispatch, true),
    };
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

impl<'n> Reader<'n> {
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
            key: Token::slot(),
            operator: Operator::Equals,
            value: Token::slot(),
            variables: Vec::new(),
            unknown_scopes: Vec::new(),
            next_unknown_scope: 0,
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

    pub fn note_native_error(&mut self, line: u32, what: String) {
        self.misread(line, what);
    }

    pub fn refuse_native(&mut self, line: u32, what: String) {
        self.unsettled = Some(Misread {
            kind: content_reader_parser::DiagnosticKind::Unsettled,
            count: None,
            line,
            what,
        });
    }

    pub fn is_unsettled(&self) -> bool {
        self.unsettled.is_some()
    }

    fn misread(&mut self, line: u32, what: String) {
        self.misread_at(line, what, false);
    }

    fn variable_misread(&mut self, line: u32, what: String) {
        self.misread_at(line, what, true);
    }

    fn misread_at(&mut self, line: u32, what: String, file_scoped: bool) {
        let note = Misread {
            kind: content_reader_parser::DiagnosticKind::Misread,
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

    fn advance_raw(&mut self) -> bool {
        let Some(token) = self.tokens.get(self.next) else {
            return false;
        };
        self.next += 1;
        if token.kind != Kind::End {
            self.current = token.clone();
            return true;
        }
        if !token.text.is_empty() {
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

    pub fn advance(&mut self) -> bool {
        let more = if self.pending {
            self.pending = false;
            self.pending_result
        } else {
            self.advance_raw()
        };
        if !more && !self.unknown_scopes.is_empty() && self.has_tokens_after_end() {
            let line = self
                .tokens
                .get(self.next.saturating_sub(1))
                .map_or(self.current.line, |token| token.line);
            self.refuse_embedded_end(line);
        }
        more
    }

    pub fn peek(&mut self) -> &Token {
        if !self.pending {
            let result = self.advance_raw();
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

    fn active_unknown(&self, origin: Option<&(usize, u32, Vec<u8>)>) -> Option<(u32, Vec<u8>)> {
        let (scope, line, block) = origin?;
        (!self.unknown_scopes.iter().any(|(active, _)| active == scope))
            .then(|| (*line, block.clone()))
    }

    pub fn resolve(&mut self, token: Token) -> Token {
        self.resolve_token(token, false)
    }

    fn resolve_in_skip(&mut self, token: Token) -> Token {
        self.resolve_token(token, true)
    }

    fn resolve_token(&mut self, token: Token, skip: bool) -> Token {
        if token.quoted || token.kind == Kind::Code || !token.text.starts_with(b"@") {
            return token;
        }
        let hash = content_reader_parser::parser::variable_hash(&token.text[1..]);
        let found = self.variables.iter().find(|defined| defined.hash == hash);
        match found {
            Some(defined) => {
                let resolved = Token {
                    line: token.line,
                    ..defined.value.clone()
                };
                let ambiguous = (!skip || defined.value.kind == Kind::Open)
                    .then(|| self.active_unknown(defined.unknown.as_ref()))
                    .flatten();
                if let Some((defined_line, block)) = ambiguous
                    && self.unsettled.is_none()
                {
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
                        count: None,
                        line: token.line,
                        what: format!(
                            "variable {} was defined on line {defined_line} inside block `{}` under unknown dispatch, so whether it was reached is unknown and this value cannot be resolved",
                            token.shown_reference(),
                            String::from_utf8_lossy(&block),
                        ),
                    });
                }
                resolved
            }
            None => token,
        }
    }

    fn define(&mut self) {
        let name = self.current();
        let hash = content_reader_parser::parser::variable_hash(&name.text[1..]);
        if let Some(existing) = self.variables.iter().find(|defined| defined.hash == hash) {
            if let Some((defined_line, block)) = self.active_unknown(existing.unknown.as_ref()) {
                if self.unsettled.is_none() {
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
                        count: None,
                        line: name.line,
                        what: format!(
                            "variable {} was defined on line {defined_line} inside block `{}` under unknown dispatch, so whether it is already taken here is unknown",
                            name.shown_reference(),
                            String::from_utf8_lossy(&block),
                        ),
                    });
                }
            } else {
                self.variable_misread(
                    name.line,
                    format!(
                        "variable {} is already taken, so its `=` and value are read as what follows",
                        name.shown_reference()
                    ),
                );
            }
            return;
        }
        if !self.unknown_scopes.is_empty() {
            let (peek_kind, peek_line) = {
                let peek = self.peek();
                (peek.kind, peek.line)
            };
            if peek_kind == Kind::Close
                && let Some((_, block)) = self.unknown_scopes.last()
            {
                if self.unsettled.is_none() {
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
                        count: None,
                        line: peek_line,
                        what: format!(
                            "definition {} ends block `{}` without `=` under unknown dispatch: a key reader consumes `}}` while a skip or list reader closes the block here",
                            name.shown_reference(),
                            String::from_utf8_lossy(block),
                        ),
                    });
                }
                return;
            }
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
                    name.shown_reference(),
                    token.shown()
                ),
            );
            return;
        }
        let peeked = self.peek().clone();
        if peeked.kind == Kind::Close
            && let Some((_, block)) = self.unknown_scopes.last()
        {
            if self.unsettled.is_none() {
                self.unsettled = Some(Misread {
                    kind: content_reader_parser::DiagnosticKind::Unsettled,
                    count: None,
                    line: peeked.line,
                    what: format!(
                        "definition {} has no value before `}}` in block `{}` under unknown dispatch: a key reader consumes `}}` as its value while a skip or list reader closes the block here",
                        name.shown_reference(),
                        String::from_utf8_lossy(block),
                    ),
                });
            }
            return;
        }
        let value = match peeked {
            token if token.text.starts_with(b"@") => {
                let resolved = self.resolve(token.clone());
                if self.unsettled.is_some() {
                    return;
                }
                self.variable_misread(
                    name.line,
                    format!(
                        "definition {} takes {}, which the variable reader leaves unread, so the next key read starts from it",
                        name.shown_reference(),
                        token.shown_reference()
                    ),
                );
                resolved
            }
            _ => {
                self.advance();
                if self.unsettled.is_some() {
                    return;
                }
                let token = self.current();
                if token.kind == Kind::Open {
                    self.variable_misread(
                        name.line,
                        format!(
                            "definition {} keeps only the `{{` of its block value; the block's contents are read as what follows",
                            name.shown_reference()
                        ),
                    );
                }
                token
            }
        };
        let unknown = self
            .unknown_scopes
            .last()
            .map(|(scope, block)| (*scope, name.line, block.clone()));
        if value.kind == Kind::Open
            && let Some((_, _, block)) = &unknown
            && self.unsettled.is_none()
        {
            self.unsettled = Some(Misread {
                kind: content_reader_parser::DiagnosticKind::Unsettled,
                count: None,
                line: name.line,
                what: format!(
                    "definition {} keeps `{{` inside block `{}` under unknown dispatch, so whether its closing brace ends the enclosing block is unknown",
                    name.shown_reference(),
                    String::from_utf8_lossy(block),
                ),
            });
        }
        self.variables.push(Variable {
            hash,
            value,
            unknown,
        });
    }

    pub fn key_operator_value(&mut self) -> Slots {
        loop {
            if !self.advance() {
                return Slots::Ended;
            }
            if self.current().starts_with_at() {
                let defined_name = self.current();
                self.define();
                if self.unsettled.is_some() {
                    return Slots::Ended;
                }
                if !self.unknown_scopes.is_empty() {
                    let (peek_kind, line) = {
                        let peek = self.peek();
                        (peek.kind, peek.line)
                    };
                    if peek_kind == Kind::Close
                        && let Some((_, block)) = self.unknown_scopes.last()
                    {
                        self.unsettled = Some(Misread {
                            kind: content_reader_parser::DiagnosticKind::Unsettled,
                            count: None,
                            line,
                            what: format!(
                                "variable {} ends block `{}` under unknown dispatch: a key reader consumes `}}` as a key while a skip or list reader closes the block here",
                                defined_name.shown_reference(),
                                String::from_utf8_lossy(block),
                            ),
                        });
                        return Slots::Ended;
                    }
                }
                continue;
            }
            self.key = self.current();
            if !self.unknown_scopes.is_empty() {
                let (peek_kind, line) = {
                    let peek = self.peek();
                    (peek.kind, peek.line)
                };
                if peek_kind == Kind::Close
                    && let Some((_, block)) = self.unknown_scopes.last()
                {
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
                        count: None,
                        line,
                        what: format!(
                            "key {} ends block `{}` without an operator under unknown dispatch: a key reader consumes `}}` as its operator while a skip or list reader closes the block here",
                            self.key.shown(),
                            String::from_utf8_lossy(block),
                        ),
                    });
                    return Slots::Ended;
                }
            }
            if !self.advance() || !self.current().operator() {
                return Slots::NoOperator;
            }
            self.operator = self.current().as_operator();
            let (peek_kind, peek_line) = {
                let peek = self.peek();
                (peek.kind, peek.line)
            };
            self.value = if peek_kind == Kind::Close {
                Token::end(peek_line)
            } else {
                self.advance();
                if self.unsettled.is_some() {
                    return Slots::Ended;
                }
                let resolved = self.resolve(self.current());
                if self.unsettled.is_some() {
                    return Slots::Ended;
                }
                resolved
            };
            return Slots::Fresh;
        }
    }

    fn file<S: Copy>(
        &mut self,
        top: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while !self.stopped && self.unsettled.is_none() && self.peek().kind != Kind::End {
            let item = self.field(top, dispatch);
            self.keep_item_with_notes(item, &mut items);
            self.broken = false;
        }
        items
    }

    fn keep_item_with_notes(&mut self, item: Option<Item>, items: &mut Vec<Item>) {
        let notes = std::mem::take(&mut self.reading_notes);
        if let Some(item) = item {
            items.push(item);
            self.item_misreads.push(notes);
        } else {
            self.unattributed_misreads.extend(notes);
        }
    }

    fn object<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        self.object_with_notes(schema, dispatch, false)
    }

    fn object_with_notes<S: Copy>(
        &mut self,
        schema: S,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
        attribute_items: bool,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            if self.at_loop_end() {
                break;
            }
            let item = self.field(schema, dispatch);
            let broken = std::mem::take(&mut self.broken);
            if broken {
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
            }
            if attribute_items {
                self.keep_item_with_notes(item, &mut items);
            } else if let Some(item) = item {
                items.push(item);
            }
            if broken {
                break;
            }
        }
        items
    }

    fn at_loop_end(&mut self) -> bool {
        if !self.peek().ends_block() {
            return false;
        }
        self.advance();
        self.current = self.resolve(self.current());
        true
    }

    pub fn next_key(&mut self) -> bool {
        if self.at_loop_end() {
            return false;
        }
        self.key_operator_value();
        true
    }

    pub fn at_end(&self) -> bool {
        if self.pending && self.pending_result {
            return false;
        }
        self.tokens[self.next.min(self.tokens.len())..]
            .iter()
            .all(|token| token.kind == Kind::End)
    }

    pub fn skip_current_block(&mut self) {
        if self.value.kind == Kind::Open {
            self.skip_block();
        }
    }

    pub fn skip_reported(&mut self) {
        if self.current.kind == Kind::Open {
            self.skip();
        }
    }

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
        let operator = self.operator;
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
                let mut shaper = crate::trigger_grammar::Shaper::new(self.names);
                let node = if matches!(shape, Shape::MtthModifier) {
                    crate::trigger_grammar::Node::Modifier
                } else {
                    crate::trigger_grammar::Node::Trigger
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
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
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
                    let meta = content_reader_parser::parser::looked_up(key.c_str(), key.kind == Kind::Quoted)
                        && matches!(
                            key.c_str().to_ascii_lowercase().as_slice(),
                            b"meta_effect" | b"meta_trigger"
                        );
                    self.next_unknown_scope += 1;
                    let scope = self.next_unknown_scope;
                    self.unknown_scopes.push((scope, key.c_str().to_vec()));
                    let items = self.generic(meta);
                    self.unknown_scopes.pop();
                    Value::Block(items)
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
                    kind: content_reader_parser::DiagnosticKind::Unsettled,
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
            if self.unsettled.is_some() {
                break;
            }
            self.current = first.clone();
            let assigned = first.kind != Kind::Open && self.peek().operator();
            let (key, value) = if assigned {
                self.key = first.clone();
                self.advance();
                self.operator = self.resolve(self.current()).as_operator();
                if self.unsettled.is_some() {
                    break;
                }
                self.advance();
                let value = self.resolve(self.current());
                if self.unsettled.is_some() {
                    break;
                }
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
                kind: content_reader_parser::DiagnosticKind::Unsettled,
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
                    kind: content_reader_parser::DiagnosticKind::Unsettled,
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
            self.misread(
                self.key.line,
                "raw meta text has unmatched braces or brackets and is reported".into(),
            );
            self.skip_reported();
        }
        text
    }

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
            if stop || !advanced || self.unsettled.is_some() {
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

    fn raw_list_first(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            self.advance();
            self.current = self.resolve(self.current());
            if self.unsettled.is_some() {
                return items;
            }
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
        items
    }

    fn raw_list(&mut self, equipment: bool, stores_value: bool, skip_open: bool) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            let advanced = self.advance();
            self.current = self.resolve(self.current());
            if stop || !advanced || self.unsettled.is_some() {
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
            if equipment && token.is_keyword("limit") {
                self.advance();
                self.current = self.resolve(self.current());
                if self.unsettled.is_some() {
                    break;
                }
                self.advance();
                self.current = self.resolve(self.current());
                if self.unsettled.is_some() {
                    break;
                }
                let value = self.current();
                if value.kind != Kind::Open {
                    self.unsettled = Some(Misread {
                        kind: content_reader_parser::DiagnosticKind::Unsettled,
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
                if self.unsettled.is_some() {
                    break;
                }
                self.advance();
                self.current = self.resolve(self.current());
                if self.unsettled.is_some() {
                    break;
                }
                token = self.current();
                if matches!(token.kind, Kind::Open | Kind::Close | Kind::End)
                    || self.peek().kind != Kind::Close
                {
                    self.unsettled = Some(Misread { kind: content_reader_parser::DiagnosticKind::Unsettled, count: None, line: token.line, what: "technology enable_equipments.limit: unsupported conditional tail (requires one equipment token)".into() });
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

    fn limited_body<S: Copy>(
        &mut self,
        schema: S,
        key: &Token,
        dispatch: &mut dyn FnMut(S, &Field<'_>) -> Shape<S>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let next = self.peek();
        if next.is_keyword("limit") {
            items.extend(self.field(schema, &mut |_, _| Shape::Trigger));
        }
        if self.value.kind == Kind::Open {
            items.extend(self.object(schema, dispatch));
        } else if self.current.kind == Kind::Open {
            self.unsettled = Some(Misread {
                kind: content_reader_parser::DiagnosticKind::Unsettled,
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
            if self.unsettled.is_some() {
                break;
            }
            self.current = key.clone();
            if key.kind == Kind::Open || !self.peek().operator() {
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
            if self.unsettled.is_some() {
                break;
            }
            self.operator = operator;
            self.advance();
            let value = self.resolve(self.current());
            if self.unsettled.is_some() {
                break;
            }
            self.current = value.clone();
            self.value = value.clone();
            let shape = if content_reader_parser::parser::read_bool(&value.text) == Some(true) {
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

    fn strings(&mut self, key: &Token) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
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
            if self.unsettled.is_some() {
                return items;
            }
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
        items
    }

    pub fn skip_block(&mut self) {
        let mut depth = 0_u32;
        loop {
            if !self.advance() {
                return;
            }
            match self.current.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return,
                Kind::Close => depth -= 1,
                _ => {}
            }
        }
    }

    fn color(&mut self, key: &Token) -> Value {
        let current = self.current();
        if current.kind == Kind::Bare && content_reader_parser::parser::number_token(&current.text, false) {
            if matches!(
                content_reader_parser::crt::scan_x32(&current.text),
                content_reader_parser::crt::Scanned::NoMatch
            ) {
                self.misread(
                    key.line,
                    format!(
                        "field {} has {} where the colour reader reads a packed hexadecimal colour: it is reported and the previous colour is kept",
                        key.shown(),
                        current.shown()
                    ),
                );
            }
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
            if self.unsettled.is_some() {
                return Value::Block(components);
            }
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
            if !(token.kind == Kind::Bare && content_reader_parser::parser::number_token(&token.text, false)) {
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

    pub fn list(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            if stop {
                return items;
            }
            let token = self.resolve(self.current());
            if self.unsettled.is_some() {
                return items;
            }
            items.push(Item {
                key: None,
                key_quoted: false,
                operator: Operator::Equals,
                kind: token.kind,
                value: Value::Scalar(token.text.clone()),
            });
            self.value = token;
        }
        items
    }

    fn bounded_list(&mut self, n: usize) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let stop = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            if stop {
                return items;
            }
            let token = self.resolve(self.current());
            if self.unsettled.is_some() {
                return items;
            }
            self.current = token.clone();
            self.value = token.clone();
            if items.len() == n && self.current.kind == Kind::Open {
                self.skip();
                if self.unsettled.is_some() {
                    return items;
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

    fn tokens(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        let mut depth = 1;
        while self.unsettled.is_none() && self.peek().kind != Kind::End && depth >= 1 {
            self.advance();
            let token = self.resolve(self.current());
            if self.unsettled.is_some() {
                return items;
            }
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

    pub fn skip(&mut self) {
        while self.unsettled.is_none() {
            let end = matches!(self.peek().kind, Kind::Close | Kind::End);
            self.advance();
            self.current = self.resolve_in_skip(self.current());
            if end || self.unsettled.is_some() {
                return;
            }
            if self.current.kind == Kind::Open {
                self.skip();
            }
        }
    }

    fn keyless_list(&mut self) -> Option<Vec<Item>> {
        let line = self.line();
        let (keyless, definition) = self.scan();
        if !keyless {
            return None;
        }
        if let Some(variable) = definition {
            self.unsettled = Some(Misread {
                kind: content_reader_parser::DiagnosticKind::Unsettled,
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
            if trigger_limits && field.keyword_is("limit") {
                return Shape::Trigger;
            }
            untraced(field)
        };
        self.object(meta, &mut dispatch)
    }

    fn scan(&self) -> (bool, Option<String>) {
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
                    let (next_at, nested_def) = scan_block(tokens, at);
                    if definition.is_none() {
                        definition = nested_def;
                    }
                    at = next_at;
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
            if token.starts_with_at() && definition.is_none() {
                definition = Some(String::from_utf8_lossy(&token.text).into_owned());
            }
            at += 2;
            match tokens.get(at).map(|value| value.kind) {
                Some(Kind::Open) => {
                    let (next_at, nested_def) = scan_block(tokens, at);
                    if definition.is_none() {
                        definition = nested_def;
                    }
                    at = next_at;
                }
                Some(Kind::Close) | None => break,
                Some(_) => at += 1,
            }
        }
        (keyless, definition)
    }

    fn balanced(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while self.unsettled.is_none() {
            let raw_token = self.peek().clone();
            self.advance();
            if raw_token.kind == Kind::End {
                let line = raw_token.line;
                self.current = self.resolve(raw_token);
                if self.unsettled.is_none() && self.has_tokens_after_end() {
                    self.refuse_embedded_end(line);
                }
                return items;
            }
            match raw_token.kind {
                Kind::Close => return items,
                Kind::Open => items.push(Item {
                    key: None,
                    key_quoted: false,
                    operator: Operator::Equals,
                    kind: Kind::Open,
                    value: Value::Block(self.balanced()),
                }),
                _ => {
                    let token = self.resolve(raw_token.clone());
                    if self.unsettled.is_some() {
                        return items;
                    }
                    if matches!(token.kind, Kind::Open | Kind::Close) {
                        self.refuse_balanced_brace(&raw_token, token.kind);
                        return items;
                    }
                    let operator = self.peek().operator();
                    if operator {
                        self.advance();
                        let operator = self.current().as_operator();
                        let (peek_kind, peek_line) = {
                            let peek = self.peek();
                            (peek.kind, peek.line)
                        };
                        if peek_kind == Kind::Close {
                            if self.unsettled.is_none() {
                                let what = match self.unknown_scopes.last() {
                                    Some((_, block)) => format!(
                                        "key {} ends block `{}` without a value under unknown dispatch: a key reader leaves `}}` unread while a list reader reads the operator as an entry",
                                        token.shown(),
                                        String::from_utf8_lossy(block),
                                    ),
                                    None => format!(
                                        "key {} ends a block without a value under unknown dispatch: a key reader leaves `}}` unread while a list reader reads the operator as an entry",
                                        token.shown(),
                                    ),
                                };
                                self.unsettled = Some(Misread {
                                    kind: content_reader_parser::DiagnosticKind::Unsettled,
                                    count: None,
                                    line: peek_line,
                                    what,
                                });
                            }
                            return items;
                        }
                        self.advance();
                        let raw_value = self.current();
                        let value = self.resolve(raw_value.clone());
                        if self.unsettled.is_some() {
                            return items;
                        }
                        if raw_value.kind == Kind::End && self.has_tokens_after_end() {
                            self.refuse_embedded_end(raw_value.line);
                            return items;
                        }
                        if raw_value.kind != value.kind
                            && matches!(value.kind, Kind::Open | Kind::Close)
                        {
                            self.refuse_balanced_brace(&raw_value, value.kind);
                            return items;
                        }
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
        items
    }

    fn has_tokens_after_end(&self) -> bool {
        self.tokens[self.next.min(self.tokens.len())..]
            .iter()
            .any(|token| token.kind != Kind::End || !token.text.is_empty())
    }

    fn refuse_embedded_end(&mut self, line: u32) {
        if self.unsettled.is_some() {
            return;
        }
        let what = match self.unknown_scopes.last() {
            Some((_, block)) => format!(
                "an embedded end marker inside block `{}` under unknown dispatch stops a list or skip reader while a key reader can resume after it, so what follows is unknown",
                String::from_utf8_lossy(block),
            ),
            None => "an embedded end marker inside a block under unknown dispatch stops a list or skip reader while a key reader can resume after it, so what follows is unknown".into(),
        };
        self.unsettled = Some(Misread {
            kind: content_reader_parser::DiagnosticKind::Unsettled,
            count: None,
            line,
            what,
        });
    }

    fn refuse_balanced_brace(&mut self, reference: &Token, resolved_kind: Kind) {
        if self.unsettled.is_some() {
            return;
        }
        let brace = if resolved_kind == Kind::Open {
            "`{`"
        } else {
            "`}`"
        };
        let what = match self.unknown_scopes.last() {
            Some((_, block)) => format!(
                "variable {} resolves to {brace} inside block `{}` under unknown dispatch, so whether it shifts the block boundary is unknown",
                reference.shown_reference(),
                String::from_utf8_lossy(block),
            ),
            None => format!(
                "variable {} resolves to {brace} inside a block under unknown dispatch, so whether it shifts the block boundary is unknown",
                reference.shown_reference(),
            ),
        };
        self.unsettled = Some(Misread {
            kind: content_reader_parser::DiagnosticKind::Unsettled,
            count: None,
            line: reference.line,
            what,
        });
    }
}

fn scan_block(tokens: &[Token], mut at: usize) -> (usize, Option<String>) {
    let mut depth = 0_usize;
    let mut definition = None;
    while let Some(token) = tokens.get(at) {
        if depth > 0
            && definition.is_none()
            && token.starts_with_at()
            && tokens.get(at + 1).is_some_and(Token::operator)
        {
            definition = Some(String::from_utf8_lossy(&token.text).into_owned());
        }
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
    (at, definition)
}

#[cfg(test)]
mod tests;
