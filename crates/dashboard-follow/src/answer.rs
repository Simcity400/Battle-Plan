
use crate::invalid;
use replay_data::Hour;
use std::{collections::HashMap, io};

pub const MAGIC: u32 = u32::from_le_bytes(*b"FOL3");

pub const NO_REASON: u32 = u32::MAX;

pub(crate) const NOT_RECORDED: u8 = 0;
pub(crate) const ROWS: u8 = 1;
pub(crate) const LOG: u8 = 2;
pub(crate) const CHANGED: u8 = 3;

const RESET: u32 = 1;
pub(crate) const CANONICAL: u8 = 1;
pub const ROUTES: u32 = u32::MAX >> 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Value = 0,
    Missing = 1,
    Unrecorded = 2,
    NoRow = 3,
}

impl State {
    fn from(byte: u8) -> io::Result<Self> {
        Ok(match byte {
            0 => Self::Value,
            1 => Self::Missing,
            2 => Self::Unrecorded,
            3 => Self::NoRow,
            _ => return Err(invalid(format!("No cell state {byte}"))),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub hour: Hour,
    pub reset: bool,
    pub answers: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Table {
    pub(crate) columns: usize,
    pub(crate) log: bool,
    pub(crate) keys: Vec<u128>,
    pub(crate) values: Vec<i64>,
    pub(crate) states: Vec<u8>,
    pub(crate) reasons: Vec<u32>,
    pub(crate) names: Vec<u32>,
    pub(crate) hours: Vec<Hour>,
    pub(crate) texts: Vec<String>,
    held: HashMap<String, u32>,
}

impl Table {
    pub(crate) fn clear(&mut self, columns: usize, log: bool) {
        self.columns = columns;
        self.log = log;
        self.keys.clear();
        self.values.clear();
        self.states.clear();
        self.reasons.clear();
        self.names.clear();
        self.hours.clear();
        self.texts.clear();
        self.held.clear();
    }

    pub(crate) fn rows(&self) -> usize {
        self.keys.len()
    }

    pub(crate) fn row(&mut self, key: u128) {
        self.keys.push(key);
        self.reasons.push(NO_REASON);
        self.names.push(NO_REASON);
    }

    pub(crate) fn cell(&mut self, value: i64, state: State) {
        self.values.push(value);
        self.states.push(state as u8);
    }

    pub(crate) fn absent(&mut self, reason: &str) {
        for _ in 0..self.columns {
            self.cell(0, State::NoRow);
        }
        self.explain(reason);
    }

    pub(crate) fn explain(&mut self, reason: &str) {
        let at = self.text(reason);
        *self.reasons.last_mut().expect("a row") = at;
    }

    pub(crate) fn name(&mut self, name: &str) {
        let at = self.text(name);
        *self.names.last_mut().expect("a row") = at;
    }

    pub(crate) fn text(&mut self, text: &str) -> u32 {
        if let Some(&at) = self.held.get(text) {
            return at;
        }
        let at = u32::try_from(self.texts.len()).ok().filter(|&at| at != NO_REASON).expect("an answer's texts are fewer than its rows' u32 count");
        self.texts.push(text.to_owned());
        self.held.insert(text.to_owned(), at);
        at
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        self.encode_kind(if self.log { LOG } else { ROWS }, out);
    }

    pub(crate) fn encode_changed(&self, out: &mut Vec<u8>) {
        self.encode_kind(CHANGED, out);
    }

    fn reason_of(&self, row: usize) -> Option<&str> {
        self.texts.get(self.reasons[row] as usize).map(String::as_str)
    }

    pub(crate) fn set_row(&mut self, row: usize, cells: &[(i64, State)], reason: Option<&str>) -> bool {
        let same_reason = self.reason_of(row) == reason;
        let at = row * self.columns;
        let mut changed = !same_reason;
        for (offset, &(value, state)) in cells.iter().enumerate() {
            if self.values[at + offset] != value || self.states[at + offset] != state as u8 {
                self.values[at + offset] = value;
                self.states[at + offset] = state as u8;
                changed = true;
            }
        }
        if !same_reason {
            let text = reason.map_or(NO_REASON, |reason| self.text(reason));
            self.reasons[row] = text;
        }
        changed
    }

    pub(crate) fn rows_into(&self, rows: &[u32], out: &mut Self) {
        out.clear(self.columns, false);
        for &row in rows {
            let (row, width) = (row as usize, self.columns);
            out.row(self.keys[row]);
            out.values.extend_from_slice(&self.values[row * width..(row + 1) * width]);
            out.states.extend_from_slice(&self.states[row * width..(row + 1) * width]);
            if let Some(reason) = self.reason_of(row) {
                out.explain(reason);
            }
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.keys.capacity() * 16 + self.values.capacity() * 8 + self.states.capacity() + (self.reasons.capacity() + self.names.capacity()) * 4 + self.hours.capacity() * 4 + self.texts.iter().map(|text| text.capacity() * 2).sum::<usize>() + self.held.capacity() * (size_of::<String>() + 4 + 1)
    }

    pub(crate) fn encoded_len(&self) -> usize {
        let (rows, up) = (self.keys.len(), |at: usize| at.next_multiple_of(8));
        16 + rows * 16 + self.values.len() * 8 + up(self.states.len()) + up(rows * 4) * 2 + up((self.texts.len() + 1) * 4) + up(self.texts.iter().map(String::len).sum())
    }

    pub(crate) fn canonical(&self) -> bool {
        !self.log && self.keys.windows(2).all(|pair| pair[0] < pair[1])
    }

    fn encode_kind(&self, kind: u8, out: &mut Vec<u8>) {
        let log = kind == LOG;
        let rows = self.keys.len();
        let flags = if kind != LOG && self.canonical() { CANONICAL } else { 0 };
        head(out, kind, flags, rows, self.columns, self.texts.len());
        out.reserve(rows * (16 + 8) + self.values.len() * 9 + 40);
        self.keys.iter().for_each(|&key| out.extend_from_slice(&(key as u64).to_le_bytes()));
        self.keys.iter().for_each(|&key| out.extend_from_slice(&((key >> 64) as u64).to_le_bytes()));
        self.values.iter().for_each(|value| out.extend_from_slice(&value.to_le_bytes()));
        if log {
            self.hours.iter().for_each(|hour| out.extend_from_slice(&hour.to_le_bytes()));
            pad(out);
        }
        out.extend_from_slice(&self.states);
        pad(out);
        self.reasons.iter().for_each(|reason| out.extend_from_slice(&reason.to_le_bytes()));
        pad(out);
        self.names.iter().for_each(|name| out.extend_from_slice(&name.to_le_bytes()));
        pad(out);
        texts(out, self.texts.iter().map(String::as_str));
    }
}

fn head(out: &mut Vec<u8>, kind: u8, flags: u8, rows: usize, columns: usize, texts: usize) {
    out.extend_from_slice(&[kind, flags, 0, 0]);
    out.extend_from_slice(&(rows as u32).to_le_bytes());
    out.extend_from_slice(&(columns as u32).to_le_bytes());
    out.extend_from_slice(&(texts as u32).to_le_bytes());
}

fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(8), 0);
}

fn texts<'a>(out: &mut Vec<u8>, texts: impl Iterator<Item = &'a str> + Clone) {
    let mut end = 0u32;
    out.extend_from_slice(&end.to_le_bytes());
    for text in texts.clone() {
        end += u32::try_from(text.len()).expect("a followed text is shorter than 4 GiB");
        out.extend_from_slice(&end.to_le_bytes());
    }
    pad(out);
    for text in texts {
        out.extend_from_slice(text.as_bytes());
    }
    pad(out);
}

pub fn not_recorded(reason: &str, out: &mut Vec<u8>) {
    head(out, NOT_RECORDED, 0, 0, 0, 1);
    texts(out, std::iter::once(reason));
}

pub fn answers_cells(message: &[u8]) -> bool {
    let mut at = 24;
    while let Some(header) = message.get(at..).and_then(|tail| tail.get(..16)) {
        if u32::from_le_bytes(header[..4].try_into().expect("answer id")) >= crate::live::CELLS { return true; }
        at += 16 + u32::from_le_bytes(header[8..12].try_into().expect("answer length")) as usize;
    }
    false
}

pub fn resets(message: &[u8]) -> bool {
    message.len() >= 8 && message.starts_with(&MAGIC.to_le_bytes()) && u32::from_le_bytes(message[4..8].try_into().expect("four bytes")) & RESET != 0
}

pub fn begin(out: &mut Vec<u8>, route: u32, clock: i32, hour: Hour, seek: u32, reset: bool) {
    debug_assert!(route <= ROUTES, "a route fits above the reset bit");
    out.clear();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&(route << 1 | if reset { RESET } else { 0 }).to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&seek.to_le_bytes());
    out.extend_from_slice(&clock.to_le_bytes());
}

pub fn push(out: &mut Vec<u8>, id: u32, hour: Hour, body: &[u8]) {
    push_held(out, id, hour, body, 0);
}

pub fn push_held(out: &mut Vec<u8>, id: u32, hour: Hour, body: &[u8], held: u32) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&held.to_le_bytes());
    out.extend_from_slice(body);
    let count = u32::from_le_bytes(out[12..16].try_into().expect("a message head")) + 1;
    out[12..16].copy_from_slice(&count.to_le_bytes());
}

