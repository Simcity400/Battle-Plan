//! The GUI, sprite, font, effect and settings readers' walk over one shared token reader,
//! [`crate::parser::native::Reader`]: its lexer, key/operator/value slots, script variables,
//! persistent loop and skips. This module keeps the adjacent state (refusals, labels,
//! generated-name counter and dynamic tokens) and the point, position, size, margin and
//! component sub-readers.
//!
//! Every loop is iterative: nesting follows the text without a chosen depth bound.

use super::{Cause, Coordinate, Margin, Position, Refusal, Size};
use crate::{
    crt::{self, Scanned},
    parser::{
        native::{self, Kind as TokenKind, Token},
        read_bool,
    },
};

/// An element's label in refusal paths, known once its block is read.
pub(crate) struct Label {
    pub parent: Option<usize>,
    pub text: String,
}

/// The file's own level, labelled with an empty path.
pub(super) const FILE: usize = 0;

pub(super) fn record(
    refusals: &mut Vec<Refusal>,
    node: usize,
    key: &str,
    reason: String,
    cause: Cause,
) {
    let refusal = Refusal {
        path: String::new(),
        key: key.to_owned(),
        reason,
        cause,
        node,
    };
    if !refusals.contains(&refusal) {
        refusals.push(refusal);
    }
}

/// The token's text up to its first NUL, or the unreadable-string marker when it holds a line
/// feed.
pub(crate) fn read_string(text: &[u8]) -> Vec<u8> {
    crate::parser::read_string(text).to_vec()
}

/// The static keyword of a key, by its lowered registered spelling. An unquoted word is looked
/// up through [`Token::lookup`] in [`super::keywords`]. Only registered ids dispatch; other
/// words (dynamic tokens or strings, 0xF) are `None`. Readers distinguishing those forms use
/// [`super::keywords::class`].
pub(crate) fn keyword_of(token: &Token) -> Option<&'static str> {
    token.lookup().and_then(super::keywords::static_keyword)
}

/// A token as a refusal names it: the end, a quoted string in quotes, a word as it is, and
/// punctuation in single quotes.
pub(crate) fn display(token: &Token) -> String {
    let text = String::from_utf8_lossy(&token.text);
    match token.kind {
        TokenKind::End => "the end of the file".into(),
        TokenKind::Quoted | TokenKind::Code => format!("\"{text}\""),
        TokenKind::Bare => text.into_owned(),
        _ => format!("'{text}'"),
    }
}

pub(crate) struct Reader<'s> {
    /// The shared token reader this walk drives: its `key`, `value` and `current` slots are
    /// accessed through [`Deref`], so there is one owner of token consumption.
    native: native::Reader<'s>,
    /// Set once this reader cannot follow the game's walk because the game itself does not
    /// (an undefined read that decides where it reads on, a loop that never ends): every element
    /// ending after it is refused with it.
    pub poison: Option<Refusal>,
    pub labels: Vec<Label>,
    /// The refusals of the file's own level.
    pub file: Vec<Refusal>,
    /// The generated-name counter as this file reads on.
    pub unnamed: i32,
    /// The lexer's dynamic tokens while this file is read ([`super::keywords`]).
    pub dynamic: super::keywords::DynamicTokens,
    /// What the last [`Self::read`] returned (the read step's value), kept apart from the token.
    pub read_ok: bool,
}

impl<'s> std::ops::Deref for Reader<'s> {
    type Target = native::Reader<'s>;

    fn deref(&self) -> &native::Reader<'s> {
        &self.native
    }
}

impl<'s> std::ops::DerefMut for Reader<'s> {
    fn deref_mut(&mut self) -> &mut native::Reader<'s> {
        &mut self.native
    }
}

impl<'s> Reader<'s> {
    /// A reader over one file's tokens with an empty variable table, lexed in mode 0 as used by
    /// GUI, sprite, font, effect and settings loaders.
    pub(crate) fn new(lexed: native::Lexed<'s>, unnamed: i32) -> Self {
        Self {
            native: native::Reader::new(lexed),
            poison: None,
            labels: vec![Label {
                parent: None,
                text: String::new(),
            }],
            file: Vec::new(),
            unnamed,
            dynamic: super::keywords::DynamicTokens::default(),
            read_ok: true,
        }
    }

    pub(super) fn label(&mut self, parent: Option<usize>, text: &str) -> usize {
        self.labels.push(Label {
            parent,
            text: text.to_owned(),
        });
        self.labels.len() - 1
    }

