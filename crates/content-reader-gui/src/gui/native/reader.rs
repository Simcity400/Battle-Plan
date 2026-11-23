
use super::{Cause, Coordinate, Margin, Position, Refusal, Size};
use content_reader_parser::{crt::{self, Scanned}, parser::read_bool};
use content_reader_script::native::{self, Kind as TokenKind, Token};

pub(crate) struct Label {
    pub parent: Option<usize>,
    pub text: String,
}

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

pub(crate) fn read_string(text: &[u8]) -> Vec<u8> {
    content_reader_parser::parser::read_string(text).to_vec()
}

pub(crate) fn keyword_of(token: &Token) -> Option<&'static str> {
    token.lookup().and_then(super::keywords::static_keyword)
}

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
    native: native::Reader<'s>,
    pub poison: Option<Refusal>,
    pub labels: Vec<Label>,
    pub file: Vec<Refusal>,
    pub unnamed: i32,
    pub dynamic: super::keywords::DynamicTokens,
    pub read_ok: bool,
    pub default_sounds: super::DefaultSounds,
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
            default_sounds: Default::default(),
        }
    }

    pub(super) fn label(&mut self, parent: Option<usize>, text: &str) -> usize {
        self.labels.push(Label {
            parent,
            text: text.to_owned(),
        });
        self.labels.len() - 1
    }

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

    pub(crate) fn peek(&mut self) -> Token {
        self.native.peek().clone()
    }

    pub(crate) fn read(&mut self) -> Token {
        self.read_ok = self.native.advance();
        self.native.current.clone()
    }

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

    pub(crate) fn unexpected(&mut self) {
        self.skip_reported();
    }

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

    pub(super) fn color(&mut self, color: &mut [f32; 4]) {
        let current = self.current.clone();
        if current.is_number() {
            let value = match crt::scan_x32(&current.text) {
                Scanned::Stored(value) => Some(value),
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

    fn bool_assignment(&mut self) -> Option<bool> {
        let operator = self.read_resolved();
        if operator.kind != TokenKind::Equals {
            return None;
        }
        read_bool(&self.read_resolved().text)
    }

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
        target.value = content_reader_parser::fixed5::decimal_i32(number);
    }

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

#[derive(Default)]
pub(super) struct Refused {
    pub by_key: std::collections::BTreeMap<String, String>,
}

impl Refused {
    pub(super) fn refuse(&mut self, key: &str, reason: String) {
        self.by_key.entry(key.to_owned()).or_insert(reason);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Untouched,
    Value(i32),
    Undefined,
}

#[derive(Clone, Copy, Debug)]
pub enum Case {
    Scalar,
    Point,
    Object(fn(&str) -> Option<Case>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistentKey {
    pub keyword: Option<&'static str>,
    pub value: Vec<u8>,
    pub quoted: bool,
    pub point: Option<[Axis; 2]>,
    pub keys: Vec<PersistentKey>,
}

pub fn persistent_keys(bytes: &[u8], case_for: fn(&str) -> Option<Case>) -> Vec<PersistentKey> {
    let mut reader = Reader::new(native::lex(bytes, native::Mode::KeepMark), 0);
    persistent_object(&mut reader, case_for)
}

fn persistent_object(
    reader: &mut Reader,
    case_for: fn(&str) -> Option<Case>,
) -> Vec<PersistentKey> {
    let mut keys = Vec::new();
    while reader.next_key() {
        let keyword = keyword_of(&reader.key);
        let mut key = PersistentKey {
            keyword,
            value: content_reader_parser::parser::c_string(&reader.value.text).to_vec(),
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

pub fn bool_of(text: &[u8]) -> Option<bool> {
    read_bool(text)
}

pub fn fnv64(text: &[u8]) -> u64 {
    content_reader_parser::parser::c_string(text)
        .iter()
        .fold(0xCBF2_9CE4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3)
        })
}