#[derive(Debug)]
pub struct Decoded<'a> {
    pub stamp: (u32, i32),
    pub route: u32,
    pub hour: Hour,
    pub reset: bool,
    pub limited: Vec<(u32, Hour)>,
    pub answers: Vec<(u32, Hour, &'a [u8])>,
    pub held: Vec<u32>,
}

pub fn charge(bytes: &[u8]) -> usize {
    if crate::panel::dated::stamp(bytes).is_some() { return 2 * bytes.len() - 24; }
    if !bytes.starts_with(&MAGIC.to_le_bytes()) { return bytes.len(); }
    let word = |at: usize| bytes.get(at..at + 4).map_or(0, |word| u32::from_le_bytes(word.try_into().expect("four bytes")) as usize);
    let (mut at, mut total) = (24, 24);
    while at + 16 <= bytes.len() {
        let (length, held) = (word(at + 8), word(at + 12));
        total += 16 + 2 * length.max(held);
        at += 16 + length;
    }
    total
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(length).filter(|&end| end <= self.bytes.len()).ok_or_else(|| invalid("A followed message ends early"))?;
        let held = &self.bytes[self.at..end];
        self.at = end;
        Ok(held)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn align(&mut self) -> io::Result<()> {
        let to = self.at.next_multiple_of(8);
        self.take(to - self.at).map(drop)
    }
}