    /// Records that the game's own walk is undefined from here on (`reason` names the native
    /// failure), so where it reads next is not known.
    pub(super) fn poison(
        &mut self,
        refusals: &mut Vec<Refusal>,
        node: usize,
        key: &str,
        reason: &str,
    ) {
        let refusal = Refusal {
            path: String::new(),
            key: key.to_owned(),
            reason: format!(
                "{reason}; where the game reads from here is undefined, so everything not \
                 finished before it is refused"
            ),
            cause: Cause::Unaligned,
            node,
        };
        if !refusals.contains(&refusal) {
            refusals.push(refusal.clone());
        }
        self.poison.get_or_insert(refusal);
    }

    /// Peek through [`native::Reader::peek`]: the next token is returned again by the next
    /// read. At EOF, its kind becomes end-of-stream while its text is retained.
    pub(crate) fn peek(&mut self) -> Token {
        self.native.peek().clone()
    }

    /// Advance through [`native::Reader::advance`], retaining the lexer's resulting token. A
    /// failed read keeps that token and its quoted flag; [`Self::read_ok`] holds the read's
    /// result.
    pub(crate) fn read(&mut self) -> Token {
        self.read_ok = self.native.advance();
        self.native.current.clone()
    }

    /// A sub-reader's read: resolve a token starting with `@`, leaving a quoted token, and
    /// write it back as the current token regardless of the read's result.
    pub(crate) fn read_resolved(&mut self) -> Token {
        let token = self.read();
        if token.starts_with_at() {
            let resolved = self.resolve(token);
            self.current = resolved.clone();
            resolved
        } else {
            token
        }
    }

    /// The parser default: log an error and skip a block value
    /// ([`native::Reader::skip_reported`]).
    pub(crate) fn unexpected(&mut self) {
        self.skip_reported();
    }

    /// The integer point token walk: report an error unless the current value token is `{`,
    /// then make seven resolved reads (`x`, `=`, a number, `y`, `=`, a number, `}`). Checks
    /// only report errors; the walk ends early only at end-of-stream. Return the third and
    /// sixth tokens when read.
    pub(super) fn point_tokens(&mut self) -> [Option<Token>; 2] {
        let mut values = [None, None];
        for place in 0..7 {
            let token = self.read_resolved();
            if !self.read_ok {
                break;
            }
            match place {
                2 => values[0] = Some(token),
                5 => values[1] = Some(token),
                _ => {}
            }
        }
        values
    }

    /// [`Self::point_tokens`] with each value scanned by `%i` into one uninitialised local and
    /// stored in turn regardless of the scan's result. An unreached axis is untouched. A first
    /// scan storing nothing leaves an undefined value; a second such scan reuses the first
    /// value.
    pub(super) fn int_point(&mut self) -> [Axis; 2] {
        let values = self.point_tokens();
        let mut local: Option<i32> = None;
        let mut point = [Axis::Untouched, Axis::Untouched];
        for (axis, token) in values.iter().enumerate() {
            let Some(token) = token else {
                break;
            };
            if let Some(value) = crt::scan_i32(&token.text).stored() {
                local = Some(value);
            }
            point[axis] = match local {
                Some(value) => Axis::Value(value),
                None => Axis::Undefined,
            };
        }
        point
    }

    /// Read color from the lexer's current token. A value `@name` resolved only into the value
    /// slot is therefore no color. A number (0xC) is scanned by `%x` into a zeroed `u32`,
    /// unpacked from `0xAARRGGBB` and divided by 255 whenever the scan returns nonzero,
    /// including EOF. A `{` reads red, green and blue from successive resolved tokens with
    /// `%f`; failed scans retain the component. Components above 1 are divided by 255. Alpha
    /// starts at 1; a following token other than `}` is scanned into alpha and one more token
    /// read. Checks only report errors and skip nothing. At EOF, the ignored read result leaves
    /// the frozen token visible.
    pub(super) fn color(&mut self, color: &mut [f32; 4]) {
        let current = self.current.clone();
        if current.is_number() {
            let value = match crt::scan_x32(&current.text) {
                Scanned::Stored(value) => Some(value),
                // `sscanf` returns EOF (non-zero): the zeroed `u32` is stored.
                Scanned::End => Some(0),
                Scanned::NoMatch => None,
            };
            if let Some(value) = value {
                let [b, g, r, a] = value.to_le_bytes();
                *color = [r, g, b, a].map(|byte| f32::from(byte) / 255.0);
            }
            return;
        }
        if current.kind != TokenKind::Open {
            return;
        }
        for component in color.iter_mut().take(3) {
            self.read_resolved();
            if let Some(value) = crt::scan_f32(&self.current.text) {
                *component = value;
            }
            if *component > 1.0 {
                *component /= 255.0;
            }
        }
        color[3] = 1.0;
        self.read_resolved();
        if self.current.kind != TokenKind::Close {
            if let Some(value) = crt::scan_f32(&self.current.text) {
                color[3] = value;
            }
            self.read_resolved();
        }
    }

