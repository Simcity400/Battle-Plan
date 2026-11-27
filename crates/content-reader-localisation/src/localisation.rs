
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{ResolvedLayers, content::normalize_path};

fn invalid(message: impl Into<String>) -> io::Error {
    content_reader_parser::fields::invalid(message)
}

const STORED: [(char, char); 5] = [
    ('§', '\u{11}'),
    ('@', '\u{12}'),
    ('£', '\u{13}'),
    ('¤', '\u{14}'),
    ('{', '\u{1a}'),
];

fn store(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match STORED.iter().find(|(from, _)| *from == character) {
            Some(_) if characters.peek() == Some(&character) => {
                characters.next();
                out.push(character);
            }
            Some((_, control)) => out.push(*control),
            None => out.push(character),
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Localisation {
    pub language: String,
    entries: Entries,
    pub logged: Vec<String>,
    pub refused: Vec<String>,
    pub duplicates: Vec<String>,
}

#[derive(Clone)]
enum Entries {
    Read(BTreeMap<String, String>),
    Kept(content_reader_layers::kept::Table),
}

impl Default for Entries {
    fn default() -> Self {
        Self::Read(BTreeMap::new())
    }
}

impl FromIterator<(String, String)> for Entries {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(entries: I) -> Self {
        Self::Read(entries.into_iter().collect())
    }
}

impl PartialEq for Entries {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for Entries {}

impl std::fmt::Debug for Entries {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl Entries {
    fn get(&self, key: &str) -> Option<&str> {
        match self {
            Self::Read(entries) => entries.get(key).map(String::as_str),
            Self::Kept(table) => table
                .find(key.as_bytes())
                .and_then(|index| table.value(index))
                .and_then(|value| std::str::from_utf8(value).ok()),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Read(entries) => entries.len(),
            Self::Kept(table) => table.len(),
        }
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (&str, &str)> + '_> {
        match self {
            Self::Read(entries) => Box::new(
                entries
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            ),
            Self::Kept(table) => Box::new((0..table.len()).filter_map(move |index| {
                Some((
                    std::str::from_utf8(table.key(index)?).ok()?,
                    std::str::from_utf8(table.value(index)?).ok()?,
                ))
            })),
        }
    }

    fn insert(&mut self, key: String, value: String) {
        if let Self::Kept(_) = self {
            *self = self
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
        }
        if let Self::Read(entries) = self {
            entries.insert(key, value);
        }
    }
}

impl Localisation {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter()
    }

    pub fn keep(&self, out: &mut Vec<u8>) {
        use content_reader_layers::kept::{Table, put_bytes, put_texts};
        put_bytes(out, self.language.as_bytes());
        put_texts(out, &self.logged);
        put_texts(out, &self.refused);
        put_texts(out, &self.duplicates);
        let entries: Vec<(&str, &str)> = self.iter().collect();
        Table::write(
            out,
            entries
                .into_iter()
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        );
    }

    pub fn from_kept(bytes: &std::sync::Arc<Vec<u8>>, at: usize) -> io::Result<(Self, usize)> {
        let mut input = content_reader_layers::kept::Input::new(bytes, at);
        let language = input.text()?;
        let logged = input.texts()?;
        let refused = input.texts()?;
        let duplicates = input.texts()?;
        let (table, end) = content_reader_layers::kept::Table::read(bytes, input.at())?;
        Ok((
            Self {
                language,
                entries: Entries::Kept(table),
                logged,
                refused,
                duplicates,
            },
            end,
        ))
    }

    pub fn localise(&self, key: &[u8]) -> Result<String, Unlocalised> {
        self.localise_with(key, &[])
    }

    pub fn localise_with(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
    ) -> Result<String, Unlocalised> {
        self.localised(key, arguments, false).0
    }

    pub fn localise_with_fallbacks(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
    ) -> (Result<String, Unlocalised>, Vec<String>) {
        self.localised(key, arguments, true)
    }

    fn localised(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        collect_fallbacks: bool,
    ) -> (Result<String, Unlocalised>, Vec<String>) {
        let key = crate::parser::c_string(key);
        if key.is_empty() {
            return (Ok(String::new()), Vec::new());
        }
        let mut output = self.output(key, arguments, collect_fallbacks);
        let mut notes = output.fallbacks.take().unwrap_or_default();
        if collect_fallbacks && !output.cuts.is_empty() {
            notes.insert(format!(
                "localisation {} was cut at the game's output buffer limit",
                String::from_utf8_lossy(key)
            ));
        }
        let text = output.finish().map_err(|cause| Unlocalised {
            key: String::from_utf8_lossy(key).into_owned(),
            cause,
        });
        (text, notes.into_iter().collect())
    }

    pub fn localise_bytes(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
    ) -> Result<Vec<u8>, Unlocalised> {
        let key = crate::parser::c_string(key);
        let output = self.output(key, arguments, false);
        match output.cause {
            Some(cause) => Err(Unlocalised {
                key: String::from_utf8_lossy(key).into_owned(),
                cause,
            }),
            None => Ok(output.bytes),
        }
    }

    pub fn requires_arguments(&self, key: &[u8], names: &[&str]) -> bool {
        let key = crate::parser::c_string(key);
        if key.is_empty() {
            return false;
        }
        let mut out = Output::default();
        self.substitute(key, &[], 0, &mut out, names);
        out.required.is_some()
    }

    pub fn localise_with_deferred(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        deferred: &[&str],
    ) -> Result<Result<String, Unlocalised>, usize> {
        let key = crate::parser::c_string(key);
        if key.is_empty() {
            return Ok(Ok(String::new()));
        }
        let mut out = Output::default();
        self.substitute(key, arguments, 0, &mut out, deferred);
        if let Some(index) = out.required {
            return Err(index);
        }
        Ok(out.finish().map_err(|cause| Unlocalised {
            key: String::from_utf8_lossy(key).into_owned(),
            cause,
        }))
    }

    #[cfg(test)]
    fn fallbacks(&self, key: &[u8], arguments: &[(&str, Argument<'_>)]) -> Vec<String> {
        self.localise_with_fallbacks(key, arguments).1
    }

    fn output(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        collect_fallbacks: bool,
    ) -> Output {
        let mut out = Output {
            fallbacks: collect_fallbacks.then(BTreeSet::new),
            ..Output::default()
        };
        self.substitute(key, arguments, 0, &mut out, &[]);
        out
    }

    fn substitute(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        depth: u32,
        out: &mut Output,
        required_arguments: &[&str],
    ) {
        if depth >= SUBSTITUTION_DEPTH {
            if let Some(fallbacks) = &mut out.fallbacks {
                fallbacks.insert(format!(
                    "localisation {} reached the game's substitution depth limit; text was omitted",
                    String::from_utf8_lossy(key)
                ));
            }
            return;
        }
        if out
            .silent
            .get(key)
            .is_some_and(|&depths| depths & (1 << depth) != 0)
        {
            return;
        }
        let written = out.bytes.len();
        self.expand(key, arguments, depth, out, required_arguments);
        if out.bytes.len() == written && out.required.is_none() {
            *out.silent.entry(key.to_vec()).or_default() |= 1 << depth;
        }
    }

    fn expand(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        depth: u32,
        out: &mut Output,
        required_arguments: &[&str],
    ) {
        let Some(text) = std::str::from_utf8(key)
            .ok()
            .and_then(|key| self.entries.get(key))
        else {
            if let Some(fallbacks) = &mut out.fallbacks {
                fallbacks.insert(format!(
                    "localisation {} is missing; {}",
                    String::from_utf8_lossy(key),
                    if depth == 0 {
                        "the key is shown"
                    } else {
                        "the reference is omitted"
                    }
                ));
            }
            if depth == 0 {
                out.copy(key);
            }
            return;
        };
        let text = crate::parser::c_string(text.as_bytes());
        let mut rest = text;
        while !rest.is_empty() {
            let Some(open) = rest.iter().position(|&byte| byte == b'$') else {
                out.copy(rest);
                return;
            };
            let Some(close) = rest[open + 1..]
                .iter()
                .position(|&byte| byte == b'$')
                .map(|at| open + 1 + at)
            else {
                out.copy(rest);
                return;
            };
            if close == open + 1 {
                out.copy(&rest[..close]);
            } else {
                if open != 0 {
                    out.copy(&rest[..open]);
                    if out.full() {
                        return;
                    }
                }
                let operand = &rest[open + 1..close];
                let name = operand.split(|&byte| byte == b'|').next().unwrap();
                if !out.full()
                    && let Some(index) = required_arguments
                        .iter()
                        .position(|required| required.as_bytes() == name)
                {
                    out.required = Some(index);
                    return;
                }
                if !argument(operand, arguments, out) {
                    self.substitute(operand, arguments, depth + 1, out, required_arguments);
                }
            }
            if out.full() {
                return;
            }
            rest = &rest[close + 1..];
        }
    }
}

const SUBSTITUTION_BYTES: usize = 0x1800;
const SUBSTITUTION_DEPTH: u32 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Argument<'a> {
    Text(&'a str),
    Int32(i32),
    Float32(u32),
    Fixed5(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unlocalised {
    pub key: String,
    pub cause: Cause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    CutInsideCharacter,
    NotText,
    Float32Precision { precision: u8 },
}

impl std::fmt::Display for Unlocalised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "localisation {}", self.key)?;
        match &self.cause {
            Cause::CutInsideCharacter => {
                f.write_str(": the game's 0x1800-byte buffer ends inside a multi-byte character")
            }
            Cause::NotText => f.write_str(": its text is not UTF-8"),
            Cause::Float32Precision { precision } => write!(
                f,
                ": Float32 precision {precision} selects an uninitialized native format"
            ),
        }
    }
}

impl std::error::Error for Unlocalised {}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    required: Option<usize>,
    capacity: Option<usize>,
    fallbacks: Option<BTreeSet<String>>,
    silent: BTreeMap<Vec<u8>, u32>,
    cuts: Vec<usize>,
    cause: Option<Cause>,
}

impl Output {
    fn room(&self) -> usize {
        self.capacity.unwrap_or(SUBSTITUTION_BYTES) - self.bytes.len()
    }

    fn full(&self) -> bool {
        self.required.is_some() || self.room() == 0
    }

    fn copy(&mut self, bytes: &[u8]) {
        let taken = bytes.len().min(self.room());
        self.bytes.extend_from_slice(&bytes[..taken]);
        if taken < bytes.len() {
            self.cuts.push(self.bytes.len());
        }
    }

    fn byte(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    fn snprintf(&mut self, text: &[u8]) -> bool {
        let room = self.room();
        if text.len() < room {
            self.bytes.extend_from_slice(text);
            return true;
        }
        self.bytes.extend_from_slice(&text[..room - 1]);
        self.cuts.push(self.bytes.len());
        self.bytes.push(b' ');
        false
    }

    fn finish(self) -> Result<String, Cause> {
        if let Some(cause) = self.cause {
            return Err(cause);
        }
        String::from_utf8(self.bytes).map_err(|error| {
            let bytes = error.as_bytes();
            let start = error.utf8_error().valid_up_to();
            let cut = self.cuts.iter().any(|&cut| {
                cut > start
                    && std::str::from_utf8(&bytes[start..cut])
                        .is_err_and(|error| error.valid_up_to() == 0 && error.error_len().is_none())
            });
            if cut {
                Cause::CutInsideCharacter
            } else {
                Cause::NotText
            }
        })
    }
}

pub fn format_scripted_variable(value: i64, format: Option<&[u8]>) -> String {
    if let Some(format) = format {
        let mut out = Output {
            capacity: Some(0x3f),
            ..Output::default()
        };
        formatted(Argument::Fixed5(value), format, &mut out);
        if !out.bytes.is_empty() {
            return out.finish().expect("fixed5 formatter writes only ASCII");
        }
    }
    crate::fixed5::fixed5_to_string(value, -1)
}

pub fn format_argument(value: Argument<'_>, format: &[u8]) -> Result<String, Cause> {
    let mut out = Output::default();
    formatted(value, format, &mut out);
    out.finish()
}

pub fn argument_format<'t>(template: &'t str, name: &str) -> Option<&'t str> {
    let mut rest = template;
    loop {
        let open = rest.find('$')?;
        let close = open + 1 + rest[open + 1..].find('$')?;
        let operand = &rest[open + 1..close];
        let (operand_name, format) = operand.split_once('|').unwrap_or((operand, ""));
        if close > open + 1 && operand_name == name {
            return Some(format);
        }
        rest = &rest[close + 1..];
    }
}

fn argument(operand: &[u8], arguments: &[(&str, Argument<'_>)], out: &mut Output) -> bool {
    if out.full() {
        return false;
    }
    let end = operand
        .iter()
        .position(|&byte| byte == b'|')
        .unwrap_or(operand.len());
    let Some((_, value)) = arguments
        .iter()
        .find(|(name, _)| name.as_bytes() == &operand[..end])
    else {
        return false;
    };
    let format = operand.get(end + 1..).unwrap_or_default();
    formatted(*value, format, out);
    true
}

#[derive(Default)]
struct Directives {
    colour: Option<u8>,
    percent: bool,
    no_multiply: bool,
    metric: bool,
    compact: bool,
    sign: bool,
    minus: bool,
    precision: u8,
    plus: bool,
    upper: bool,
    lower: bool,
    hide_zero: bool,
    trim: bool,
}

impl Directives {
    fn read(format: &[u8]) -> Self {
        let mut read = Self {
            precision: 2,
            ..Self::default()
        };
        for (index, &byte) in format.iter().enumerate() {
            match byte {
                b'%' => {
                    if read.percent && format[index - 1] == b'%' {
                        read.no_multiply = true;
                    }
                    read.percent = true;
                }
                b'*' => read.metric = true,
                b'+' => (read.sign, read.minus) = (true, false),
                b'-' => (read.sign, read.minus) = (true, true),
                b'0'..=b'9' => read.precision = byte - b'0',
                b'=' => read.plus = true,
                b'U' => read.upper = true,
                b'^' => read.compact = true,
                b'_' => read.hide_zero = true,
                b'l' => read.lower = true,
                b'~' => read.trim = true,
                byte if byte.is_ascii_alphabetic() => read.colour = Some(byte),
                _ => (),
            }
        }
        read
    }
}

pub fn crt_case(byte: u8, upper: bool) -> u8 {
    if upper {
        byte.to_ascii_uppercase()
    } else {
        byte.to_ascii_lowercase()
    }
}

pub fn crt_case_text(text: &str, upper: bool, first_only: bool) -> String {
    let end = text.find('\0').unwrap_or(text.len());
    let changed = if first_only { end.min(1) } else { end };
    let mut bytes = text.as_bytes().to_vec();
    for byte in &mut bytes[..changed] {
        *byte = crt_case(*byte, upper);
    }
    String::from_utf8(bytes).expect("only ASCII bytes change")
}

fn formatted(value: Argument<'_>, format: &[u8], out: &mut Output) {
    let read = Directives::read(format);
    if let Some(colour) = read.colour {
        for byte in [0x11, colour] {
            out.byte(byte);
            if out.full() {
                return;
            }
        }
    }
    let multiplier = if read.no_multiply { 1 } else { 100 };
    let suffixes = if read.metric {
        Some(METRIC)
    } else if read.compact {
        Some(COMPACT)
    } else {
        None
    };
    let (number, positive, negative) = match value {
        Argument::Text(text) => {
            let text = text.split('\0').next().unwrap_or_default();
            if !text.is_empty() {
                let first =
                    (read.upper || read.lower).then(|| crt_case(text.as_bytes()[0], read.upper));
                let start = out.bytes.len();
                out.copy(text.as_bytes());
                if let (Some(first), Some(slot)) = (first, out.bytes.get_mut(start)) {
                    *slot = first;
                }
                if out.full() {
                    return;
                }
            }
            (None, false, false)
        }
        Argument::Int32(value) => {
            let shown = match suffixes {
                Some(suffixes) => scaled(value, suffixes, read.precision),
                None if read.percent => value.wrapping_mul(multiplier).to_string(),
                None => value.to_string(),
            };
            (Some(shown), value > 0, value < 0)
        }
        Argument::Float32(bits) => {
            if read.precision > 6 {
                let cause = Cause::Float32Precision {
                    precision: read.precision,
                };
                if let Some(fallbacks) = &mut out.fallbacks {
                    fallbacks.insert(format!(
                        "Float32 precision {} selects an uninitialized native format; the operand is refused",
                        read.precision
                    ));
                }
                out.cause.get_or_insert(cause);
                return;
            }
            let value = f32::from_bits(bits);
            let shown = if read.percent && !value.is_nan() {
                value * multiplier as f32
            } else {
                value
            };
            (
                Some(float32(shown, read.precision)),
                value > 0.0,
                value < 0.0,
            )
        }
        Argument::Fixed5(value) => {
            let shown = match suffixes {
                Some(suffixes) => {
                    let whole = (value / 100_000) as i32;
                    if whole.wrapping_abs() < 1000 {
                        crate::fixed5::format_fixed5(value, false, read.precision)
                    } else {
                        scaled(whole, suffixes, read.precision)
                    }
                }
                None => {
                    let shown = if read.percent {
                        i64::from(multiplier).wrapping_mul(value)
                    } else {
                        value
                    };
                    crate::fixed5::format_fixed5(shown, read.trim, read.precision)
                }
            };
            (Some(shown), value > 0, value < 0)
        }
    };
    if let Some(number) = number
        && (!read.hide_zero || positive || negative)
    {
        let mut text = Vec::new();
        if read.sign {
            text.push(0x11);
            text.push(match (positive || negative, positive == read.minus) {
                (false, _) => b'Y',
                (true, true) => b'R',
                (true, false) => b'G',
            });
        }
        if read.plus && positive {
            text.push(b'+');
        }
        text.extend_from_slice(number.as_bytes());
        if read.percent {
            text.push(b'%');
        }
        if read.sign {
            text.extend_from_slice(b"\x11!");
        }
        if !out.snprintf(&text) {
            return;
        }
    }
    if read.colour.is_some() {
        out.byte(0x11);
        if !out.full() {
            out.byte(b'!');
        }
    }
}

fn float32(value: f32, precision: u8) -> String {
    if value.is_nan() {
        return if value.to_bits() == 0xffc0_0000 {
            "-nan(ind)"
        } else if value.is_sign_negative() {
            "-nan"
        } else {
            "nan"
        }
        .to_owned();
    }
    format!("{:.*}", usize::from(precision), f64::from(value))
}

const METRIC: [&str; 4] = ["", "K", "M", "G"];
const COMPACT: [&str; 4] = ["", "K", "M", "B"];

fn scaled(value: i32, suffixes: [&str; 4], precision: u8) -> String {
    let magnitude = match value.wrapping_neg() {
        negated if negated < 0 => value,
        negated => negated,
    };
    let sign = if value < 0 { "-" } else { "" };
    if magnitude < 1000 {
        return format!("{sign}{magnitude}");
    }
    let mut scale = 1i32;
    let mut power = 0;
    let mut quotient = magnitude as u32;
    loop {
        scale = scale.wrapping_mul(1000);
        power += 1;
        quotient /= 1000;
        if quotient < 1000 {
            break;
        }
    }
    let whole = magnitude / scale;
    let precision = usize::from(precision.min(4));
    if precision < 1 {
        return format!("{sign}{whole}{}", suffixes[power]);
    }
    let remainder = format!("{:0width$}", magnitude % scale, width = 3 * power);
    format!(
        "{sign}{whole}.{}{}",
        &remainder[..precision.min(remainder.len())],
        suffixes[power]
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Colour(char),
    Flag(&'a str),
    Icon(&'a str),
    Control14,
    Control1A(&'a str),
}

impl Piece<'_> {
    pub fn describe(&self) -> String {
        match self {
            Self::Text(_) => "text".into(),
            Self::Colour(code) => format!("the colour code 0x11 {code:?}"),
            Self::Flag(tag) => format!("the inline flag 0x12 {tag:?} (an authored `@`)"),
            Self::Icon(name) => format!("the icon 0x13 {name:?} (an authored `£`)"),
            Self::Control14 => "the control byte 0x14 (an authored `¤`)".into(),
            Self::Control1A(digits) => {
                format!("the control byte 0x1A {digits:?} (an authored `{{`)")
            }
        }
    }
}

pub fn plain_text(text: &str) -> Result<String, String> {
    let mut clean = String::new();
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => clean.push_str(text),
            Piece::Colour(_) | Piece::Icon(_) | Piece::Flag(_) => (),
            Piece::Control14 | Piece::Control1A(_) => {
                return Err(format!(
                    "{}, which plain text cannot show",
                    piece.describe()
                ));
            }
        }
    }
    Ok(clean)
}

pub fn display_text(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "is not UTF-8".to_owned())?;
    plain_text(text)
        .map(|text| text.trim().to_owned())
        .map_err(|reason| format!("holds {reason}"))
}

pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut run = 0;
    let mut at = 0;
    let after = |from: usize, count: usize| {
        text[from..]
            .char_indices()
            .nth(count)
            .map_or(text.len(), |(offset, _)| from + offset)
    };
    while let Some(character) = text[at..].chars().next() {
        let control = at;
        let body = at + character.len_utf8();
        let (piece, next) = match character {
            '\u{11}' => match text[body..].chars().next() {
                Some(code) => (Some(Piece::Colour(code)), body + code.len_utf8()),
                None => (None, body),
            },
            '\u{12}' => {
                let end = after(body, 3);
                (Some(Piece::Flag(&text[body..end])), end)
            }
            '\u{13}' => {
                let mut end = body;
                let mut count = 0;
                let mut next = text.len();
                for (offset, name) in text[body..].char_indices() {
                    if !(name.is_ascii_alphanumeric() || name == '_' || name == '|') {
                        next = body + offset + name.len_utf8();
                        break;
                    }
                    end = body + offset + 1;
                    count += 1;
                    if count == 128 {
                        next = end;
                        break;
                    }
                }
                (Some(Piece::Icon(&text[body..end])), next.max(end))
            }
            '\u{14}' => (Some(Piece::Control14), body),
            '\u{1a}' => {
                let end = after(body, 2);
                (Some(Piece::Control1A(&text[body..end])), end)
            }
            _ => {
                at = body;
                continue;
            }
        };
        if run < control {
            out.push(Piece::Text(&text[run..control]));
        }
        out.extend(piece);
        at = next;
        run = next;
    }
    if run < text.len() {
        out.push(Piece::Text(&text[run..]));
    }
    out
}

impl FromIterator<(String, String)> for Localisation {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(entries: I) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, value)| (key, store(&value)))
                .collect(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedFile {
    pub entries: Vec<(String, String)>,
    pub logged: Vec<String>,
    pub refused: Vec<String>,
}

fn file_space(character: char) -> bool {
    matches!(character, '\t'..='\r' | '\u{14}' | '\u{1c}'..=' ' | '\u{a0}' | '\u{2007}' | '\u{202f}')
}

fn file_key(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '\'' | '-' | '.' | '_')
}

fn file_value(
    body: &[char],
    column: usize,
    line: u32,
    logged: &mut Vec<String>,
    keep: bool,
) -> String {
    let mut decoded = String::new();
    let mut index = 0;
    while index < body.len() {
        let character = body[index];
        if character == '\\' && index + 1 < body.len() {
            index += 1;
            let value = match body[index] {
                '"' => '"',
                '\\' => '\\',
                'n' | 'N' => '\n',
                't' | 'T' => '\t',
                other => {
                    logged.push(format!(
                        "{line}: unsupported escape character {} at column {}; escape kept without backslash",
                        other as u32, column + index + 1
                    ));
                    other
                }
            };
            if keep {
                decoded.push(value);
            }
        } else if keep {
            decoded.push(character);
        }
        index += 1;
    }
    store(decoded.split('\0').next().unwrap_or_default())
}

pub fn parse_file(bytes: &[u8], language: &str) -> ParsedFile {
    scan_file(bytes, language, &mut Vec::new())
}

fn scan_file(bytes: &[u8], language: &str, headers: &mut Vec<String>) -> ParsedFile {
    let mut parsed = ParsedFile::default();
    if bytes.is_empty() {
        return parsed;
    }
    let Some(bytes) = bytes.strip_prefix(b"\xef\xbb\xbf") else {
        parsed
            .logged
            .push("1: localisation files require UTF-8 with a leading BOM".into());
        parsed
            .logged
            .push("1: leading UTF-8 BOM is missing; file skipped".into());
        return parsed;
    };
    let mut current = None;
    let mut characters = Vec::new();
    let mut rest = bytes;
    let mut number = 0;
    while !rest.is_empty() {
        number += 1;
        let end = rest
            .iter()
            .position(|&byte| byte == b'\n')
            .unwrap_or(rest.len());
        let line = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or_default();
        match std::str::from_utf8(line) {
            Ok(text) => {
                characters.clear();
                characters.extend(text.chars());
            }
            Err(_) => parsed.logged.push(format!(
                "{number}: line is not valid UTF-8; previous decoded line reused"
            )),
        }
        let start = characters.iter().take_while(|&&c| file_space(c)).count();
        if characters.get(start) == Some(&'#') || start == line.len() {
            continue;
        }
        let key_end = start
            + characters[start..]
                .iter()
                .take_while(|&&c| file_key(c))
                .count();
        if key_end == start {
            parsed.logged.push(format!(
                "{number}: invalid localisation key at column {}; remainder skipped",
                key_end + 1
            ));
            break;
        }
        if characters.get(key_end) != Some(&':') {
            parsed.logged.push(format!(
                "{number}: key is missing its colon at column {}; remainder skipped",
                key_end + 1
            ));
            break;
        }
        let key: String = characters[start..key_end].iter().collect();
        let mut value_start = key_end + 1;
        while characters
            .get(value_start)
            .is_some_and(char::is_ascii_digit)
        {
            value_start += 1;
        }
        while characters.get(value_start).is_some_and(|&c| file_space(c)) {
            value_start += 1;
        }
        if value_start == characters.len() {
            headers.push(key.clone());
            current = Some(key);
            continue;
        }
        if characters.get(value_start) != Some(&'"') {
            parsed.logged.push(format!(
                "{number}: value is missing its opening quote at column {}; remainder skipped",
                value_start + 1
            ));
            break;
        }
        if current.is_none() {
            parsed.refused.push(format!("{number}: entry {key} precedes any language header; language context is uninitialized and the remainder is refused"));
            break;
        }
        let body_start = value_start + 1;
        let body_end = (body_start..characters.len())
            .rev()
            .find(|&at| characters[at] == '"')
            .unwrap_or(0);
        let body = if body_end > body_start {
            &characters[body_start..body_end]
        } else {
            &[]
        };
        let keep = current.as_deref().and_then(|key| key.strip_prefix("l_")) == Some(language);
        let value = file_value(body, body_start, number, &mut parsed.logged, keep);
        if keep {
            parsed.entries.push((key, value));
        }
    }
    parsed
}

fn is_replace(path: impl AsRef<[u8]>) -> bool {
    path.as_ref()
        .split(|byte| *byte == b'/')
        .any(|part| part.eq_ignore_ascii_case(b"replace"))
}

fn load_order(path: impl AsRef<[u8]>, layer: usize) -> (bool, usize, Vec<u8>) {
    let path = path.as_ref();
    (is_replace(path), layer, path.to_vec())
}

fn merge_file(
    localisation: &mut Localisation,
    sources: &mut BTreeMap<String, String>,
    replace: bool,
    path: &str,
    parsed: ParsedFile,
) {
    for (key, value) in parsed.entries {
        if replace {
            localisation.entries.insert(key, value);
            continue;
        }
        match sources.get(&key) {
            Some(kept) => localisation.duplicates.push(format!("{key} {kept} {path}")),
            None => {
                sources.insert(key.clone(), path.to_owned());
                localisation.entries.insert(key, value);
            }
        }
    }
    localisation
        .logged
        .extend(parsed.logged.into_iter().map(|log| format!("{path}:{log}")));
    localisation.refused.extend(
        parsed
            .refused
            .into_iter()
            .map(|refusal| format!("{path}:{refusal}")),
    );
}

pub fn read(layers: &ResolvedLayers, language: &str) -> io::Result<Localisation> {
    let language = carried(layers, language);
    read_with(layers, language, &|bytes| parse_file(bytes, language))
}

pub fn declared_languages(layers: &ResolvedLayers) -> io::Result<Vec<String>> {
    let bytes = layers
        .read(LANGUAGE_LIST)
        .map_err(|error| io::Error::new(error.kind(), format!("{LANGUAGE_LIST}: {error}")))?;
    let mut headers = Vec::new();
    let parsed = scan_file(&bytes, "", &mut headers);
    let mut declared: Vec<String> = Vec::new();
    for header in headers {
        if let Some(name) = header.strip_prefix("l_")
            && !name.is_empty()
            && !declared.iter().any(|known| known == name)
        {
            declared.push(name.to_owned());
        }
    }
    if declared.is_empty() {
        let why = parsed.logged.first().map_or(String::new(), |line| format!(" (line {line})"));
        return Err(invalid(format!("{LANGUAGE_LIST} declares no language{why}")));
    }
    Ok(declared)
}

pub fn selected_language(layers: &ResolvedLayers, requested: Option<&str>) -> io::Result<String> {
    let mut declared = declared_languages(layers)?;
    let entry = |name: &str| requested.and_then(|wanted| wanted.strip_prefix("l_")) == Some(name);
    Ok(match declared.iter().position(|name| entry(name)) {
        Some(at) => declared.swap_remove(at),
        None => declared.swap_remove(0),
    })
}

const LANGUAGE_LIST: &str = "localisation/languages.yml";

fn carried<'a>(layers: &'a ResolvedLayers, language: &'a str) -> &'a str {
    match layers.recorded_language() {
        Some(recorded) if recorded != language => {
            content_reader_layers::content::diagnostics::Diagnostics::note(format!(
                "Text is shown in {recorded}, the language this recording was made in: it carries no {language} text."
            ));
            recorded
        }
        _ => language,
    }
}

fn localisation_files(
    layers: &ResolvedLayers,
    language: &str,
) -> io::Result<Vec<(bool, usize, Vec<u8>)>> {
    if language.is_empty()
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(invalid(format!("unsupported language name {language:?}")));
    }
    let mut files = Vec::new();
    for (path, layer) in layers.effective_paths()? {
        let lower = path.to_ascii_lowercase();
        if !lower.starts_with(b"localisation/") || !lower.ends_with(b".yml") {
            continue;
        }
        files.push(load_order(path, layer));
    }
    files.sort();
    Ok(files)
}

pub fn observe_files(layers: &ResolvedLayers, language: &str) -> io::Result<()> {
    let language = carried(layers, language);
    let files = localisation_files(layers, language)?;
    let read = content_reader_layers::parallel::map(&files, |(_, _, path)| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = layers.read_path(&normalize_path(path)?)?;
            io::Result::Ok(())
        }))
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Localisation reader worker failed",
            ))
        })
    });
    for result in read {
        result?;
    }
    Ok(())
}