pub fn decode(bytes: &[u8]) -> io::Result<Decoded<'_>> {
    let mut reader = Reader { bytes, at: 0 };
    let magic = reader.u32()?;
    if magic != MAGIC {
        return Err(invalid("Not a followed message"));
    }
    let flags = reader.u32()?;
    let hour = reader.u32()? as Hour;
    let count = reader.u32()?;
    let stamp = (reader.u32()?, reader.u32()? as i32);
    let mut answers = Vec::new();
    let mut limited = Vec::new();
    let mut held = Vec::new();
    for _ in 0..count {
        let id = reader.u32()?;
        let at = reader.u32()? as Hour;
        let length = reader.u32()? as usize;
        let whole = reader.u32()?;
        if length == 0 { limited.push((id, at)); } else { answers.push((id, at, reader.take(length)?)); held.push(whole); }
    }
    if reader.at != bytes.len() {
        return Err(invalid("A followed message holds bytes past its answers"));
    }
    Ok(Decoded { stamp, route: flags >> 1, hour, reset: flags & RESET != 0, limited, answers, held })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub columns: usize,
    pub keys: Vec<u128>,
    pub values: Vec<i64>,
    pub states: Vec<State>,
    pub reasons: Vec<u32>,
    pub names: Vec<u32>,
    pub hours: Option<Vec<Hour>>,
    pub changed: bool,
    pub canonical: bool,
    pub texts: Vec<String>,
    pub not_recorded: Option<String>,
}