    /// The position reader into `target`. Only a block is read (the current token must
    /// be `{`; anything else is an error with nothing skipped): until a `}` or the end of the
    /// stream, each token read and resolved is `x` (0x20) or `y` (0x21), a coordinate; `min`
    /// (0x2A3), `max` (0x2A4) or `step` (0xBA), a bound (position bound reader); or anything else, an
    /// unexpected token, a block skipped. The closing token is read.
    pub(super) fn position(&mut self, refused: &mut Refused, key: &str, target: &mut Position) {
        if self.current.kind != TokenKind::Open {
            return;
        }
        loop {
            if self.peek().ends_block() {
                break;
            }
            let token = self.read_resolved();
            if !self.read_ok {
                break;
            }
            if token.is_keyword("x") {
                self.coordinate(refused, &format!("{key}.x"), &mut target.x, false);
            } else if token.is_keyword("y") {
                self.coordinate(refused, &format!("{key}.y"), &mut target.y, false);
            } else if token.is_keyword("min") {
                self.bound(refused, &format!("{key}.min"), &mut target.min, false);
            } else if token.is_keyword("max") {
                self.bound(refused, &format!("{key}.max"), &mut target.max, false);
            } else if token.is_keyword("step") {
                self.bound(refused, &format!("{key}.step"), &mut target.step, false);
            } else {
                self.skip_reported();
            }
        }
        self.read_resolved();
    }

    /// Read size into `target`, following [`Self::position`]'s structure. `width` (0x78) and
    /// `height` (0x1F) read coordinates; `min`, `max` and `step` read bounds;
    /// `preserve_aspect_ratio` (0x2EA) reads a boolean assignment.
    pub(super) fn size(&mut self, refused: &mut Refused, key: &str, target: &mut Size) {
        if self.current.kind != TokenKind::Open {
            return;
        }
        loop {
            if self.peek().ends_block() {
                break;
            }
            let token = self.read_resolved();
            if !self.read_ok {
                break;
            }
            if token.is_keyword("width") {
                self.coordinate(refused, &format!("{key}.width"), &mut target.size[0], true);
            } else if token.is_keyword("height") {
                self.coordinate(refused, &format!("{key}.height"), &mut target.size[1], true);
            } else if token.is_keyword("min") {
                self.bound(refused, &format!("{key}.min"), &mut target.min, true);
            } else if token.is_keyword("max") {
                self.bound(refused, &format!("{key}.max"), &mut target.max, true);
            } else if token.is_keyword("step") {
                self.bound(refused, &format!("{key}.step"), &mut target.step, true);
            } else if token.is_keyword("preserve_aspect_ratio") {
                if let Some(flag) = self.bool_assignment() {
                    target.preserve_aspect_ratio = flag;
                }
            } else {
                self.skip_reported();
            }
        }
        self.read_resolved();
    }

    /// Read a bound's operator and value, resolving both. Unless they are `=` and `{`, report
    /// an error and skip the current token's block. Within the block, `x`/`width` and
    /// `y`/`height` read coordinates; other tokens are unexpected and skip a block. Consume the
    /// closing `}` or EOF.
    fn bound(
        &mut self,
        refused: &mut Refused,
        key: &str,
        target: &mut [Coordinate; 2],
        size: bool,
    ) {
        let operator = self.read_resolved();
        if !self.read_ok {
            return;
        }
        if operator.kind == TokenKind::Equals {
            let value = self.read_resolved();
            if !self.read_ok {
                return;
            }
            if value.kind == TokenKind::Open {
                loop {
                    if self.peek().ends_block() {
                        break;
                    }
                    let token = self.read_resolved();
                    if !self.read_ok {
                        break;
                    }
                    let names: [&str; 2] = if size {
                        ["width", "height"]
                    } else {
                        ["x", "y"]
                    };
                    match names.iter().position(|name| token.is_keyword(name)) {
                        Some(axis) => self.coordinate(
                            refused,
                            &format!("{key}.{}", names[axis]),
                            &mut target[axis],
                            size,
                        ),
                        None => self.skip_reported(),
                    }
                }
                self.read_resolved();
                return;
            }
        }
        self.skip_reported();
    }