fn read_with(
    layers: &ResolvedLayers,
    language: &str,
    parse: &(dyn Fn(&[u8]) -> ParsedFile + Sync),
) -> io::Result<Localisation> {
    let files = localisation_files(layers, language)?;
    let mut localisation = Localisation {
        language: language.to_owned(),
        ..Localisation::default()
    };
    let mut sources = BTreeMap::new();
    let read = content_reader_layers::parallel::map(&files, |(_, _, path)| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bytes = layers.read_path(&normalize_path(path)?)?;
            io::Result::Ok(parse(&bytes))
        }))
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Localisation reader worker failed",
            ))
        })
    });
    for ((replace, _, path), parsed) in files.iter().zip(read) {
        let parsed = parsed?;
        let path = String::from_utf8_lossy(path);
        merge_file(&mut localisation, &mut sources, *replace, &path, parsed);
    }
    Ok(localisation)
}

const MEMO_MAGIC: &[u8; 8] = b"HOI4LP\x00\x01";

pub fn read_cached(
    layers: &ResolvedLayers,
    language: &str,
    cache_dir: &std::path::Path,
) -> io::Result<Localisation> {
    let language = carried(layers, language);
    let Some((program, build)) = build() else {
        return read(layers, language);
    };
    let build = *build;
    let path = cache_dir.join(format!("localisation-{language}-{program}.bin"));
    let memo = std::fs::read(&path)
        .ok()
        .and_then(|bytes| decode_memo(&bytes, build, language))
        .unwrap_or_default();
    let used = std::sync::Mutex::new(BTreeMap::<[u8; 32], Option<ParsedFile>>::new());
    let localisation = read_with(layers, language, &|bytes| {
        let digest = *blake3::hash(bytes).as_bytes();
        let kept = memo.get(&digest).cloned();
        let parsed = kept.clone().unwrap_or_else(|| parse_file(bytes, language));
        let mut used = used.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = used.entry(digest).or_default();
        if kept.is_none() {
            *entry = Some(parsed.clone());
        }
        parsed
    })?;
    let used = used
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if used.values().any(Option::is_some) || used.len() != memo.len() {
        let kept: BTreeMap<[u8; 32], ParsedFile> = used
            .into_iter()
            .filter_map(|(digest, parsed)| {
                Some((digest, parsed.or_else(|| memo.get(&digest).cloned())?))
            })
            .collect();
        let _ = write_memo(&path, build, language, &kept);
    }
    Ok(localisation)
}