impl Answer {
    pub fn parse(body: &[u8]) -> io::Result<Self> {
        let mut reader = Reader { bytes: body, at: 0 };
        let head = reader.take(4)?;
        let (kind, flags) = (head[0], head[1]);
        let rows = reader.u32()? as usize;
        let columns = reader.u32()? as usize;
        let count = reader.u32()? as usize;
        let numbers = |reader: &mut Reader<'_>, count: usize| -> io::Result<Vec<u64>> {
            Ok(reader.take(count * 8)?.chunks_exact(8).map(|bytes| u64::from_le_bytes(bytes.try_into().expect("eight bytes"))).collect())
        };
        let mut answer = Self { columns, keys: Vec::new(), values: Vec::new(), states: Vec::new(), reasons: Vec::new(), names: Vec::new(), hours: None, changed: kind == CHANGED, canonical: flags & CANONICAL != 0, texts: Vec::new(), not_recorded: None };
        if kind > CHANGED {
            return Err(invalid(format!("No followed answer kind {kind}")));
        }
        if kind != NOT_RECORDED {
            let low = numbers(&mut reader, rows)?;
            let high = numbers(&mut reader, rows)?;
            answer.keys = low.iter().zip(&high).map(|(&low, &high)| u128::from(high) << 64 | u128::from(low)).collect();
            answer.values = numbers(&mut reader, rows * columns)?.into_iter().map(|bits| bits as i64).collect();
            if kind == LOG {
                answer.hours = Some(reader.take(rows * 4)?.chunks_exact(4).map(|bytes| i32::from_le_bytes(bytes.try_into().expect("four bytes"))).collect());
                reader.align()?;
            }
            answer.states = reader.take(rows * columns)?.iter().map(|&byte| State::from(byte)).collect::<io::Result<_>>()?;
            reader.align()?;
            let indexes = |reader: &mut Reader<'_>| -> io::Result<Vec<u32>> { Ok(reader.take(rows * 4)?.chunks_exact(4).map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes"))).collect()) };
            answer.reasons = indexes(&mut reader)?;
            reader.align()?;
            answer.names = indexes(&mut reader)?;
            reader.align()?;
        }
        let ends = (0..=count).map(|_| reader.u32().map(|end| end as usize)).collect::<io::Result<Vec<usize>>>()?;
        reader.align()?;
        if ends[0] != 0 || ends.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(invalid("A followed answer's text offsets do not ascend from zero"));
        }
        let bytes = reader.take(ends[count])?;
        for pair in ends.windows(2) {
            answer.texts.push(String::from_utf8(bytes[pair[0]..pair[1]].to_vec()).map_err(|_| invalid("A followed text is not UTF-8"))?);
        }
        reader.align()?;
        if reader.at != body.len() {
            return Err(invalid("A followed answer holds bytes past its texts"));
        }
        if kind == NOT_RECORDED {
            answer.not_recorded = Some(answer.texts.first().cloned().ok_or_else(|| invalid("A not recorded answer gives no reason"))?);
        }
        Ok(answer)
    }

    pub fn rows(&self) -> usize {
        self.keys.len()
    }

    pub fn reason(&self, row: usize) -> Option<&str> {
        self.reasons.get(row).and_then(|&at| self.texts.get(at as usize)).map(String::as_str)
    }

    pub fn name(&self, row: usize) -> Option<&str> {
        self.names.get(row).and_then(|&at| self.texts.get(at as usize)).map(String::as_str)
    }
}