    /// Read and resolve the operator and, after `=`, the value. Convert the value as a boolean;
    /// return `None` when it stores nothing.
    fn bool_assignment(&mut self) -> Option<bool> {
        let operator = self.read_resolved();
        if operator.kind != TokenKind::Equals {
            return None;
        }
        read_bool(&self.read_resolved().text)
    }

    /// The margin reader (`margin`, `background_margin`, `padding`): only a block is read (else an
    /// error, and a skip of the current token's block); in it `left` (0x73), `top` (0x25A),
    /// `right` (0x74) and `bottom` (0x25B) read a position coordinate, and any other token is
    /// reported with nothing skipped, so a block among them is read token by token and its `}`
    /// ends the margin. The closing token is read.
    pub(super) fn margin(&mut self, refused: &mut Refused, key: &str, target: &mut Margin) {
        if self.current.kind != TokenKind::Open {
            self.skip_reported();
            return;
        }
        loop {
            if self.peek().ends_block() {
                break;
            }
            let token = self.read_resolved();
            if !self.read_ok {
                break;
            }
            let side = if token.is_keyword("left") {
                Some(("left", &mut target.left))
            } else if token.is_keyword("top") {
                Some(("top", &mut target.top))
            } else if token.is_keyword("right") {
                Some(("right", &mut target.right))
            } else if token.is_keyword("bottom") {
                Some(("bottom", &mut target.bottom))
            } else {
                None
            };
            if let Some((name, side)) = side {
                self.coordinate(refused, &format!("{key}.{name}"), side, false);
            }
        }
        self.read_resolved();
    }

    /// One coordinate assignment: resolve its operator and value, reporting an operator other
    /// than `=` but still using it. `auto` (0x2A2) sets the value to 0 and the auto bit.
    /// Otherwise a trailing `%` ORs in the percentage bit, and `%%` on a size also ORs in the
    /// position-relative bit. Scan the preceding text as a signed decimal integer and clear the
    /// auto bit.
    ///
    /// Both readers inspect the last byte through an unchecked, zero-extended index; the size
    /// reader also inspects the byte before a trailing `%`. An empty value, or a lone `%` on a
    /// size, reads 4 GiB past the text. That invalid read refuses the field.
    fn coordinate(
        &mut self,
        refused: &mut Refused,
        key: &str,
        target: &mut Coordinate,
        size: bool,
    ) {
        self.read_resolved();
        if !self.read_ok {
            return;
        }
        let value = self.read_resolved();
        if !self.read_ok {
            return;
        }
        if value.is_keyword("auto") {
            target.value = 0;
            target.auto = true;
            return;
        }
        // Both readers use the stored text only up to its first NUL.
        let text = value.c_str();
        if text.is_empty() || (size && text == b"%") {
            refused.refuse(
                key,
                format!(
                    "is {}, whose conversion reads 4 GiB past its text through the unchecked \
                     string byte lookup, an invalid read",
                    display(&value)
                ),
            );
            return;
        }
        let (number, percent, from_position) = match text {
            [rest @ .., b'%', b'%'] if size => (rest, true, true),
            [rest @ .., b'%'] => (rest, true, false),
            _ => (text, false, false),
        };
        target.percent |= percent;
        target.from_position |= from_position;
        target.auto = false;
        target.value = crate::fixed5::decimal_i32(number);
    }

    /// The integer component reader into `components`, only when the value (`+0xC0`) is
    /// `{` (anything else is recorded, a block skipped): each pass peeks, and a `}` or the end
    /// (token 0x13) is read and ends it, unresolved; otherwise each token read and resolved
    /// is a number (0xC), scanned with `%i` into the next component; or the next component's
    /// label, then `=` and a number; anything else
    /// is recorded and skips a block. `store(index, value)` receives each component `%i` stores.
    /// A label compared past the table of four reads past it: `Err` with the native failure.
    pub(super) fn components(&mut self, mut store: impl FnMut(usize, i32)) -> Result<(), String> {
        const LABELS: [&str; 4] = ["x", "y", "z", "w"];
        if self.value.kind != TokenKind::Open {
            self.unexpected();
            return Ok(());
        }
        let mut index = 0usize;
        loop {
            if self.peek().ends_block() {
                self.read();
                return Ok(());
            }
            let token = self.read();
            let token = if token.starts_with_at() {
                let resolved = self.resolve(token);
                self.current = resolved.clone();
                resolved
            } else {
                token
            };
            let number = if token.is_number() {
                Some(token)
            } else if index >= LABELS.len() {
                return Err(format!(
                    "holds {} after four components, which the integer component reader compares \
                     with a label read past its table of four",
                    display(&token)
                ));
            } else if token.is_keyword(LABELS[index]) {
                if self.read_resolved().kind == TokenKind::Equals {
                    let value = self.read_resolved();
                    if value.is_number() {
                        Some(value)
                    } else {
                        self.unexpected();
                        None
                    }
                } else {
                    self.unexpected();
                    None
                }
            } else {
                self.unexpected();
                None
            };
            if let Some(number) = number {
                if let Some(value) = crt::scan_i32(&number.text).stored() {
                    store(index, value);
                }
                index += 1;
            }
        }
    }
}