fn build() -> Option<&'static (String, (u64, u128))> {
    static BUILD: std::sync::OnceLock<Option<(String, (u64, u128))>> = std::sync::OnceLock::new();
    BUILD
        .get_or_init(|| {
            let executable = std::env::current_exe().ok()?;
            let program = executable.file_stem()?.to_str()?.to_owned();
            let metadata = executable.metadata().ok()?;
            let modified = metadata
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            Some((program, (metadata.len(), modified.as_nanos())))
        })
        .as_ref()
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend((text.len() as u64).to_le_bytes());
    out.extend(text.as_bytes());
}

fn put_texts(out: &mut Vec<u8>, texts: &[String]) {
    out.extend((texts.len() as u64).to_le_bytes());
    for text in texts {
        put_text(out, text);
    }
}

fn write_memo(
    path: &std::path::Path,
    build: (u64, u128),
    language: &str,
    kept: &BTreeMap<[u8; 32], ParsedFile>,
) -> io::Result<()> {
    let mut out = Vec::new();
    out.extend(MEMO_MAGIC);
    out.extend(build.0.to_le_bytes());
    out.extend(build.1.to_le_bytes());
    put_text(&mut out, language);
    out.extend((kept.len() as u64).to_le_bytes());
    for (digest, parsed) in kept {
        out.extend(digest);
        out.extend((parsed.entries.len() as u64).to_le_bytes());
        for (key, value) in &parsed.entries {
            put_text(&mut out, key);
            put_text(&mut out, value);
        }
        put_texts(&mut out, &parsed.logged);
        put_texts(&mut out, &parsed.refused);
    }
    let checksum = *blake3::hash(&out).as_bytes();
    out.extend(checksum);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&temporary, &out)
        .and_then(|()| std::fs::rename(&temporary, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
}

fn decode_memo(
    bytes: &[u8],
    build: (u64, u128),
    language: &str,
) -> Option<std::collections::HashMap<[u8; 32], ParsedFile>> {
    struct Input<'a>(&'a [u8]);
    impl<'a> Input<'a> {
        fn take(&mut self, length: usize) -> Option<&'a [u8]> {
            let (head, rest) = self.0.split_at_checked(length)?;
            self.0 = rest;
            Some(head)
        }
        fn count(&mut self, least: usize) -> Option<usize> {
            let count = usize::try_from(u64::from_le_bytes(self.take(8)?.try_into().ok()?)).ok()?;
            (count <= self.0.len() / least.max(1)).then_some(count)
        }
        fn text(&mut self) -> Option<String> {
            let length = self.count(1)?;
            String::from_utf8(self.take(length)?.to_vec()).ok()
        }
        fn texts(&mut self) -> Option<Vec<String>> {
            let count = self.count(8)?;
            (0..count).map(|_| self.text()).collect()
        }
    }
    let (payload, checksum) = bytes.split_at_checked(bytes.len().checked_sub(32)?)?;
    if blake3::hash(payload).as_bytes() != checksum {
        return None;
    }
    let mut input = Input(payload);
    if input.take(MEMO_MAGIC.len())? != MEMO_MAGIC
        || u64::from_le_bytes(input.take(8)?.try_into().ok()?) != build.0
        || u128::from_le_bytes(input.take(16)?.try_into().ok()?) != build.1
        || input.text()? != language
    {
        return None;
    }
    let count = input.count(32)?;
    let mut memo = std::collections::HashMap::with_capacity(count);
    for _ in 0..count {
        let digest: [u8; 32] = input.take(32)?.try_into().ok()?;
        let entries = input.count(16)?;
        let entries = (0..entries)
            .map(|_| Some((input.text()?, input.text()?)))
            .collect::<Option<Vec<_>>>()?;
        let parsed = ParsedFile {
            entries,
            logged: input.texts()?,
            refused: input.texts()?,
        };
        memo.insert(digest, parsed);
    }
    input.0.is_empty().then_some(memo)
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_names_split_at_the_first_bar() {
        assert_eq!(
            super::icon_lookup("flag_x|12|3"),
            ([b"GFX_flag_x".to_vec(), b"flag_x".to_vec()], Some(0))
        );
        assert_eq!(super::icon_lookup("a|12").1, Some(11));
        assert_eq!(super::icon_lookup("a|").1, Some(-1));
        assert_eq!(super::icon_lookup("a|0").1, Some(-1));
        assert_eq!(super::icon_lookup("a|4294967299").1, Some(2));
        assert_eq!(super::icon_lookup("a|2147483648").1, Some(0));
        assert_eq!(super::icon_lookup("a").1, None);
    }

    #[test]
    fn the_selected_language_follows_the_mod_stacks_language_list() {
        let root = std::env::temp_dir().join(format!("language-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (base, addon) = (root.join("base"), root.join("mod"));
        let write = |folder: &std::path::Path, text: &[u8]| {
            std::fs::create_dir_all(folder.join("localisation")).unwrap();
            std::fs::write(folder.join("localisation/languages.yml"), text).unwrap();
        };
        write(&base, "\u{feff}l_english:\n l_english: \"English\"\nl_french:\n l_english: \"English\"\n".as_bytes());
        let vanilla = ResolvedLayers::from_folders(&[(&base, &[])]).unwrap();
        assert_eq!(selected_language(&vanilla, None).unwrap(), "english");
        assert_eq!(selected_language(&vanilla, Some("l_french")).unwrap(), "french");
        write(&addon, "\u{feff}# list\nl_german:\n l_german: \"Deutsch\"\nl_english:\n".as_bytes());
        let stack = || ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        assert_eq!(declared_languages(&stack()).unwrap(), ["german", "english"]);
        assert_eq!(selected_language(&stack(), None).unwrap(), "german");
        assert_eq!(selected_language(&stack(), Some("l_english")).unwrap(), "english");
        for unlisted in ["l_french", "english", "l_"] {
            assert_eq!(selected_language(&stack(), Some(unlisted)).unwrap(), "german", "{unlisted}");
        }
        for unreadable in [&b"l_german:\n"[..], "\u{feff}l_german\n".as_bytes(), "\u{feff}# none\n".as_bytes()] {
            write(&addon, unreadable);
            let why = selected_language(&stack(), Some("l_english")).unwrap_err().to_string();
            assert!(why.contains("localisation/languages.yml declares no language"), "{why}");
        }
        let bare = root.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let missing = ResolvedLayers::from_folders(&[(&bare, &[])]).unwrap();
        let why = selected_language(&missing, None).unwrap_err().to_string();
        assert!(why.contains("localisation/languages.yml"), "{why}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    use super::*;

    #[test]
    fn a_memoised_read_reads_as_a_cold_one() {
        let root = std::env::temp_dir().join(format!(
            "localisation-memo-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("game");
        let cache = root.join("cache");
        std::fs::create_dir_all(game.join("localisation/english")).unwrap();
        std::fs::create_dir_all(game.join("localisation/french")).unwrap();
        let write = |text: &str| {
            std::fs::write(
                game.join("localisation/english/a_l_english.yml"),
                format!("\u{feff}l_english:\n {text}\n"),
            )
            .unwrap();
        };
        write("key:0 \"first\"\n dup:0 \"one\"");
        std::fs::write(
            game.join("localisation/english/b_l_english.yml"),
            "\u{feff}l_english:\n dup:0 \"two\"\n",
        )
        .unwrap();
        std::fs::write(
            game.join("localisation/french/a_l_french.yml"),
            "\u{feff}l_french:\n key:0 \"premier\"\n",
        )
        .unwrap();
        let (program, build) = build().unwrap();
        let build = *build;
        let memo = cache.join(format!("localisation-english-{program}.bin"));
        let layers = ResolvedLayers::from_folders(&[(&game, &[][..])]).unwrap();
        let cold = read(&layers, "english").unwrap();
        assert_eq!(cold.get("key"), Some("first"));
        assert!(!cold.duplicates.is_empty());
        assert_eq!(read_cached(&layers, "english", &cache).unwrap(), cold);
        let written = std::fs::read(&memo).unwrap();
        assert_eq!(decode_memo(&written, build, "english").unwrap().len(), 3);
        assert_eq!(read_cached(&layers, "english", &cache).unwrap(), cold);
        assert_eq!(std::fs::read(&memo).unwrap(), written);
        let mut damaged = written.clone();
        let middle = damaged.len() / 2;
        damaged[middle] ^= 1;
        assert!(decode_memo(&damaged, build, "english").is_none());
        assert!(decode_memo(&written, build, "french").is_none());
        assert!(decode_memo(&written, (build.0 + 1, build.1), "english").is_none());
        assert!(decode_memo(&written[..written.len() - 1], build, "english").is_none());
        std::fs::write(&memo, &damaged).unwrap();
        assert_eq!(read_cached(&layers, "english", &cache).unwrap(), cold);
        assert_eq!(std::fs::read(&memo).unwrap(), written);
        write("key:0 \"changed\"");
        assert_eq!(
            read_cached(&layers, "english", &cache).unwrap_err().to_string(),
            read(&layers, "english").unwrap_err().to_string()
        );
        let changed = ResolvedLayers::from_folders(&[(&game, &[][..])]).unwrap();
        assert_eq!(
            read_cached(&changed, "english", &cache).unwrap(),
            read(&changed, "english").unwrap()
        );
        let kept = decode_memo(&std::fs::read(&memo).unwrap(), build, "english").unwrap();
        assert_eq!(kept.len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn other_languages_validate_escapes_without_building_values() {
        let body: Vec<_> = "large value \\q \\n \\T".chars().collect();
        let mut selected = Vec::new();
        let mut skipped = Vec::new();
        assert!(!file_value(&body, 5, 8, &mut selected, true).is_empty());
        assert!(file_value(&body, 5, 8, &mut skipped, false).is_empty());
        assert_eq!(selected, skipped);
        assert_eq!(skipped.len(), 1);
    }

    fn parse(text: &str, language: &str) -> ParsedFile {
        parse_file(text.as_bytes(), language)
    }

    #[test]
    fn required_arguments_follow_native_references_escapes_and_limits() {
        let full = format!("{}$ACTOR$", "x".repeat(SUBSTITUTION_BYTES));
        let almost_full = format!("{}$ACTOR$", "x".repeat(SUBSTITUTION_BYTES - 1));
        let loc = loc(&[
            ("plain", "a¤b"),
            ("alias", "$plain$"),
            ("actor", "$ACTOR|format$"),
            ("target", "$TARGET$"),
            ("nested", "$actor$"),
            (
                "ACTOR",
                "an authored key cannot replace the supplied argument",
            ),
            ("escaped", "$$ACTOR$$"),
            ("unclosed", "$ACTOR"),
            ("cycle", "$cycle$"),
            ("full", &full),
            ("almost_full", &almost_full),
        ]);
        for key in ["plain", "alias", "escaped", "unclosed", "cycle", "full", ""] {
            assert!(
                !loc.requires_arguments(key.as_bytes(), &["ACTOR", "TARGET"]),
                "{key}"
            );
        }
        for key in ["actor", "target", "nested", "almost_full"] {
            assert!(
                loc.requires_arguments(key.as_bytes(), &["ACTOR", "TARGET"]),
                "{key}"
            );
        }
        assert!(loc.requires_arguments(b"nested\0ignored", &["ACTOR"]));
        assert!(!loc.requires_arguments(b"target", &["ACTOR"]));
        assert!(!loc.requires_arguments(b"actor", &[]));
    }

    #[test]
    fn deferred_arguments_stop_the_walk_only_where_it_reaches_them() {
        let loc = loc(&[
            ("both", "$FIRST$ and $SECOND|U$"),
            ("first", "only $FIRST$"),
            ("plain", "no $missing$ arguments"),
        ]);
        let first = [("FIRST", Argument::Text("one"))];
        assert_eq!(
            loc.localise_with_deferred(b"both", &first, &["SECOND", "FIRST"]),
            Err(1)
        );
        assert_eq!(
            loc.localise_with_deferred(b"both", &first, &["SECOND"]),
            Err(0)
        );
        assert_eq!(
            loc.localise_with_deferred(b"first", &first, &["SECOND"]),
            Ok(Ok("only one".to_owned()))
        );
        let both = [
            ("FIRST", Argument::Text("one")),
            ("SECOND", Argument::Text("two")),
        ];
        assert_eq!(
            loc.localise_with_deferred(b"both", &both, &[]),
            Ok(loc.localise_with(b"both", &both))
        );
        assert_eq!(
            loc.localise_with_deferred(b"plain", &[], &["FIRST"]),
            Ok(Ok("no  arguments".to_owned()))
        );
        assert_eq!(
            loc.localise_with_deferred(b"", &[], &["FIRST"]),
            Ok(Ok(String::new()))
        );
    }

    #[test]
    fn substitution_recovery_names_missing_keys_references_and_limits() {
        let loc = loc(&[
            ("nested", "$missing$"),
            ("cycle", "$cycle$"),
            ("long", &"x".repeat(SUBSTITUTION_BYTES + 1)),
        ]);
        assert_eq!(
            loc.fallbacks(b"missing", &[]),
            ["localisation missing is missing; the key is shown"]
        );
        assert_eq!(
            loc.fallbacks(b"nested", &[]),
            ["localisation missing is missing; the reference is omitted"]
        );
        assert_eq!(
            loc.fallbacks(b"cycle", &[]),
            ["localisation cycle reached the game's substitution depth limit; text was omitted"]
        );
        assert_eq!(
            loc.fallbacks(b"long", &[]),
            ["localisation long was cut at the game's output buffer limit"]
        );
        assert!(loc.fallbacks(b"", &[]).is_empty());
    }

    #[test]
    fn text_and_recovery_share_the_substitution_result() {
        let loc = loc(&[("nested", "$ARG$:$missing$"), ("cycle", "$cycle$")]);
        let (text, notes) =
            loc.localise_with_fallbacks(b"nested\0ignored", &[("ARG", Argument::Text("mod name"))]);
        assert_eq!(text.unwrap(), "mod name:");
        assert_eq!(
            notes,
            ["localisation missing is missing; the reference is omitted"]
        );
        let (text, notes) = loc.localise_with_fallbacks(b"cycle", &[]);
        assert_eq!(text.unwrap(), "");
        assert_eq!(notes.len(), 1);
        assert!(loc.localise_with_fallbacks(b"", &[]).1.is_empty());
    }

    #[test]
    fn live_substitution_does_not_collect_recovery_notes() {
        let loc = loc(&[
            ("nested", "$ARG$:$missing$"),
            ("cycle", "$cycle$"),
            ("long", &"x".repeat(SUBSTITUTION_BYTES + 1)),
            ("cut", &format!("{}é", "x".repeat(SUBSTITUTION_BYTES - 1))),
        ]);
        let arguments = [("ARG", Argument::Text("mod name"))];
        for key in [
            b"missing".as_slice(),
            b"nested",
            b"cycle",
            b"long",
            b"cut",
            b"\xff",
        ] {
            let live = loc.output(key, &arguments, false);
            assert!(live.fallbacks.is_none());
            assert_eq!(live.bytes, loc.output(key, &arguments, true).bytes);
            assert_eq!(
                loc.localise_with(key, &arguments),
                loc.localise_with_fallbacks(key, &arguments).0
            );
        }
    }

    fn loc(entries: &[(&str, &str)]) -> Localisation {
        entries
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn cause(result: Result<String, Unlocalised>) -> Cause {
        result.unwrap_err().cause
    }

    #[test]
    fn a_missing_key_is_its_own_bytes_and_references_expand_in_the_viewer_language() {
        let loc = loc(&[
            ("GER_rommel", "Erwin $GER_rommel_last$"),
            ("GER_rommel_last", "§YRommel§!"),
            ("SCRIPTED", "[GetName] ]"),
            ("CYCLE", "$CYCLE$"),
            ("DANGLING", "$MISSING$"),
        ]);
        assert_eq!(
            loc.localise(b"GER_rommel\0\0").as_deref(),
            Ok("Erwin \u{11}YRommel\u{11}!")
        );
        assert_eq!(
            loc.localise(b"Unlocalised Name").as_deref(),
            Ok("Unlocalised Name")
        );
        assert_eq!(loc.localise(b"DANGLING").as_deref(), Ok(""));
        assert_eq!(loc.localise(b"CYCLE").as_deref(), Ok(""));
        assert_eq!(loc.localise(b"").as_deref(), Ok(""));
        assert_eq!(loc.localise(b"SCRIPTED").as_deref(), Ok("[GetName] ]"));
        let refused = loc.localise(b"\xff").unwrap_err();
        assert_eq!(refused.cause, Cause::NotText);
        assert_eq!(
            refused.to_string(),
            "localisation \u{fffd}: its text is not UTF-8"
        );
    }

    #[test]
    fn substitution_follows_the_cursor_depth_and_dollar_forms() {
        let mut entries: Vec<(String, String)> = (0..23)
            .map(|index| (format!("A{index}"), format!("$A{}$", index + 1)))
            .collect();
        entries.push(("A23".into(), "x".repeat(3000)));
        entries.push(("LONG".into(), "$A0$$A0$$A0$".into()));
        entries.push(("DOLLARS".into(), "10$$ and $ lone".into()));
        entries.push(("FORMAT".into(), "$A23|H$!".into()));
        for depth in 0..40 {
            entries.push((format!("D{depth}"), format!("{depth} $D{}$", depth + 1)));
        }
        let loc: Localisation = entries.into_iter().collect();
        assert_eq!(loc.localise(b"A0").map(|text| text.len()), Ok(3000));
        assert_eq!(loc.localise(b"LONG").map(|text| text.len()), Ok(0x1800));
        assert_eq!(loc.localise(b"DOLLARS").as_deref(), Ok("10$ and $ lone"));
        assert_eq!(loc.localise(b"FORMAT").as_deref(), Ok("!"));
        let deep = loc.localise(b"D0").unwrap();
        assert!(deep.starts_with("0 1 2 "));
        assert!(deep.ends_with("31 "));
        let key = vec![b'k'; 0x2000];
        assert_eq!(loc.localise(&key).map(|text| text.len()), Ok(0x1800));
    }

    #[test]
    fn an_argument_keeps_its_template_format_outside_the_template() {
        let template = "10$$ off $USES$$BONUS|%0+$ for $LIST|Y$.";
        assert_eq!(argument_format(template, "BONUS"), Some("%0+"));
        assert_eq!(argument_format(template, "USES"), Some(""));
        assert_eq!(argument_format(template, "MISSING"), None);
        let loc = loc(&[("KEY", template)]);
        assert_eq!(
            loc.localise_with(b"KEY", &[("BONUS", Argument::Fixed5(20_000)), ("LIST", Argument::Text("Armor")), ("USES", Argument::Text(""))]).as_deref(),
            Ok("10$ off \u{11}G20%\u{11}! for \u{11}YArmor\u{11}!.")
        );
        assert_eq!(format_argument(Argument::Fixed5(20_000), b"%0+").as_deref(), Ok("\u{11}G20%\u{11}!"));
        assert_eq!(format_argument(Argument::Fixed5(-5_000), b"=%H").as_deref(), Ok("\u{11}H-5.00%\u{11}!"));
        assert_eq!(format_argument(Argument::Fixed5(1_500_000), b"=+0").as_deref(), Ok("\u{11}G+15\u{11}!"));
    }

    #[test]
    fn branching_references_that_write_nothing_are_walked_once_per_depth() {
        let loc = loc(&[
            ("TWICE", "$TWICE$$TWICE$"),
            ("WIDE", "$WIDE$$WIDE$$WIDE$$WIDE$"),
            ("GAPS", "$GAPS$$MISSING$$GAPS$"),
            ("AROUND", "<$TWICE$|$WIDE$|$ONE$>"),
            ("ONE", "1"),
            ("EVERY_DEPTH", "$TWICE$m$EVERY_DEPTH$"),
            ("FILL", "$TWICE$x$FILL$$FILL$"),
            ("LATE", "$TWICE$$TARGET$"),
        ]);
        for key in ["TWICE", "WIDE", "GAPS"] {
            assert_eq!(loc.localise(key.as_bytes()).as_deref(), Ok(""), "{key}");
        }
        let (text, notes) = loc.localise_with_fallbacks(b"AROUND", &[]);
        assert_eq!(text.as_deref(), Ok("<||1>"));
        assert_eq!(
            notes,
            [
                "localisation TWICE reached the game's substitution depth limit; text was omitted",
                "localisation WIDE reached the game's substitution depth limit; text was omitted",
            ]
        );
        assert_eq!(
            loc.fallbacks(b"GAPS", &[]),
            [
                "localisation GAPS reached the game's substitution depth limit; text was omitted",
                "localisation MISSING is missing; the reference is omitted",
                "localisation MISSING reached the game's substitution depth limit; text was omitted",
            ]
        );
        assert_eq!(loc.localise(b"EVERY_DEPTH"), Ok("m".repeat(32)));
        assert_eq!(loc.localise(b"FILL"), Ok("x".repeat(SUBSTITUTION_BYTES)));
        let target = [("TARGET", Argument::Text("t"))];
        assert_eq!(loc.localise_with(b"LATE", &target).as_deref(), Ok("t"));
        assert!(loc.requires_arguments(b"LATE", &["TARGET"]));
        assert_eq!(loc.localise_with_deferred(b"LATE", &[], &["TARGET"]), Err(0));
        let mut entries: Vec<(String, String)> = (1..30)
            .map(|depth| (format!("DEEP{depth}"), format!("$DEEP{}$", depth + 1)))
            .collect();
        entries.push(("DEEP30".into(), "$NEAR$".into()));
        entries.push(("NEAR".into(), "$TEXT$".into()));
        entries.push(("TEXT".into(), "y".into()));
        entries.push(("BOTH".into(), "$DEEP1$$NEAR$".into()));
        let deep: Localisation = entries.into_iter().collect();
        assert_eq!(deep.localise(b"BOTH").as_deref(), Ok("y"));
    }

    #[test]
    fn values_are_stored_with_control_bytes_and_doubled_patterns_are_literal() {
        let parsed = parse(
            "\u{feff}l_english:\n A: \"§Yx§! @GER £icon ¤ {12\"\n B: \"§§ @@ ££ ¤¤ {{\"\n \
             C: \"§§§Y\"\n X: \"key\"\n D: \"[$£X$]\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            vec![
                (
                    "A".into(),
                    "\u{11}Yx\u{11}! \u{12}GER \u{13}icon \u{14} \u{1a}12".to_owned()
                ),
                ("B".into(), "§ @ £ ¤ {".to_owned()),
                ("C".into(), "§\u{11}Y".to_owned()),
                ("X".into(), "key".to_owned()),
                ("D".into(), "[$\u{13}X$]".to_owned()),
            ]
        );
        let loc = Localisation {
            entries: parsed.entries.into_iter().collect(),
            ..Localisation::default()
        };
        assert_eq!(loc.get("X"), Some("key"));
        assert_eq!(loc.localise(b"D").as_deref(), Ok("[]"));
        assert_eq!(loc.localise(b"X").as_deref(), Ok("key"));
    }

    #[test]
    fn the_buffer_is_counted_in_stored_bytes_and_the_cut_is_checked_after() {
        let pad = "x".repeat(SUBSTITUTION_BYTES - 1);
        let tail = |tail: &str| {
            loc(&[("PAD", pad.as_str()), ("T", &format!("$PAD${tail}"))]).localise(b"T")
        };
        for control in ["§", "£", "¤", "@", "{"] {
            let text = tail(&format!("{control}a")).unwrap();
            assert_eq!(text.len(), SUBSTITUTION_BYTES);
            assert!(!text.ends_with('a'));
        }
        assert_eq!(cause(tail("§§")), Cause::CutInsideCharacter);
        assert!(tail("@@").unwrap().ends_with('@'));
        assert_eq!(cause(tail("é")), Cause::CutInsideCharacter);
        assert!(tail("]").unwrap().ends_with(']'));
        let exact = loc(&[("T", &format!("{}é", "x".repeat(0x17FE)))]);
        assert!(exact.localise(b"T").unwrap().ends_with('é'));
        assert_eq!(tail("ab").map(|text| text.len()), Ok(0x1800));
        let empty = Localisation::default();
        let mut key = pad.clone().into_bytes();
        key.extend_from_slice("§".as_bytes());
        assert_eq!(cause(empty.localise(&key)), Cause::CutInsideCharacter);
        let mut key = "x".repeat(SUBSTITUTION_BYTES).into_bytes();
        key.push(0xFF);
        assert_eq!(empty.localise(&key).map(|text| text.len()), Ok(0x1800));
        assert_eq!(cause(empty.localise(b"a\xffb")), Cause::NotText);
    }

    fn with(entry: &str, arguments: &[(&str, Argument<'_>)]) -> Result<String, Unlocalised> {
        loc(&[("KEY", entry), ("NESTED", "<$VALUE|Y$>")]).localise_with(b"KEY", arguments)
    }

    #[test]
    fn arguments_are_formatted_by_type_as_the_formatter_does() {
        let int = |format: &str, value| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Int32(value))],
            )
        };
        let fixed = |format: &str, value| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Fixed5(value))],
            )
        };
        assert_eq!(int("|Y", 5).as_deref(), Ok("\u{11}Y5\u{11}!"));
        assert_eq!(int("|1%", 7).as_deref(), Ok("700%"));
        assert_eq!(int("|%%", 7).as_deref(), Ok("7%"));
        assert_eq!(int("|=", 3).as_deref(), Ok("+3"));
        assert_eq!(int("|=", 0).as_deref(), Ok("0"));
        assert_eq!(int("|_", 0).as_deref(), Ok(""));
        assert_eq!(int("|+", 2).as_deref(), Ok("\u{11}G2\u{11}!"));
        assert_eq!(int("|-", 2).as_deref(), Ok("\u{11}R2\u{11}!"));
        assert_eq!(int("|+", -2).as_deref(), Ok("\u{11}R-2\u{11}!"));
        assert_eq!(int("|+", 0).as_deref(), Ok("\u{11}Y0\u{11}!"));
        assert_eq!(fixed("|R0", 36_958_320).as_deref(), Ok("\u{11}R369\u{11}!"));
        assert_eq!(fixed("", 199_999).as_deref(), Ok("1.99"));
        assert_eq!(fixed("", -150_000).as_deref(), Ok("-1.50"));
        assert_eq!(fixed("|1", 100_000).as_deref(), Ok("1.0"));
        assert_eq!(fixed("|~", 100_000).as_deref(), Ok("1"));
        assert_eq!(fixed("|3~", 150_000).as_deref(), Ok("1.5"));
        assert_eq!(fixed("|6~", 150_000).as_deref(), Ok("1.50000"));
        assert_eq!(fixed("|9", 150_000).as_deref(), Ok("1.50000"));
        assert_eq!(fixed("|%0", 12_345).as_deref(), Ok("12%"));
        let text = |format: &str, value| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Text(value))],
            )
        };
        assert_eq!(text("", "3 ").as_deref(), Ok("3 "));
        assert_eq!(text("|U", "abc").as_deref(), Ok("Abc"));
        assert_eq!(text("|l+%", "ABC").as_deref(), Ok("aBC"));
        assert_eq!(text("|U", "émile").as_deref(), Ok("émile"));
        assert_eq!(text("|H", "").as_deref(), Ok("\u{11}H\u{11}!"));
        assert_eq!(
            with("$NESTED$", &[("VALUE", Argument::Int32(1))]).as_deref(),
            Ok("<\u{11}Y1\u{11}!>")
        );
        assert_eq!(fixed("|7~", 150_000).as_deref(), Ok("1.50000"));
        assert_eq!(fixed("|7~", 123_450).as_deref(), Ok("1.23450"));
        assert_eq!(fixed("|9~", 100_000).as_deref(), Ok("1"));
    }

    #[test]
    fn float_arguments_round_promoted_values_and_multiply_percent_in_single_precision() {
        let float = |format: &str, value: f32| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Float32(value.to_bits()))],
            )
            .unwrap()
        };
        for (value, shown) in [(1.125, "1.12"), (1.375, "1.38"), (-1.125, "-1.12")] {
            assert_eq!(float("", value), shown);
        }
        assert_eq!(float("|0", 2.5), "2");
        assert_eq!(float("|0", 3.5), "4");
        assert_eq!(float("|6", 0.29), "0.290000");
        assert_eq!(float("|6%", 0.29), "29.000000%");
        assert_eq!(float("|6%%", 0.29), "0.290000%");
        assert_eq!(float("|3%", 0.125), "12.500%");
        assert_eq!(float("|*^~1%", 1500.0), "150000.0%");
        assert_eq!(float("|^~3", 1.5), "1.500");
        assert_eq!(float("|61", 1.5), "1.5");
        assert_eq!(
            float("|6", f32::MAX),
            "340282346638528859811704183484516925440.000000"
        );
        assert_eq!(float("|%", f32::MAX), "inf%");
        assert_eq!(float("|%", -f32::MAX), "-inf%");
    }

    #[test]
    fn float_argument_signs_zero_nan_and_infinity_follow_the_native_comparisons() {
        let float = |format: &str, bits: u32| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Float32(bits))],
            )
            .unwrap()
        };
        assert_eq!(float("", (-0.0_f32).to_bits()), "-0.00");
        assert_eq!(float("|+=", (-0.0_f32).to_bits()), "\u{11}Y-0.00\u{11}!");
        assert_eq!(float("|_", (-0.0_f32).to_bits()), "");
        assert_eq!(float("|Y_", 0), "\u{11}Y\u{11}!");
        assert_eq!(float("|_+=", 1), "\u{11}G+0.00\u{11}!");
        assert_eq!(float("|_+=", 0x8000_0001), "\u{11}R-0.00\u{11}!");
        assert_eq!(float("|+=", f32::INFINITY.to_bits()), "\u{11}G+inf\u{11}!");
        assert_eq!(
            float("|-=", f32::NEG_INFINITY.to_bits()),
            "\u{11}G-inf\u{11}!"
        );
        for (bits, shown) in [
            (0x7fc0_0000, "nan"),
            (0xffc0_0000, "-nan(ind)"),
            (0xffc0_0001, "-nan"),
            (0x7f80_0001, "nan"),
            (0xff80_0001, "-nan"),
        ] {
            assert_eq!(float("", bits), shown);
            assert_eq!(float("|%", bits), format!("{shown}%"));
            assert_eq!(float("|+=", bits), format!("\u{11}Y{shown}\u{11}!"));
            assert_eq!(float("|_", bits), "");
        }
    }

    #[test]
    fn float_undefined_precision_refuses_only_when_the_argument_is_demanded() {
        let value = [("VALUE", Argument::Float32(0))];
        let localisation = loc(&[("KEY", "before $VALUE|7$ after $VALUE|7$")]);
        let (text, notes) = localisation.localise_with_fallbacks(b"KEY", &value);
        assert!(
            text.as_ref()
                .unwrap_err()
                .to_string()
                .contains("Float32 precision 7")
        );
        assert_eq!(cause(text), Cause::Float32Precision { precision: 7 });
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("Float32 precision 7"));
        assert_eq!(
            localisation
                .localise_bytes(b"KEY", &value)
                .unwrap_err()
                .cause,
            Cause::Float32Precision { precision: 7 }
        );
        for precision in 7..=9 {
            assert_eq!(
                cause(with(&format!("$VALUE|{precision}_$"), &value)),
                Cause::Float32Precision { precision }
            );
        }
        assert_eq!(
            localisation.localise(b"KEY").as_deref(),
            Ok("before  after ")
        );
        let pad = "x".repeat(SUBSTITUTION_BYTES);
        let full = loc(&[("KEY", &format!("{pad}$VALUE|9$"))]);
        assert_eq!(full.localise_with(b"KEY", &value), Ok(pad));
        let bytes = Localisation::default();
        assert_eq!(bytes.localise_bytes(b"\xff", &[]), Ok(vec![0xff]));
    }

    #[test]
    fn float_arguments_use_the_shared_formatted_output_boundaries() {
        let arguments = [("VALUE", Argument::Float32(1.5_f32.to_bits()))];
        let pad = "x".repeat(SUBSTITUTION_BYTES - 4);
        let loc = loc(&[("KEY", &format!("{pad}$VALUE|2$"))]);
        let (text, notes) = loc.localise_with_fallbacks(b"KEY", &arguments);
        assert_eq!(text.unwrap(), format!("{pad}1.5 "));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn scaled_forms_divide_by_powers_of_a_thousand() {
        let int = |format: &str, value| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Int32(value))],
            )
            .unwrap()
        };
        let fixed = |format: &str, value| {
            with(
                &format!("$VALUE{format}$"),
                &[("VALUE", Argument::Fixed5(value))],
            )
            .unwrap()
        };
        for (format, value, shown) in [
            ("|^", 999, "999"),
            ("|^", 1000, "1.00K"),
            ("|^0", 1999, "1K"),
            ("|^", 1999, "1.99K"),
            ("|^", 1005, "1.00K"),
            ("|^", 999_999, "999.99K"),
            ("|^4", 1234, "1.234K"),
            ("|^9", 1500, "1.500K"),
            ("|^4", 1_234_567, "1.2345M"),
            ("|^", 1_000_000_000, "1.00B"),
            ("|*", 1_000_000_000, "1.00G"),
            ("|*^", 1_000_000_000, "1.00G"),
            ("|^4", i32::MAX, "2.1474B"),
            ("|^", -1500, "-1.50K"),
            ("|^", i32::MIN, "--2147483648"),
            ("|^%", 1500, "1.50K%"),
            ("|^+=", 1500, "\u{11}G+1.50K\u{11}!"),
            ("|^_", 0, ""),
        ] {
            assert_eq!(int(format, value), shown, "{format} {value}");
        }
        for (format, value, shown) in [
            ("|^", 99_999_999, "999.99"),
            ("|^", 100_000_000, "1.00K"),
            ("|^", 123_456_789_000, "1.23M"),
            ("|^~", 50_000, "0.50"),
            ("|^0", 50_000, "0"),
            ("|^9", 150_000, "1.50000"),
            ("|^", ((1i64 << 32) + 5) * 100_000, "5.00"),
            ("|^", (1i64 << 31) * 100_000, "-2147483648.00"),
            ("|^", ((1i64 << 32) + 1500) * 100_000, "1.50K"),
            ("|^+", 5, "\u{11}G0.00\u{11}!"),
        ] {
            assert_eq!(fixed(format, value), shown, "{format} {value}");
        }
    }

    #[test]
    fn colour_writes_follow_the_order_at_the_buffer_end() {
        let at_end = |room: usize, entry: &str, value| {
            let pad = "x".repeat(SUBSTITUTION_BYTES - room);
            loc(&[("PAD", pad.as_str()), ("KEY", &format!("$PAD${entry}"))])
                .localise_with(b"KEY", &[("VALUE", Argument::Int32(value))])
                .map(|text| text[SUBSTITUTION_BYTES - room..].to_owned())
        };
        assert_eq!(at_end(1, "$VALUE|Y$", 5).as_deref(), Ok("\u{11}"));
        assert_eq!(at_end(2, "$VALUE|Y$", 5).as_deref(), Ok("\u{11}Y"));
        assert_eq!(at_end(3, "$VALUE|Y$", 55).as_deref(), Ok("\u{11}Y "));
        assert_eq!(at_end(4, "$VALUE|Y$", 5).as_deref(), Ok("\u{11}Y5\u{11}"));
        assert_eq!(at_end(5, "$VALUE|Y$", 5).as_deref(), Ok("\u{11}Y5\u{11}!"));
        assert_eq!(at_end(4, "$VALUE|+$", 5).as_deref(), Ok("\u{11}G5 "));
        assert_eq!(at_end(6, "$VALUE|+$", 5).as_deref(), Ok("\u{11}G5\u{11}!"));
    }

    #[test]
    fn stored_text_splits_into_the_pieces_the_text_layout_reads() {
        use Piece::*;
        assert_eq!(
            pieces(
                "a\u{11}Yb\u{11}!\u{12}GERc\u{13}pol_power d\u{13}x|2\u{13}e\u{14}\u{1a}12f\u{11}"
            ),
            vec![
                Text("a"),
                Colour('Y'),
                Text("b"),
                Colour('!'),
                Flag("GER"),
                Text("c"),
                Icon("pol_power"),
                Text("d"),
                Icon("x|2"),
                Text("e"),
                Control14,
                Control1A("12"),
                Text("f"),
            ]
        );
        let long = format!("\u{13}{} z", "n".repeat(128));
        assert_eq!(pieces(&long)[1], Text(" z"));
        assert_eq!(pieces("\u{12}GE"), vec![Flag("GE")]);
        assert_eq!(pieces("é§"), vec![Text("é§")]);
    }

    #[test]
    fn entries_keep_variables_and_decode_escapes_once() {
        let parsed = parse(
            "\u{feff}l_english:\n STYLE_SAMPLE:0 \"§YHarbor§! Sector\"\n \
             A: \"$OVERLORDADJ$ Zone\\nsecond\"\n B:12 \"x\"\nNOSPACE: \"y\"\n \
             C: \"a\\\\n b\\\\\\\\n\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            vec![
                (
                    "STYLE_SAMPLE".into(),
                    "\u{11}YHarbor\u{11}! Sector".to_owned()
                ),
                ("A".into(), "$OVERLORDADJ$ Zone\nsecond".to_owned()),
                ("B".into(), "x".to_owned()),
                ("NOSPACE".into(), "y".to_owned()),
                ("C".into(), "a\\n b\\\\n".to_owned()),
            ]
        );
        assert!(parsed.logged.is_empty());
    }

    #[test]
    fn the_last_quote_delimits_even_a_quote_in_a_comment() {
        let parsed = parse(
            "\u{feff}l_english:\n \
             A: \"The sign reads \"Studio East\"\"\n \
             B: \"Paint the door violet.\" #colour labels read \"Violet\"\n \
             C: \"#\"\n D: \"say \\\"hi\\\" now\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            vec![
                ("A".into(), "The sign reads \"Studio East\"".to_owned()),
                (
                    "B".into(),
                    "Paint the door violet.\" #colour labels read \"Violet".to_owned()
                ),
                ("C".into(), "#".to_owned()),
                ("D".into(), "say \"hi\" now".to_owned()),
            ]
        );
    }

    #[test]
    fn only_the_requested_language_is_kept_across_several_headers() {
        let text = "\u{feff}# leading comment\nl_english:\n A: \"one\"\nl_french:\n A: \"un\"\n\
                    l_english:\n B: \"two\"\n";
        assert_eq!(
            parse(text, "english").entries,
            vec![
                ("A".into(), "one".to_owned()),
                ("B".into(), "two".to_owned())
            ]
        );
        assert_eq!(
            parse(text, "french").entries,
            vec![("A".into(), "un".to_owned())]
        );
        assert!(parse(text, "german").entries.is_empty());
    }

    #[test]
    fn an_unclosed_value_is_empty_and_a_bad_following_line_stops_the_file() {
        let parsed = parse(
            "\u{feff}l_english:\n A: \"unterminated\n continued.\"\n B: \"good\"\n C: bare\n",
            "english",
        );
        assert_eq!(parsed.entries, vec![("A".into(), String::new())]);
        assert_eq!(
            parsed.logged,
            ["3: key is missing its colon at column 12; remainder skipped"]
        );
        let orphan = parse("\u{feff} A: \"x\"\nl_english:\n B: \"y\"\n", "english");
        assert!(orphan.entries.is_empty());
        assert_eq!(orphan.refused.len(), 1);
    }

    #[test]
    fn values_ignore_text_after_the_last_quote() {
        for suffix in ["\u{60}", ".", "*", "trailing_marker"] {
            let file = format!("\u{feff}l_english:\n A:0 \"value\\n\\n\"{suffix}\n B: \"next\"\n");
            let parsed = parse(&file, "english");
            assert_eq!(
                parsed.entries,
                [
                    ("A".into(), "value\n\n".into()),
                    ("B".into(), "next".into())
                ]
            );
            assert!(parsed.logged.is_empty());
        }
    }

    #[test]
    fn unclosed_quotes_store_empty_without_swallowing_the_next_entry() {
        for value in ["unfinished", ""] {
            let file = format!("\u{feff}l_english:\n OPEN_LABEL: \"{value}\n NEXT: \"next\"\n");
            let parsed = parse(&file, "english");
            assert_eq!(
                parsed.entries,
                [
                    ("OPEN_LABEL".into(), "".into()),
                    ("NEXT".into(), "next".into())
                ]
            );
            assert!(parsed.logged.is_empty());
        }
    }

    #[test]
    fn headers_accept_a_version_and_any_bare_key_selects_a_table() {
        let parsed = parse(
            "\u{feff} l_english:0\n A: \"one\"\n l_french:12\n A: \"un\"\n arbitrary:42\n B: \"elsewhere\"\n l_english:7 \r\n C: \"two\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            [("A".into(), "one".into()), ("C".into(), "two".into())]
        );
        assert!(parsed.logged.is_empty());
    }

    #[test]
    fn doubled_bom_logs_an_invalid_key_and_stops_the_file() {
        let parsed = parse(
            "\u{feff}\u{feff}l_french:\n A: \"un\"\n l_french:\n B: \"deux\"\n",
            "french",
        );
        assert!(parsed.entries.is_empty());
        assert_eq!(
            parsed.logged,
            ["1: invalid localisation key at column 1; remainder skipped"]
        );
    }

    #[test]
    fn trailing_escaped_quote_is_the_delimiter_and_keeps_the_backslash() {
        let parsed = parse(
            "\u{feff}l_english:\n A: \"Silver Kite\\\"\n B: \"next\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            [
                ("A".into(), "Silver Kite\\".into()),
                ("B".into(), "next".into())
            ]
        );
        assert!(parsed.logged.is_empty());
    }

    #[test]
    fn token_before_the_quote_logs_and_stops_the_file() {
        let parsed = parse(
            "\u{feff}l_spanish:\n A:0 £synthetic_marker  \"Entry cost\"\n B: \"next\"\n",
            "spanish",
        );
        assert!(parsed.entries.is_empty());
        assert_eq!(
            parsed.logged,
            ["2: value is missing its opening quote at column 6; remainder skipped"]
        );
    }

    #[test]
    fn escapes_decode_case_variants_tabs_and_log_unknown_characters() {
        let parsed = parse(
            "\u{feff}l_english:\n A: \"x\\N\\n\\T\\t\\q\\\\n\\\"y\"\n",
            "english",
        );
        assert_eq!(parsed.entries, [("A".into(), "x\n\n\t\tq\\n\"y".into())]);
        assert_eq!(
            parsed.logged,
            ["2: unsupported escape character 113 at column 16; escape kept without backslash"]
        );
    }

    #[test]
    fn missing_bom_logs_twice_and_empty_files_are_not_read() {
        let parsed = parse("l_english:\n A: \"one\"\n", "english");
        assert!(parsed.entries.is_empty());
        assert_eq!(parsed.logged.len(), 2);
        assert!(parsed.logged[1].contains("leading UTF-8 BOM is missing"));
        assert_eq!(parse_file(&[], "english"), ParsedFile::default());
        assert_eq!(
            parse_file(b"\xef\xbb\xbf", "english"),
            ParsedFile::default()
        );
    }

    #[test]
    fn failed_utf8_conversion_reuses_the_previous_decoded_line() {
        let parsed = parse_file(
            b"\xef\xbb\xbfl_english:\n A: \"one\"\n\xff\xff\n B: \"two\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            [
                ("A".into(), "one".into()),
                ("A".into(), "one".into()),
                ("B".into(), "two".into())
            ]
        );
        assert_eq!(
            parsed.logged,
            ["3: line is not valid UTF-8; previous decoded line reused"]
        );
    }

    #[test]
    fn failed_conversion_can_skip_the_reused_line_by_its_raw_byte_count() {
        let parsed = parse_file(
            b"\xef\xbb\xbfl_english:\n A: \"one\"\n\xff\n B: \"two\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            [("A".into(), "one".into()), ("B".into(), "two".into())]
        );
        assert_eq!(parsed.logged.len(), 1);
    }

    #[test]
    fn key_and_space_tests_are_not_rust_whitespace_or_yaml() {
        let parsed = parse(
            "\u{feff}l_english:\n\u{1c}\u{a0}0'A-.z_:0\u{2007}\"yes\"\n BAD : \"no\"\n NEXT: \"unread\"\n",
            "english",
        );
        assert_eq!(parsed.entries, [("0'A-.z_".into(), "yes".into())]);
        assert_eq!(
            parsed.logged,
            ["3: key is missing its colon at column 5; remainder skipped"]
        );
        for rejected in ["£X", "é", "+A", "A/B", "A\u{85}"] {
            let parsed = parse(
                &format!("\u{feff}l_english:\n {rejected}: \"x\"\n NEXT: \"unread\"\n"),
                "english",
            );
            assert!(parsed.entries.is_empty(), "{rejected}");
            assert_eq!(parsed.logged.len(), 1, "{rejected}");
        }
        let parsed = parse("\u{feff}l_english:\n\u{a0}\n A: \"unread\"\n", "english");
        assert!(parsed.entries.is_empty());
        assert!(parsed.logged[0].contains("invalid localisation key at column 2"));
    }

    #[test]
    fn every_base_file_loads_before_every_replace_file_whatever_its_layer() {
        assert!(!is_replace("localisation/english/a_l_english.yml"));
        assert!(is_replace("localisation/replace/english/a.yml"));
        assert!(is_replace("localisation/Replace/a.yml"));
        assert!(!is_replace("localisation/replaced/a.yml"));
        let mut files = [
            load_order("localisation/english/b.yml", 1),
            load_order("localisation/replace/english/a.yml", 0),
            load_order("localisation/replace/english/z.yml", 1),
            load_order("localisation/english/a.yml", 1),
            load_order("localisation/english/c.yml", 0),
        ];
        files.sort();
        assert_eq!(
            files
                .iter()
                .map(|file| std::str::from_utf8(&file.2).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "localisation/english/c.yml",
                "localisation/english/a.yml",
                "localisation/english/b.yml",
                "localisation/replace/english/a.yml",
                "localisation/replace/english/z.yml",
            ]
        );
    }

    fn merged(files: &[(&str, usize, &str)]) -> Localisation {
        let mut ordered = files
            .iter()
            .map(|(path, layer, text)| (load_order(path, *layer), *text))
            .collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        let mut localisation = Localisation {
            language: "english".into(),
            ..Localisation::default()
        };
        let mut sources = BTreeMap::new();
        for ((replace, _, path), text) in ordered {
            merge_file(
                &mut localisation,
                &mut sources,
                replace,
                &String::from_utf8_lossy(&path),
                parse(&format!("\u{feff}{text}"), "english"),
            );
        }
        localisation
    }

    #[test]
    fn two_base_files_keep_the_first_definition_and_record_the_conflict() {
        let localisation = merged(&[
            (
                "localisation/english/b.yml",
                1,
                "l_english:\n A: \"late\"\n",
            ),
            (
                "localisation/english/a.yml",
                0,
                "l_english:\n A: \"early\"\n B: \"only\"\n",
            ),
        ]);
        assert_eq!(localisation.get("A"), Some("early"));
        assert_eq!(localisation.get("B"), Some("only"));
        assert_eq!(
            localisation.duplicates,
            vec!["A localisation/english/a.yml localisation/english/b.yml"]
        );
        let repeated = merged(&[(
            "localisation/english/a.yml",
            0,
            "l_english:\n A: \"one\"\n A: \"two\"\n",
        )]);
        assert_eq!(repeated.get("A"), Some("one"));
        assert_eq!(repeated.get("B"), None);
        assert_eq!(repeated.len(), 1);
        assert!(!repeated.is_empty());
        assert_eq!(repeated.iter().collect::<Vec<_>>(), vec![("A", "one")]);
        assert_eq!(repeated.duplicates.len(), 1);
    }

    #[test]
    fn a_kept_localisation_reads_in_place_as_its_files_were_read() {
        let read = merged(&[
            (
                "localisation/english/b.yml",
                1,
                "l_english:\n A: \"late\"\n NAME: \"$A$ and §Yyellow§!\"\n",
            ),
            (
                "localisation/english/a.yml",
                0,
                "l_english:\n A: \"early\"\n B: \"\"\n C: \"bad \\q escape\"\n",
            ),
        ]);
        assert!(!read.duplicates.is_empty() && !read.logged.is_empty());
        let mut out = b"before".to_vec();
        read.keep(&mut out);
        let end = out.len();
        out.extend(b"after");
        let bytes = std::sync::Arc::new(out);
        let (kept, at) = Localisation::from_kept(&bytes, 6).unwrap();
        assert_eq!(at, end);
        assert_eq!(kept, read);
        for key in ["A", "B", "C", "NAME", "MISSING", ""] {
            assert_eq!(kept.get(key), read.get(key), "{key}");
            assert_eq!(kept.localise(key.as_bytes()), read.localise(key.as_bytes()), "{key}");
        }
        assert!(Localisation::from_kept(&std::sync::Arc::new(bytes[..end - 1].to_vec()), 6).is_err());
        let mut grown = kept;
        grown.entries.insert("D".into(), "late key".into());
        assert_eq!((grown.get("A"), grown.get("D"), grown.len()), (Some("early"), Some("late key"), read.len() + 1));
    }

    #[test]
    fn a_replace_file_overwrites_silently_even_from_an_earlier_layer() {
        let localisation = merged(&[
            (
                "localisation/replace/english/a.yml",
                0,
                "l_english:\n A: \"replaced\"\n",
            ),
            (
                "localisation/english/a.yml",
                1,
                "l_english:\n A: \"base\"\n",
            ),
        ]);
        assert_eq!(localisation.get("A"), Some("replaced"));
        assert!(localisation.duplicates.is_empty());
        let both = merged(&[
            (
                "localisation/replace/english/z.yml",
                0,
                "l_english:\n A: \"second\"\n",
            ),
            (
                "localisation/replace/english/a.yml",
                0,
                "l_english:\n A: \"first\"\n",
            ),
        ]);
        assert_eq!(both.get("A"), Some("second"));
        assert!(both.duplicates.is_empty());
    }

    #[test]
    fn an_unusable_language_name_is_refused_before_any_file_is_read() {
        let layers = ResolvedLayers::default();
        for language in ["", "English", "en-gb", "../x"] {
            assert!(read(&layers, language).is_err(), "{language}");
        }
        assert!(read(&layers, "english").unwrap().is_empty());
    }

    #[test]
    fn an_archive_file_whose_name_is_not_utf8_loads() {
        use content_reader_layers::test_support::{Record, zip_records};
        let root = std::env::temp_dir().join(format!(
            "hoi4_localisation_raw_name_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("raw.zip");
        let text = "\u{feff}l_english:\n RAW: \"raw\"\n";
        std::fs::write(
            &archive,
            zip_records(
                &[],
                &[Record {
                    name: b"localisation/\x80.yml",
                    host: 0,
                    flags: 0,
                    deflate: false,
                    data: text.as_bytes(),
                }],
            ),
        )
        .unwrap();
        let layers = ResolvedLayers::from_folders(&[(&archive, &[][..])]).unwrap();
        let localisation = read(&layers, "english").unwrap();
        assert_eq!(localisation.get("RAW"), Some("raw"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn many_files_merge_in_load_order_whichever_worker_reads_them() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_localisation_many_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("localisation/replace")).unwrap();
        for file in 0..200 {
            std::fs::write(
                root.join(format!("localisation/f{file:03}_l_english.yml")),
                format!("\u{feff}l_english:\n SHARED: \"{file}\"\n OWN_{file}: \"own {file}\"\n"),
            )
            .unwrap();
        }
        std::fs::write(
            root.join("localisation/replace/r_l_english.yml"),
            "\u{feff}l_english:\n REPLACED: \"replaced\"\n OWN_7: \"seven\"\n",
        )
        .unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let localisation = read(&layers, "english").unwrap();
        assert_eq!(localisation.get("SHARED"), Some("0"), "the first file's definition wins");
        for file in (0..200).filter(|file| *file != 7) {
            let own = format!("own {file}");
            assert_eq!(localisation.get(&format!("OWN_{file}")), Some(own.as_str()));
        }
        assert_eq!(localisation.get("OWN_7"), Some("seven"), "replace/ overwrites");
        assert_eq!(localisation.duplicates.len(), 199, "every later SHARED is a duplicate");
        assert_eq!(localisation.get("REPLACED"), Some("replaced"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

pub fn icon_lookup(name: &str) -> ([Vec<u8>; 2], Option<i32>) {
    let (stem, suffix) = match name.split_once('|') {
        Some((stem, suffix)) => (stem, Some(suffix)),
        None => (name, None),
    };
    let frame = suffix.map(|suffix| {
        let mut value: i32 = 0;
        for byte in suffix.bytes() {
            if !byte.is_ascii_digit() {
                return 0;
            }
            value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        }
        if value < 0 { 0 } else { value - 1 }
    });
    (
        [format!("GFX_{stem}").into_bytes(), stem.as_bytes().to_vec()],
        frame,
    )
}