pub fn merge(older: &[u8], newer: &[u8]) -> io::Result<(Vec<u8>, u32)> {
    let (old, new) = (decode(older)?, decode(newer)?);
    if old.route != new.route || old.stamp != new.stamp {
        return Err(invalid("Messages of two follows do not merge"));
    }
    let mut out = Vec::with_capacity(older.len() + newer.len());
    begin(&mut out, new.route, new.stamp.1, new.hour, new.stamp.0, old.reset || new.reset);
    for &(id, mut through) in &new.limited {
        if !new.reset && !new.answers.iter().any(|&(answered, ..)| answered == id) {
            let earlier = old.limited.iter().rev().find(|&&(limited, _)| limited == id).map_or(old.hour, |&(_, hour)| hour);
            through = through.max(earlier);
        }
        push(&mut out, id, through, &[]);
    }
    let mut superseded = 0;
    let is_log = |body: &[u8]| body.first() == Some(&LOG);
    let laid = |earlier: &[u8], later: &[u8]| later.first() == Some(&CHANGED) && matches!(earlier.first(), Some(&ROWS | &CHANGED));
    for (at, &(id, hour, body)) in old.answers.iter().enumerate() {
        if new.reset {
            superseded += 1;
        } else if let Some(&(.., later)) = new.answers.iter().find(|&&(held, ..)| held == id) {
            if !is_log(body) && !laid(body, later) {
                superseded += 1;
            }
        } else {
            push_held(&mut out, id, hour, body, old.held[at]);
        }
    }
    for (at, &(id, hour, body)) in new.answers.iter().enumerate() {
        match old.answers.iter().find(|&&(held, ..)| held == id).map(|&(.., earlier)| earlier) {
            Some(earlier) if is_log(earlier) && is_log(body) && !new.reset => push(&mut out, id, hour, &logs(earlier, body)?),
            Some(earlier) if !new.reset && laid(earlier, body) => {
                let merged = changed(earlier, body)?;
                let whole = if merged.first() == Some(&CHANGED) { new.held[at] } else { 0 };
                push_held(&mut out, id, hour, &merged, whole);
            }
            _ => push_held(&mut out, id, hour, body, new.held[at]),
        }
    }
    Ok((out, superseded))
}

fn copy_row(table: &mut Table, answer: &Answer, row: usize) {
    table.row(answer.keys[row]);
    for column in 0..answer.columns {
        table.cell(answer.values[row * answer.columns + column], answer.states[row * answer.columns + column]);
    }
    if let Some(reason) = answer.reason(row) {
        table.explain(reason);
    }
    if let Some(name) = answer.name(row) {
        table.name(name);
    }
}

fn changed(earlier: &[u8], later: &[u8]) -> io::Result<Vec<u8>> {
    let (a, b) = (Answer::parse(earlier)?, Answer::parse(later)?);
    if a.columns != b.columns {
        return Err(invalid("Rows changed are of another shape than the rows they change"));
    }
    let by_key: std::collections::HashMap<u128, usize> = b.keys.iter().enumerate().map(|(row, &key)| (key, row)).collect();
    let held: std::collections::HashSet<u128> = a.keys.iter().copied().collect();
    let new: Vec<usize> = (0..b.rows()).filter(|&row| !held.contains(&b.keys[row])).collect();
    if !a.changed && !new.is_empty() {
        return Err(invalid("Rows changed name a key the rows they change do not hold"));
    }
    let mut rows: Vec<(u128, &Answer, usize)> = (0..a.rows()).map(|row| match by_key.get(&a.keys[row]) {
        Some(&later) => (a.keys[row], &b, later),
        None => (a.keys[row], &a, row),
    }).collect();
    rows.extend(new.into_iter().map(|row| (b.keys[row], &b, row)));
    if a.canonical && b.canonical {
        rows.sort_unstable_by_key(|&(key, ..)| key);
    }
    let mut table = Table::default();
    table.clear(a.columns, false);
    for (_, answer, row) in rows {
        copy_row(&mut table, answer, row);
    }
    let mut out = Vec::new();
    if a.changed { table.encode_changed(&mut out) } else { table.encode(&mut out) }
    Ok(out)
}

fn logs(earlier: &[u8], later: &[u8]) -> io::Result<Vec<u8>> {
    let mut table = Table::default();
    let (a, b) = (Answer::parse(earlier)?, Answer::parse(later)?);
    table.clear(a.columns, true);
    for answer in [&a, &b] {
        for row in 0..answer.rows() {
            table.hours.push(answer.hours.as_ref().map_or(0, |hours| hours[row]));
            copy_row(&mut table, answer, row);
        }
    }
    let mut out = Vec::new();
    table.encode(&mut out);
    Ok(out)
}