/// Fields refused for good, by key: an invalid read the game makes stores what cannot be known.
#[derive(Default)]
pub(super) struct Refused {
    pub by_key: std::collections::BTreeMap<String, String>,
}

impl Refused {
    pub(super) fn refuse(&mut self, key: &str, reason: String) {
        self.by_key.entry(key.to_owned()).or_insert(reason);
    }
}

/// One axis integer point reader wrote, or did not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// The walk ended before the axis's value.
    Untouched,
    Value(i32),
    /// The uninitialised local.
    Undefined,
}

/// How an object's field reader consumes tokens after its key and stored value.
#[derive(Clone, Copy, Debug)]
pub enum Case {
    /// Converts the stored value and reads no further token. A `{` value leaves
    /// its block to the enclosing loop, which reads its contents as keys.
    Scalar,
    /// Reads its own seven tokens through `Reader::int_point`.
    Point,
    /// Reads sub-object keys on the same reader until `}` or the end, using
    /// its own field dispatch. A scalar value still reads the keys that follow.
    Object(fn(&str) -> Option<Case>),
}

/// One key passed to an object's field reader by the persistent key loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistentKey {
    /// The keyword id assigned by the lexer to an unquoted word after folding and registered
    /// lookup. `None` for a quoted key (0xF), a number (0xC) or an unregistered word; none
    /// dispatch.
    pub keyword: Option<&'static str>,
    /// The stored value token (`+0xC0`) used by scalar field readers: its text up to the first
    /// NUL and whether it was quoted.
    pub value: Vec<u8>,
    pub quoted: bool,
    /// A [`Case::Point`] key's two axes as integer point reader stored them.
    pub point: Option<[Axis; 2]>,
    /// A [`Case::Object`] key's own keys, in order.
    pub keys: Vec<PersistentKey>,
}

/// Walks a file's keys in order with the mode-0 lexer and no dynamic tokens.
/// `case_for` dispatches registered keywords; unhandled keys skip their blocks.
pub fn persistent_keys(bytes: &[u8], case_for: fn(&str) -> Option<Case>) -> Vec<PersistentKey> {
    let mut reader = Reader::new(native::lex(bytes, native::Mode::KeepMark), 0);
    persistent_object(&mut reader, case_for)
}

/// Reads keys through the closing `}` or the end.
fn persistent_object(
    reader: &mut Reader,
    case_for: fn(&str) -> Option<Case>,
) -> Vec<PersistentKey> {
    let mut keys = Vec::new();
    while reader.next_key() {
        let keyword = keyword_of(&reader.key);
        let mut key = PersistentKey {
            keyword,
            value: crate::parser::c_string(&reader.value.text).to_vec(),
            quoted: reader.value.quoted(),
            point: None,
            keys: Vec::new(),
        };
        let case = keyword.and_then(case_for);
        match case {
            Some(Case::Scalar) => {}
            Some(Case::Point) => key.point = Some(reader.int_point()),
            Some(Case::Object(cases)) => key.keys = persistent_object(reader, cases),
            None => reader.unexpected(),
        }
        keys.push(key);
    }
    keys
}

/// The boolean token reader of a stored value: `yes`, `no`, or `None` when it stores nothing.
pub fn bool_of(text: &[u8]) -> Option<bool> {
    read_bool(text)
}

/// FNV-1a/64 over the value's text, with basis `cbf29ce484222325` and case-sensitive bytes, as
/// stored by tooltip keys.
pub fn fnv64(text: &[u8]) -> u64 {
    crate::parser::c_string(text)
        .iter()
        .fold(0xCBF2_9CE4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3)
        })
}
