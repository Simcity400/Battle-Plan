
use crate::{
    block::{put_varint, unzig, zig},
    invalid,
    leaf::Reader,
    manifest::Status,
    state::{Cell, Table},
    store::{TableHour, TableView},
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, binary_heap::PeekMut},
    io,
};

#[cfg(test)]
mod tests;

const VERSION: u8 = 4;
pub const BASE_HOURS: i64 = 6;
pub const FANOUT: i64 = 16;

pub const MAX_LEVEL: u8 = 8;

pub const fn bucket_hours(level: u8) -> i64 {
    BASE_HOURS * FANOUT.pow(level as u32)
}

pub const fn bucket_of(hour: i64, level: u8) -> i64 {
    hour.div_euclid(bucket_hours(level))
}

pub mod aggregate {
    pub const SUM: u8 = 1;
    pub const ABS: u8 = 2;
    pub const FALLS: u8 = 4;
    pub const CHANGES: u8 = 8;
    pub const ALL: u8 = SUM | ABS | FALLS | CHANGES;
}

pub trait Cells {
    fn number(&self, column: usize, slot: u32) -> Option<i64>;
    fn key(&self, slot: u32) -> Option<u128>;
    fn slot(&self, key: u128) -> Option<u32>;
    fn each_row(&self, each: &mut dyn FnMut(u32));
}

impl Cells for Table {
    fn number(&self, column: usize, slot: u32) -> Option<i64> {
        numeric(self, column, slot)
    }

    fn key(&self, slot: u32) -> Option<u128> {
        Table::key(self, slot)
    }

    fn slot(&self, key: u128) -> Option<u32> {
        Table::slot(self, key)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        self.live().ones().for_each(each);
    }
}

impl Cells for TableView<'_> {
    fn number(&self, column: usize, slot: u32) -> Option<i64> {
        TableView::number(self, column, slot)
    }

    fn key(&self, slot: u32) -> Option<u128> {
        TableView::key(self, slot)
    }

    fn slot(&self, key: u128) -> Option<u32> {
        TableView::slot(self, key)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        self.rows().for_each(each);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Computed {
    Value(i64),
    NotApplicable(&'static str),
    Unrecorded,
}

impl Computed {
    pub fn of(value: Option<i64>) -> Self {
        value.map_or(Self::Unrecorded, Self::Value)
    }

    pub fn value(self) -> Option<i64> {
        match self {
            Self::Value(value) => Some(value),
            Self::NotApplicable(_) | Self::Unrecorded => None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Source {
    Cell(usize),
    Computed {
        id: u32,
        inputs: u64,
        value: fn(&dyn Cells, u32) -> Computed,
    },
}

impl Source {
    pub fn id(&self) -> u32 {
        match self {
            Self::Cell(column) => *column as u32,
            Self::Computed { id, .. } => COMPUTED + id,
        }
    }

    pub fn read(&self, table: &dyn Cells, slot: u32) -> Computed {
        match self {
            Self::Cell(column) => Computed::of(table.number(*column, slot)),
            Self::Computed { value, .. } => value(table, slot),
        }
    }

    pub fn value(&self, table: &dyn Cells, slot: u32) -> Option<i64> {
        self.read(table, slot).value()
    }

    fn reads(&self, mask: u64) -> bool {
        match self {
            Self::Cell(column) => *column >= 64 || mask >> column & 1 != 0,
            Self::Computed { inputs, .. } => mask & inputs != 0 || mask == u64::MAX,
        }
    }
}

pub const COMPUTED: u32 = 1 << 16;

#[derive(Clone, Copy)]
pub struct Column {
    pub source: Source,
    pub aggregates: u8,
    pub rank: u8,
}

#[derive(Clone)]
pub enum Shape {
    Values(Vec<Column>),
    Lifetimes,
    Log(Vec<Facet>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    Column(usize),
    Key(u32),
}

impl Shape {
    fn code(&self) -> u8 {
        match self {
            Self::Values(_) => 0,
            Self::Lifetimes => 1,
            Self::Log(_) => 2,
        }
    }
}

pub fn numeric(table: &Table, column: usize, slot: u32) -> Option<i64> {
    if !table.recorded(column, slot) {
        return None;
    }
    Some(match table.column(column)?.cell(slot)? {
        Cell::U8(value) => i64::from(value),
        Cell::I8(value) => i64::from(value),
        Cell::U16(value) => i64::from(value),
        Cell::I16(value) => i64::from(value),
        Cell::U32(value) => i64::from(value),
        Cell::I32(value) => i64::from(value),
        Cell::U64(value) => value as i64,
        Cell::I64(value) => value,
        Cell::Bool(value) => i64::from(value),
        Cell::OptionalI32(value) => i64::from(value?),
        Cell::OptionalU32(value) => i64::from(value?),
        Cell::OptionalI64(value) => value?,
        Cell::OptionalU64(value) => value? as i64,
        Cell::Bytes(_) | Cell::U32List(_) | Cell::U64List(_) | Cell::Opaque(_) => return None,
    })
}

fn available(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Truncated { .. })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fold {
    pub hours: u32,
    pub first: i64,
    pub last: i64,
    pub min: i64,
    pub max: i64,
    pub first_bucket: i64,
    pub last_bucket: i64,
    pub min_bucket: i64,
    pub max_bucket: i64,
    pub sum: i128,
    pub abs: u128,
    pub falls: u64,
    pub changes: u64,
}

impl Fold {
    pub fn constant(value: i64, hours: u32, bucket: i64) -> Self {
        Self {
            hours,
            first: value,
            last: value,
            min: value,
            max: value,
            first_bucket: bucket,
            last_bucket: bucket,
            min_bucket: bucket,
            max_bucket: bucket,
            sum: i128::from(value) * i128::from(hours),
            abs: u128::from(value.unsigned_abs()) * u128::from(hours),
            falls: 0,
            changes: 0,
        }
    }

    pub fn merge(&mut self, later: &Self) {
        if later.min < self.min {
            (self.min, self.min_bucket) = (later.min, later.min_bucket);
        }
        if later.max > self.max {
            (self.max, self.max_bucket) = (later.max, later.max_bucket);
        }
        self.last = later.last;
        self.last_bucket = later.last_bucket;
        self.hours += later.hours;
        self.sum += later.sum;
        self.abs += later.abs;
        self.falls += later.falls;
        self.changes += later.changes;
    }

    fn constant_value(&self) -> Option<i64> {
        (self.first == self.last && self.min == self.max && self.first == self.min).then_some(self.first)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Life {
    pub hours: u32,
    pub first: u32,
    pub end: u32,
}

impl Life {
    pub fn contiguous(&self) -> bool {
        self.end - self.first == self.hours
    }

    fn merge(&mut self, later: &Self, offset: u32) {
        if later.hours == 0 {
            return;
        }
        if self.hours == 0 {
            *self = Life {
                hours: later.hours,
                first: later.first + offset,
                end: later.end + offset,
            };
            return;
        }
        self.hours += later.hours;
        self.end = later.end + offset;
    }
}


const NONE: u8 = 0;
const SAME: u8 = 1;
const SET: u8 = 2;
const VARY: u8 = 3;

const EMPTY: u8 = 0;
const FULL: u8 = 1;
const PART: u8 = 2;

fn put_i128(out: &mut Vec<u8>, value: i128) {
    let mut value = ((value << 1) ^ (value >> 127)) as u128;
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_u128(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn get_u128(reader: &mut Reader<'_>) -> io::Result<u128> {
    let mut value = 0u128;
    for shift in (0..128).step_by(7) {
        let byte = reader.u8()?;
        if u32::from(byte & 0x7f) >> (128 - shift).min(7) != 0 {
            break;
        }
        value |= u128::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(invalid("A summary varint overflows 128 bits"))
}

fn get_i128(reader: &mut Reader<'_>) -> io::Result<i128> {
    let value = get_u128(reader)?;
    Ok(((value >> 1) as i128) ^ -((value & 1) as i128))
}

struct TokenWriter {
    aggregates: u8,
    out: Vec<u8>,
    run: Option<(u8, u64)>,
    last: Option<i64>,
}

impl TokenWriter {
    fn new(aggregates: u8) -> Self {
        Self {
            aggregates,
            out: Vec::new(),
            run: None,
            last: None,
        }
    }

    fn flush(&mut self) {
        if let Some((tag, count)) = self.run.take() {
            self.out.push(tag);
            put_varint(&mut self.out, count);
        }
    }

    fn running(&mut self, tag: u8) {
        self.repeat(tag, 1);
    }

    fn repeat(&mut self, tag: u8, count: u64) {
        if count == 0 {
            return;
        }
        match &mut self.run {
            Some((held, run)) if *held == tag => *run += count,
            _ => {
                self.flush();
                self.run = Some((tag, count));
            }
        }
    }

    fn constant(&mut self, value: i64, count: u64, full: u32) {
        if count == 0 {
            return;
        }
        self.bucket(Some(&Fold::constant(value, full, 0)), full);
        self.repeat(SAME, count - 1);
    }

    fn bucket(&mut self, fold: Option<&Fold>, full: u32) {
        let Some(fold) = fold.filter(|fold| fold.hours > 0) else {
            self.running(NONE);
            return;
        };
        let constant = fold.constant_value().filter(|_| fold.hours == full);
        match (constant, self.last) {
            (Some(value), Some(last)) if value == last && fold.changes == 0 && fold.falls == 0 => self.running(SAME),
            (Some(value), last) => {
                self.flush();
                self.out.push(SET);
                put_varint(&mut self.out, zig(value.wrapping_sub(last.unwrap_or(0))));
                self.counts(fold);
                self.last = Some(value);
            }
            (None, last) => {
                self.flush();
                self.out.push(VARY);
                put_varint(&mut self.out, zig(fold.first.wrapping_sub(last.unwrap_or(0))));
                put_varint(&mut self.out, zig(fold.last.wrapping_sub(fold.first)));
                put_u128(&mut self.out, (i128::from(fold.first.min(fold.last)) - i128::from(fold.min)) as u128);
                put_u128(&mut self.out, (i128::from(fold.max) - i128::from(fold.first.max(fold.last))) as u128);
                put_varint(&mut self.out, u64::from(fold.hours));
                if self.aggregates & aggregate::SUM != 0 {
                    put_i128(&mut self.out, fold.sum);
                }
                if self.aggregates & aggregate::ABS != 0 {
                    put_u128(&mut self.out, fold.abs);
                }
                self.counts(fold);
                self.last = Some(fold.last);
            }
        }
    }

    fn counts(&mut self, fold: &Fold) {
        if self.aggregates & aggregate::FALLS != 0 {
            put_varint(&mut self.out, fold.falls);
        }
        if self.aggregates & aggregate::CHANGES != 0 {
            put_varint(&mut self.out, fold.changes);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.flush();
        self.out
    }
}

pub struct TokenReader<'a> {
    reader: Reader<'a>,
    aggregates: u8,
    run: Option<(u8, u64)>,
    last: Option<i64>,
}

impl<'a> TokenReader<'a> {
    fn new(bytes: &'a [u8], aggregates: u8) -> Self {
        Self {
            reader: Reader::new(bytes),
            aggregates,
            run: None,
            last: None,
        }
    }

    pub fn next(&mut self, bucket: i64, full: u32) -> io::Result<Option<Fold>> {
        if self.run.is_none_or(|(_, left)| left == 0) {
            let tag = self.reader.u8()?;
            match tag {
                NONE | SAME => self.run = Some((tag, self.reader.varint()?)),
                SET | VARY => return self.one(tag, bucket, full).map(Some),
                _ => return Err(invalid("A summary token is malformed")),
            }
        }
        let (tag, left) = self.run.as_mut().expect("a run");
        if *left == 0 {
            return Err(invalid("A summary run is empty"));
        }
        *left -= 1;
        match *tag {
            NONE => Ok(None),
            _ => {
                let value = self.last.ok_or_else(|| invalid("A summary repeats no value"))?;
                Ok(Some(Fold::constant(value, full, bucket)))
            }
        }
    }

    pub fn span(&mut self, bucket: i64, full: u32) -> io::Result<Span> {
        let (tag, count) = match self.run.take_if(|(_, left)| *left > 0) {
            Some(run) => run,
            None => match self.reader.u8()? {
                tag @ (NONE | SAME) => (tag, self.reader.varint()?),
                tag @ (SET | VARY) => return Ok(Span::One(self.one(tag, bucket, full)?)),
                _ => return Err(invalid("A summary token is malformed")),
            },
        };
        if count == 0 {
            return Err(invalid("A summary run is empty"));
        }
        Ok(match tag {
            NONE => Span::None(count),
            _ => Span::Same(self.last.ok_or_else(|| invalid("A summary repeats no value"))?, count),
        })
    }

    fn one(&mut self, tag: u8, bucket: i64, full: u32) -> io::Result<Fold> {
        if tag == SET {
            let value = unzig(self.reader.varint()?).wrapping_add(self.last.unwrap_or(0));
            let mut fold = Fold::constant(value, full, bucket);
            self.counts(&mut fold)?;
            self.last = Some(value);
            return Ok(fold);
        }
        let first = unzig(self.reader.varint()?).wrapping_add(self.last.unwrap_or(0));
        let last = unzig(self.reader.varint()?).wrapping_add(first);
        let below = get_u128(&mut self.reader)?;
        let above = get_u128(&mut self.reader)?;
        let min = i64::try_from(i128::from(first.min(last)) - below as i128)
            .map_err(|_| invalid("A bucket's least value passes 64 bits"))?;
        let max = i64::try_from(i128::from(first.max(last)) + above as i128)
            .map_err(|_| invalid("A bucket's greatest value passes 64 bits"))?;
        let hours = self.reader.varint_u32()?;
        let mut fold = Fold {
            hours,
            first,
            last,
            min,
            max,
            first_bucket: bucket,
            last_bucket: bucket,
            min_bucket: bucket,
            max_bucket: bucket,
            ..Fold::default()
        };
        if self.aggregates & aggregate::SUM != 0 {
            fold.sum = get_i128(&mut self.reader)?;
        }
        if self.aggregates & aggregate::ABS != 0 {
            fold.abs = get_u128(&mut self.reader)?;
        }
        self.counts(&mut fold)?;
        self.last = Some(last);
        Ok(fold)
    }

    fn counts(&mut self, fold: &mut Fold) -> io::Result<()> {
        if self.aggregates & aggregate::FALLS != 0 {
            fold.falls = self.reader.varint()?;
        }
        if self.aggregates & aggregate::CHANGES != 0 {
            fold.changes = self.reader.varint()?;
        }
        Ok(())
    }

    pub fn exhausted(&self) -> bool {
        self.reader.left() == 0 && self.run.is_none_or(|(_, left)| left == 0)
    }
}

pub enum Span {
    None(u64),
    Same(i64, u64),
    One(Fold),
}

pub enum LifeSpan {
    Empty(u64),
    Full(u64),
    Part(Life),
}

fn repeat_life(out: &mut Vec<u8>, run: &mut Option<(u8, u64)>, tag: u8, count: u64) {
    if count == 0 {
        return;
    }
    match run {
        Some((held, held_count)) if *held == tag => *held_count += count,
        _ => {
            if let Some((held, held_count)) = run.take() {
                out.push(held);
                put_varint(out, held_count);
            }
            *run = Some((tag, count));
        }
    }
}

fn put_life(out: &mut Vec<u8>, run: &mut Option<(u8, u64)>, life: &Life, full: u32) {
    let tag = match life.hours {
        0 => EMPTY,
        hours if hours == full => FULL,
        _ => PART,
    };
    if tag != PART {
        repeat_life(out, run, tag, 1);
        return;
    }
    if let Some((held, count)) = run.take() {
        out.push(held);
        put_varint(out, count);
    }
    out.push(PART);
    put_varint(out, u64::from(life.hours));
    put_varint(out, u64::from(life.first));
    put_varint(out, u64::from(life.end));
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    pub level: u8,
    pub compressed: bool,
    pub first_hour: i64,
    pub hours: u32,
    pub first_bucket: i64,
    pub buckets: u32,
    pub keys: u32,
    pub shape: u8,
    pub columns: Vec<(u32, u8, u8)>,
    pub frames: Vec<(u64, u64, u64)>,
}

impl Directory {
    fn write(&self) -> Vec<u8> {
        let mut out = vec![VERSION, self.level, u8::from(self.compressed)];
        put_varint(&mut out, zig(self.first_hour));
        put_varint(&mut out, u64::from(self.hours));
        put_varint(&mut out, zig(self.first_bucket));
        put_varint(&mut out, u64::from(self.buckets));
        put_varint(&mut out, u64::from(self.keys));
        out.push(self.shape);
        put_varint(&mut out, self.columns.len() as u64);
        for &(id, aggregates, rank) in &self.columns {
            put_varint(&mut out, u64::from(id));
            out.push(aggregates);
            out.push(rank);
        }
        for &(offset, stored, raw) in &self.frames {
            put_varint(&mut out, offset);
            put_varint(&mut out, stored);
            put_varint(&mut out, raw);
        }
        out
    }

    pub fn read(bytes: &[u8]) -> io::Result<(Self, usize)> {
        let length = u32::from_le_bytes(
            bytes
                .get(..4)
                .ok_or_else(|| invalid("A summary piece ends before its directory"))?
                .try_into()
                .expect("four bytes"),
        ) as usize;
        let body = bytes
            .get(4..4 + length)
            .ok_or_else(|| invalid("A summary piece ends inside its directory"))?;
        let mut reader = Reader::new(body);
        if reader.u8()? != VERSION {
            return Err(invalid("A summary piece of another version"));
        }
        let level = reader.u8()?;
        let compressed = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("A summary piece's directory is malformed")),
        };
        let first_hour = unzig(reader.varint()?);
        let hours = reader.varint_u32()?;
        let first_bucket = unzig(reader.varint()?);
        let buckets = reader.varint_u32()?;
        let keys = reader.varint_u32()?;
        let shape = reader.u8()?;
        let count = reader.count()?;
        if count > body.len() || level > MAX_LEVEL || shape > 2 {
            return Err(invalid("A summary piece's directory is malformed"));
        }
        let mut columns = Vec::with_capacity(count);
        for _ in 0..count {
            columns.push((reader.varint_u32()?, reader.u8()?, reader.u8()?));
        }
        let ranked = columns.iter().filter(|column| column.2 > 0).count();
        let mut frames = Vec::with_capacity(1 + count + ranked);
        for _ in 0..1 + count + ranked {
            frames.push((reader.varint()?, reader.varint()?, reader.varint()?));
        }
        reader.finish()?;
        if bucket_of(first_hour, level) != first_bucket
            || hours == 0
            || bucket_of(first_hour + i64::from(hours) - 1, level) - first_bucket + 1 != i64::from(buckets)
        {
            return Err(invalid("A summary piece's buckets disagree with its hours"));
        }
        Ok((
            Self {
                level,
                compressed,
                first_hour,
                hours,
                first_bucket,
                buckets,
                keys,
                shape,
                columns,
                frames,
            },
            4 + length,
        ))
    }

    pub fn covered(&self, at: u32) -> u32 {
        let size = bucket_hours(self.level);
        let start = (self.first_bucket + i64::from(at)) * size;
        let from = start.max(self.first_hour);
        let to = (start + size).min(self.first_hour + i64::from(self.hours));
        (to - from).max(0) as u32
    }

    fn offset(&self, at: u32) -> u32 {
        let size = bucket_hours(self.level);
        let start = (self.first_bucket + i64::from(at)) * size;
        (self.first_hour.max(start) - start) as u32
    }
}

pub struct Raw {
    directory: Directory,
    frames: Vec<Vec<u8>>,
}

impl Raw {
    pub fn raw_bytes(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }

    pub fn assemble(self, level: i32) -> io::Result<Vec<u8>> {
        let mut directory = self.directory;
        directory.compressed = true;
        directory.frames.clear();
        let mut stored = Vec::with_capacity(self.frames.len());
        let mut offset = 0u64;
        for raw in self.frames {
            let held = zstd::bulk::compress(&raw, level).map_err(|_| invalid("A summary frame did not compress"))?;
            directory.frames.push((offset, held.len() as u64, raw.len() as u64));
            offset += held.len() as u64;
            stored.push(held);
        }
        let head = directory.write();
        let mut out = Vec::with_capacity(4 + head.len() + offset as usize);
        out.extend_from_slice(&(head.len() as u32).to_le_bytes());
        out.extend_from_slice(&head);
        for held in stored {
            out.extend_from_slice(&held);
        }
        Ok(out)
    }
}

pub fn recompress(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
    let (directory, start) = Directory::read(bytes)?;
    let piece = Piece {
        directory,
        keys: Vec::new(),
        head: Vec::new(),
        lives: Vec::new(),
        frames: &bytes[start..],
    };
    let end = piece.directory.frames.iter().map(|&(offset, stored, _)| offset.saturating_add(stored)).max().unwrap_or(0);
    if end != piece.frames.len() as u64 {
        return Err(invalid("A summary piece holds bytes past its frames"));
    }
    let frames = (0..piece.directory.frames.len()).map(|at| piece.frame(at)).collect::<io::Result<Vec<_>>>()?;
    Raw { directory: piece.directory, frames }.assemble(level)
}

fn place(reader: &mut Reader<'_>) -> io::Result<u32> {
    let at = u32::try_from(reader.at()).map_err(|_| invalid("A summary frame passes 2^32 bytes"))?;
    let length = reader.count()?;
    reader.raw(length)?;
    Ok(at)
}

fn prefixed_at(bytes: &[u8], place: u32) -> &[u8] {
    let mut reader = Reader::new(&bytes[place as usize..]);
    let length = reader.count().expect("a length read whole before");
    reader.raw(length).expect("bytes read whole before")
}

pub struct Piece<'a> {
    pub directory: Directory,
    pub keys: Vec<u128>,
    head: Vec<u8>,
    lives: Vec<u32>,
    frames: &'a [u8],
}

impl<'a> Piece<'a> {
    pub fn read(bytes: &'a [u8]) -> io::Result<Self> {
        let (directory, start) = Directory::read(bytes)?;
        let frames = &bytes[start..];
        let mut piece = Self {
            directory,
            keys: Vec::new(),
            head: Vec::new(),
            lives: Vec::new(),
            frames,
        };
        let head = piece.frame(0)?;
        let mut reader = Reader::new(&head);
        let count = piece.directory.keys as usize;
        if count > head.len() {
            return Err(invalid("A summary piece counts more keys than it holds"));
        }
        piece.keys.reserve_exact(count);
        piece.lives.reserve_exact(count);
        let mut key = 0u128;
        for at in 0..count {
            let gap = get_u128(&mut reader)?;
            key = if at == 0 { gap } else { key.wrapping_add(gap).wrapping_add(1) };
            piece.keys.push(key);
        }
        for _ in 0..count {
            piece.lives.push(place(&mut reader)?);
        }
        reader.finish()?;
        piece.head = head;
        Ok(piece)
    }

    pub fn lives(&self, at: usize) -> LifeReader<'_> {
        LifeReader {
            reader: Reader::new(prefixed_at(&self.head, self.lives[at])),
            directory: &self.directory,
            run: None,
        }
    }

    pub fn life(&self, at: usize) -> io::Result<Vec<Life>> {
        let mut lives = self.lives(at);
        let mut out = Vec::with_capacity(self.directory.buckets as usize);
        for bucket in 0..self.directory.buckets {
            out.push(lives.next(bucket)?);
        }
        if !lives.finished() {
            return Err(invalid("A key's life passes its piece's buckets"));
        }
        Ok(out)
    }

    fn frame(&self, at: usize) -> io::Result<Vec<u8>> {
        let &(offset, stored, raw) = self
            .directory
            .frames
            .get(at)
            .ok_or_else(|| invalid("A summary piece names no such frame"))?;
        let bytes = self
            .frames
            .get(offset as usize..(offset + stored) as usize)
            .ok_or_else(|| invalid("A summary frame lies past its piece"))?;
        let decoded = match self.directory.compressed {
            true => zstd::bulk::decompress(bytes, raw as usize).map_err(|_| invalid("A summary frame does not decompress"))?,
            false => bytes.to_vec(),
        };
        if decoded.len() as u64 != raw {
            return Err(invalid("A summary frame decompresses to another length"));
        }
        Ok(decoded)
    }

    pub fn column_of(&self, id: u32) -> Option<usize> {
        self.directory.columns.iter().position(|&(held, _, _)| held == id)
    }

    pub fn column(&self, at: usize) -> io::Result<ColumnFrame> {
        let raw = self.frame(1 + at)?;
        let mut places = Vec::with_capacity(self.keys.len());
        let mut reader = Reader::new(&raw);
        for _ in 0..self.keys.len() {
            places.push(place(&mut reader)?);
        }
        reader.finish()?;
        Ok(ColumnFrame {
            aggregates: self.directory.columns[at].1,
            raw,
            places,
        })
    }

    pub fn key_at(&self, key: u128) -> Option<usize> {
        self.keys.binary_search(&key).ok()
    }

    pub fn rank(&self, at: usize) -> u8 {
        self.directory.columns.get(at).map_or(0, |column| column.2)
    }

    pub fn ranks(&self, at: usize) -> io::Result<Option<Ranks>> {
        let rank = self.rank(at);
        if rank == 0 {
            return Ok(None);
        }
        let before = self.directory.columns[..at].iter().filter(|column| column.2 > 0).count();
        let raw = self.frame(1 + self.directory.columns.len() + before)?;
        Ranks::read(&raw, self.directory.buckets, self.keys.len(), rank).map(Some)
    }
}

pub struct LifeReader<'a> {
    reader: Reader<'a>,
    directory: &'a Directory,
    run: Option<(u8, u64)>,
}

impl LifeReader<'_> {
    pub fn next(&mut self, at: u32) -> io::Result<Life> {
        if self.run.is_none_or(|(_, left)| left == 0) {
            match self.reader.u8()? {
                tag @ (EMPTY | FULL) => self.run = Some((tag, self.reader.varint()?)),
                PART => {
                    return Ok(Life {
                        hours: self.reader.varint_u32()?,
                        first: self.reader.varint_u32()?,
                        end: self.reader.varint_u32()?,
                    });
                }
                _ => return Err(invalid("A summary life token is malformed")),
            }
        }
        let (tag, left) = self.run.as_mut().expect("a run");
        if *left == 0 {
            return Err(invalid("A summary life run is empty"));
        }
        *left -= 1;
        Ok(match *tag {
            EMPTY => Life::default(),
            _ => {
                let first = self.directory.offset(at);
                let hours = self.directory.covered(at);
                Life { hours, first, end: first + hours }
            }
        })
    }

    pub fn span(&mut self) -> io::Result<LifeSpan> {
        let (tag, count) = match self.run.take_if(|(_, left)| *left > 0) {
            Some(run) => run,
            None => match self.reader.u8()? {
                tag @ (EMPTY | FULL) => (tag, self.reader.varint()?),
                PART => {
                    return Ok(LifeSpan::Part(Life {
                        hours: self.reader.varint_u32()?,
                        first: self.reader.varint_u32()?,
                        end: self.reader.varint_u32()?,
                    }));
                }
                _ => return Err(invalid("A summary life token is malformed")),
            },
        };
        if count == 0 {
            return Err(invalid("A summary life run is empty"));
        }
        Ok(match tag {
            EMPTY => LifeSpan::Empty(count),
            _ => LifeSpan::Full(count),
        })
    }

    pub fn finished(&self) -> bool {
        self.reader.left() == 0 && self.run.is_none_or(|(_, left)| left == 0)
    }
}

pub struct ColumnFrame {
    pub aggregates: u8,
    raw: Vec<u8>,
    places: Vec<u32>,
}

impl ColumnFrame {
    pub fn tokens(&self, at: usize) -> TokenReader<'_> {
        TokenReader::new(prefixed_at(&self.raw, self.places[at]), self.aggregates)
    }

    pub fn buckets(&self, piece: &Piece<'_>, at: usize) -> io::Result<Vec<Option<Fold>>> {
        let mut tokens = self.tokens(at);
        let mut out = Vec::with_capacity(piece.directory.buckets as usize);
        for bucket in 0..piece.directory.buckets {
            out.push(tokens.next(piece.directory.first_bucket + i64::from(bucket), piece.directory.covered(bucket))?);
        }
        if tokens.reader.left() != 0 || tokens.run.is_some_and(|(_, left)| left != 0) {
            return Err(invalid("A key's tokens pass its piece's buckets"));
        }
        Ok(out)
    }

    pub fn counts(&self, piece: &Piece<'_>, at: usize) -> io::Result<Vec<u64>> {
        let mut reader = Reader::new(prefixed_at(&self.raw, self.places[at]));
        let mut out = Vec::with_capacity(piece.directory.buckets as usize);
        while out.len() < piece.directory.buckets as usize {
            match reader.u8()? {
                0 => out.extend(std::iter::repeat_n(0, reader.count()?)),
                1 => out.push(reader.varint()?),
                _ => return Err(invalid("A log count token is malformed")),
            }
        }
        reader.finish()?;
        if out.len() != piece.directory.buckets as usize {
            return Err(invalid("A key's counts pass its piece's buckets"));
        }
        Ok(out)
    }
}

const RANKS_SAME: u8 = 0;
const RANKS_LIST: u8 = 1;

#[derive(Debug, PartialEq, Eq)]
pub struct Ranks {
    positions: Vec<u32>,
    lists: Vec<std::ops::Range<usize>>,
}

impl Ranks {
    pub fn bucket(&self, at: u32) -> &[u32] {
        self.lists.get(at as usize).map_or(&[], |range| &self.positions[range.clone()])
    }

    fn build(directory: &Directory, tokens: &[&[u8]], aggregates: u8, rank: u8) -> io::Result<Self> {
        let buckets = directory.buckets;
        let rank = usize::from(rank);
        let mut best: Vec<Vec<(i64, u32)>> = vec![Vec::new(); buckets as usize];
        let mut offer = |bucket: u32, value: i64, position: u32| {
            let list = &mut best[bucket as usize];
            if list.len() == rank && list.last().is_some_and(|worst| worst.0 >= value) {
                return;
            }
            let at = list.partition_point(|held| held.0 >= value);
            list.insert(at, (value, position));
            list.truncate(rank);
        };
        let run = |count: u64, at: u32| u32::try_from(count).ok().filter(|&count| count <= buckets - at).ok_or_else(|| invalid("A key's tokens pass its piece's buckets"));
        for (position, &bytes) in tokens.iter().enumerate() {
            let position = position as u32;
            let mut reader = TokenReader::new(bytes, aggregates);
            let mut at = 0;
            while at < buckets {
                match reader.span(directory.first_bucket + i64::from(at), directory.covered(at))? {
                    Span::None(count) => at += run(count, at)?,
                    Span::One(fold) => {
                        offer(at, fold.last, position);
                        at += 1;
                    }
                    Span::Same(value, count) => {
                        let end = at + run(count, at)?;
                        for bucket in at..end {
                            offer(bucket, value, position);
                        }
                        at = end;
                    }
                }
            }
        }
        let mut ranks = Self { positions: Vec::new(), lists: Vec::with_capacity(buckets as usize) };
        for list in best {
            let start = ranks.positions.len();
            ranks.positions.extend(list.into_iter().map(|(_, position)| position));
            ranks.lists.push(start..ranks.positions.len());
        }
        Ok(ranks)
    }

    fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut previous: &[u32] = &[];
        let mut same = 0u64;
        for at in 0..self.lists.len() {
            let list = self.bucket(at as u32);
            if list == previous {
                same += 1;
                continue;
            }
            if same > 0 {
                out.push(RANKS_SAME);
                put_varint(&mut out, same);
                same = 0;
            }
            out.push(RANKS_LIST);
            let prefix = list.iter().zip(previous).take_while(|(a, b)| a == b).count();
            put_varint(&mut out, prefix as u64);
            put_varint(&mut out, list.len() as u64);
            let mut expected = prefix as i64;
            for &position in &list[prefix..] {
                match previous.iter().position(|&held| held == position) {
                    Some(place) => {
                        put_varint(&mut out, zig(place as i64 - expected) + 1);
                        expected = place as i64 + 1;
                    }
                    None => {
                        out.push(0);
                        put_varint(&mut out, u64::from(position));
                    }
                }
            }
            previous = list;
        }
        if same > 0 {
            out.push(RANKS_SAME);
            put_varint(&mut out, same);
        }
        out
    }

    fn read(raw: &[u8], buckets: u32, keys: usize, rank: u8) -> io::Result<Self> {
        let malformed = || invalid("A summary's ranks are malformed");
        let mut reader = Reader::new(raw);
        let mut ranks = Self { positions: Vec::new(), lists: Vec::with_capacity(buckets as usize) };
        let mut previous = 0..0;
        while ranks.lists.len() < buckets as usize {
            match reader.u8()? {
                RANKS_SAME => {
                    let count = reader.count()?;
                    if count == 0 || count > buckets as usize - ranks.lists.len() {
                        return Err(malformed());
                    }
                    ranks.lists.extend(std::iter::repeat_n(previous.clone(), count));
                }
                RANKS_LIST => {
                    let prefix = reader.count()?;
                    let length = reader.count()?;
                    if length > usize::from(rank) || prefix > length || prefix > previous.len() {
                        return Err(malformed());
                    }
                    let start = ranks.positions.len();
                    ranks.positions.extend_from_within(previous.start..previous.start + prefix);
                    let mut expected = prefix as i64;
                    for _ in prefix..length {
                        let position = match reader.varint()? {
                            0 => reader.varint_u32()?,
                            code => {
                                let place = expected + unzig(code - 1);
                                let place = usize::try_from(place).ok().filter(|&place| place < previous.len()).ok_or_else(malformed)?;
                                expected = place as i64 + 1;
                                ranks.positions[previous.start + place]
                            }
                        };
                        if position as usize >= keys {
                            return Err(malformed());
                        }
                        ranks.positions.push(position);
                    }
                    previous = start..ranks.positions.len();
                    ranks.lists.push(previous.clone());
                }
                _ => return Err(malformed()),
            }
        }
        reader.finish()?;
        Ok(ranks)
    }
}

#[derive(Default)]
struct Counts {
    out: Vec<u8>,
    next: u32,
    open: u32,
    count: u64,
}

impl Counts {
    fn add(&mut self, bucket: u32) {
        if self.count > 0 && bucket != self.open {
            self.close();
        }
        self.open = bucket;
        self.count += 1;
    }

    fn close(&mut self) {
        if self.count == 0 {
            return;
        }
        if self.open > self.next {
            self.out.push(0);
            put_varint(&mut self.out, u64::from(self.open - self.next));
        }
        self.out.push(1);
        put_varint(&mut self.out, self.count);
        self.next = self.open + 1;
        self.count = 0;
    }

    fn finish(mut self, buckets: u32) -> Vec<u8> {
        self.close();
        if buckets > self.next {
            self.out.push(0);
            put_varint(&mut self.out, u64::from(buckets - self.next));
        }
        self.out
    }

    fn of(counts: &[u64]) -> Vec<u8> {
        let mut out = Self::default();
        for (bucket, &count) in counts.iter().enumerate() {
            if count > 0 {
                out.open = bucket as u32;
                out.count = count;
                out.close();
            }
        }
        out.finish(counts.len() as u32)
    }
}

fn life_tokens(life: &[Life], directory: &Directory) -> Vec<u8> {
    let mut out = Vec::new();
    let mut run = None;
    for (at, bucket) in life.iter().enumerate() {
        put_life(&mut out, &mut run, bucket, directory.covered(at as u32));
    }
    if let Some((tag, count)) = run {
        out.push(tag);
        put_varint(&mut out, count);
    }
    out
}

fn put_prefixed(frame: &mut Vec<u8>, bytes: &[u8]) {
    put_varint(frame, bytes.len() as u64);
    frame.extend_from_slice(bytes);
}

fn prefixed(frame: &[u8], count: usize) -> io::Result<Vec<&[u8]>> {
    let mut reader = Reader::new(frame);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.count()?;
        out.push(reader.raw(length)?);
    }
    reader.finish()?;
    Ok(out)
}

fn build(directory: Directory, keys: &[u128], lives: &[u8], columns: Vec<Vec<u8>>) -> io::Result<Raw> {
    let mut head = Vec::with_capacity(2 * keys.len() + lives.len());
    let mut previous = 0u128;
    for (at, &key) in keys.iter().enumerate() {
        let gap = if at == 0 { key } else { key - previous - 1 };
        put_u128(&mut head, gap);
        previous = key;
    }
    head.extend_from_slice(lives);
    let mut ranks = Vec::new();
    for (column, &(_, aggregates, rank)) in columns.iter().zip(&directory.columns) {
        if rank > 0 {
            ranks.push(Ranks::build(&directory, &prefixed(column, keys.len())?, aggregates, rank)?.write());
        }
    }
    let mut frames = Vec::with_capacity(1 + columns.len() + ranks.len());
    frames.push(head);
    frames.extend(columns);
    frames.extend(ranks);
    Ok(Raw { directory, frames })
}


fn within(bucket: i64, first_hour: i64, end: i64) -> u32 {
    let start = bucket * BASE_HOURS;
    let from = start.max(first_hour);
    let to = (start + BASE_HOURS).min(end);
    (to - from).max(0) as u32
}

struct Stream {
    open: Option<(i64, Fold)>,
    next: i64,
    writer: TokenWriter,
}

impl Stream {
    fn new(first_bucket: i64, aggregates: u8) -> Self {
        Self {
            open: None,
            next: first_bucket,
            writer: TokenWriter::new(aggregates),
        }
    }

    fn close_before(&mut self, bucket: i64, first_hour: i64, end: i64) {
        if let Some((held, fold)) = self.open.take_if(|(held, _)| *held < bucket) {
            self.writer.repeat(NONE, (held - self.next).max(0) as u64);
            self.writer.bucket(Some(&fold), within(held, first_hour, end));
            self.next = held + 1;
        }
        self.writer.repeat(NONE, (bucket - self.next).max(0) as u64);
        self.next = self.next.max(bucket);
    }

    fn at(&mut self, bucket: i64, first_hour: i64, value: i64) -> &mut Fold {
        if self.open.as_ref().is_some_and(|(held, _)| *held < bucket) {
            self.close_before(bucket, first_hour, i64::MAX);
        }
        &mut self
            .open
            .get_or_insert_with(|| {
                (
                    bucket,
                    Fold {
                        first: value,
                        last: value,
                        min: value,
                        max: value,
                        first_bucket: bucket,
                        last_bucket: bucket,
                        min_bucket: bucket,
                        max_bucket: bucket,
                        ..Fold::default()
                    },
                )
            })
            .1
    }

    fn add(&mut self, first_hour: i64, value: i64, from: i64, to: i64) {
        let mut at = from;
        while at < to {
            let bucket = bucket_of(at, 0);
            let start = bucket * BASE_HOURS;
            let whole = (to - start) / BASE_HOURS;
            if at == start && start >= first_hour && whole > 0 && self.open.as_ref().is_none_or(|(held, _)| *held < bucket) {
                self.close_before(bucket, first_hour, i64::MAX);
                self.writer.constant(value, whole as u64, BASE_HOURS as u32);
                self.next = bucket + whole;
                at = start + whole * BASE_HOURS;
                continue;
            }
            let end = ((bucket + 1) * BASE_HOURS).min(to);
            let part = Fold::constant(value, (end - at) as u32, bucket);
            let fold = self.at(bucket, first_hour, value);
            if fold.hours == 0 {
                let (changes, falls) = (fold.changes, fold.falls);
                *fold = part;
                fold.changes = changes;
                fold.falls = falls;
            } else {
                fold.merge(&part);
            }
            at = end;
        }
    }

    fn finish(mut self, end_bucket: i64, first_hour: i64, end: i64) -> Vec<u8> {
        self.close_before(end_bucket, first_hour, end);
        self.writer.finish()
    }
}

struct LifeStream {
    open: Option<(i64, Life)>,
    next: i64,
    out: Vec<u8>,
    run: Option<(u8, u64)>,
    any: bool,
}

impl LifeStream {
    fn new(first_bucket: i64) -> Self {
        Self {
            open: None,
            next: first_bucket,
            out: Vec::new(),
            run: None,
            any: false,
        }
    }

    fn close_before(&mut self, bucket: i64, first_hour: i64, end: i64) {
        if let Some((held, life)) = self.open.take_if(|(held, _)| *held < bucket) {
            repeat_life(&mut self.out, &mut self.run, EMPTY, (held - self.next).max(0) as u64);
            put_life(&mut self.out, &mut self.run, &life, within(held, first_hour, end));
            self.next = held + 1;
        }
        repeat_life(&mut self.out, &mut self.run, EMPTY, (bucket - self.next).max(0) as u64);
        self.next = self.next.max(bucket);
    }

    fn add(&mut self, first_hour: i64, from: i64, to: i64) {
        let mut at = from;
        while at < to {
            let bucket = bucket_of(at, 0);
            let start = bucket * BASE_HOURS;
            let whole = (to - start) / BASE_HOURS;
            if at == start && start >= first_hour && whole > 0 && self.open.as_ref().is_none_or(|(held, _)| *held < bucket) {
                self.close_before(bucket, first_hour, i64::MAX);
                repeat_life(&mut self.out, &mut self.run, FULL, whole as u64);
                self.next = bucket + whole;
                self.any = true;
                at = start + whole * BASE_HOURS;
                continue;
            }
            let end = ((bucket + 1) * BASE_HOURS).min(to);
            if self.open.as_ref().is_some_and(|(held, _)| *held < bucket) {
                self.close_before(bucket, first_hour, i64::MAX);
            }
            let start = bucket * BASE_HOURS;
            let (first, last) = ((at - start) as u32, (end - start) as u32);
            match &mut self.open {
                Some((_, held)) => {
                    held.hours += last - first;
                    held.end = last;
                }
                None => self.open = Some((bucket, Life { hours: last - first, first, end: last })),
            }
            self.any = true;
            at = end;
        }
    }

    fn finish(mut self, end_bucket: i64, first_hour: i64, end: i64) -> Vec<u8> {
        self.close_before(end_bucket, first_hour, end);
        if let Some((tag, count)) = self.run.take() {
            self.out.push(tag);
            put_varint(&mut self.out, count);
        }
        self.out
    }
}

#[derive(Clone, Copy)]
struct Cell_ {
    value: Option<i64>,
    since: i64,
    before: Option<i64>,
}

struct Track {
    covered: bool,
    since: i64,
    life: LifeStream,
    cells: Vec<Cell_>,
    streams: Vec<Stream>,
}

pub(crate) const CHUNK: usize = 8 << 10;
const CHANGE_ROOM: usize = 64;

const WROTE: u64 = 0;
const CLEARED: u64 = 1;
const BEGAN: u64 = 2;
const BEGAN_NONE: u64 = 3;
const COVERED: u64 = 4;
const UNCOVERED: u64 = 5;
const HOUR_MARK: u64 = 6;
const KIND_BITS: u32 = 3;

#[derive(Default)]
struct Changes {
    chunks: Vec<(Vec<u8>, Option<usize>)>,
    open: Vec<u8>,
    hour: u32,
    track: u32,
}

impl Changes {
    fn head(&mut self, offset: u32, kind: u64, column: usize, track: u32) {
        if self.open.capacity() == 0 {
            self.open.reserve_exact(CHUNK + CHANGE_ROOM);
        }
        if offset != self.hour {
            put_varint(&mut self.open, u64::from(offset - self.hour) << KIND_BITS | HOUR_MARK);
            self.hour = offset;
        }
        put_varint(&mut self.open, (column as u64) << KIND_BITS | kind);
        put_varint(&mut self.open, zig(i64::from(track) - i64::from(self.track)));
        self.track = track;
    }

    fn ended(&mut self) {
        if self.open.len() < CHUNK {
            return;
        }
        let chunk = match zstd::bulk::compress(&self.open, 1) {
            Ok(mut stored) if stored.len() < self.open.len() => {
                stored.shrink_to_fit();
                (stored, Some(self.open.len()))
            }
            _ => (self.open.clone(), None),
        };
        self.chunks.push(chunk);
        self.open.clear();
    }

    fn each(&self, tracks: usize, visit: &mut dyn FnMut(usize, u32, u64, Option<u64>)) -> io::Result<()> {
        let malformed = || invalid("A summary builder's changes are malformed");
        let (mut hour, mut track) = (0u32, 0i64);
        let chunks = self.chunks.iter().map(|(stored, raw)| (stored.as_slice(), *raw));
        for (stored, raw) in chunks.chain(std::iter::once((self.open.as_slice(), None))) {
            let decoded;
            let bytes = match raw {
                Some(raw) => {
                    decoded = zstd::bulk::decompress(stored, raw).map_err(|_| malformed())?;
                    decoded.as_slice()
                }
                None => stored,
            };
            let mut reader = Reader::new(bytes);
            while reader.left() > 0 {
                let head = reader.varint()?;
                let kind = head & ((1 << KIND_BITS) - 1);
                if kind == HOUR_MARK {
                    hour = u32::try_from(head >> KIND_BITS).ok().and_then(|step| hour.checked_add(step)).ok_or_else(malformed)?;
                    continue;
                }
                track += unzig(reader.varint()?);
                let at = usize::try_from(track).ok().filter(|&at| at < tracks).ok_or_else(malformed)?;
                let value = match kind {
                    WROTE | BEGAN => Some(reader.varint()?),
                    _ => None,
                };
                visit(at, hour, head, value);
            }
        }
        Ok(())
    }

    fn split(self, tracks: usize) -> io::Result<Split> {
        let mut ends = vec![0usize; tracks];
        self.each(tracks, &mut |at, hour, head, value| ends[at] += varint_len(u64::from(hour)) + varint_len(head) + value.map_or(0, varint_len))?;
        let mut total = 0;
        for end in &mut ends {
            (*end, total) = (total, total + *end);
        }
        let mut bytes = vec![0; total];
        self.each(tracks, &mut |at, hour, head, value| {
            let at = &mut ends[at];
            for value in [Some(u64::from(hour)), Some(head), value].into_iter().flatten() {
                *at = put_varint_at(&mut bytes, *at, value);
            }
        })?;
        Ok(Split { bytes, ends })
    }

    fn held_bytes(&self) -> usize {
        self.open.capacity() + self.chunks.capacity() * size_of::<(Vec<u8>, Option<usize>)>() + self.chunks.iter().map(|(stored, _)| stored.capacity()).sum::<usize>()
    }
}

fn varint_len(value: u64) -> usize {
    (64 - (value | 1).leading_zeros() as usize).div_ceil(7)
}

fn put_varint_at(out: &mut [u8], mut at: usize, mut value: u64) -> usize {
    while value >= 0x80 {
        out[at] = value as u8 | 0x80;
        value >>= 7;
        at += 1;
    }
    out[at] = value as u8;
    at + 1
}

struct Split {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Split {
    fn track(&self, at: usize) -> &[u8] {
        let start = if at == 0 { 0 } else { self.ends[at - 1] };
        &self.bytes[start..self.ends[at]]
    }
}

pub struct Builder {
    shape: Shape,
    columns: Vec<Column>,
    first_hour: i64,
    first_bucket: i64,
    tracks: Tracks,
    counts: HashMap<u128, Counts, foldhash::fast::RandomState>,
    available: bool,
    truncated: bool,
}

impl Builder {
    pub fn open(shape: &Shape, held: &dyn Cells, status: Status, view: Option<&[u32]>, first_hour: i32) -> Self {
        let first_hour = i64::from(first_hour);
        let columns = match shape {
            Shape::Values(columns) => columns.clone(),
            _ => Vec::new(),
        };
        let mut builder = Self {
            shape: shape.clone(),
            tracks: Tracks::new(first_hour, columns.len()),
            columns,
            first_hour,
            first_bucket: bucket_of(first_hour, 0),
            counts: HashMap::default(),
            available: available(status),
            truncated: matches!(status, Status::Truncated { .. }),
        };
        if !matches!(builder.shape, Shape::Log(_)) && builder.available {
            let rows = view.map_or_else(|| {
                let mut rows = 0;
                held.each_row(&mut |_| rows += 1);
                rows
            }, <[u32]>::len);
            let (tracks, columns) = (&mut builder.tracks, &builder.columns);
            tracks.reserve(rows);
            let mut seed = |slot| {
                let Some(key) = held.key(slot) else { return };
                let at = tracks.at(key);
                tracks.open(at, columns.iter().map(|column| column.source.value(held, slot)));
            };
            match view {
                Some(read) => read.iter().copied().for_each(seed),
                None => held.each_row(&mut seed),
            }
        }
        builder
    }

    pub fn held_bytes(&self) -> usize {
        let counts = self.counts.capacity() * (size_of::<(u128, Counts)>() + 1) + self.counts.values().map(|counts| counts.out.capacity()).sum::<usize>();
        self.tracks.held_bytes() + counts
    }

    fn cover(first_hour: i64, track: &mut Track, covered: bool, hour: i64) {
        if track.covered == covered {
            return;
        }
        if track.covered {
            track.life.add(first_hour, track.since, hour);
        }
        track.covered = covered;
        track.since = hour;
    }

    fn set(first_hour: i64, stream: &mut Stream, cell: &mut Cell_, value: Option<i64>, hour: i64) {
        if cell.value == value && cell.since <= hour {
            return;
        }
        let before = if cell.since < hour { cell.value } else { cell.before };
        if let Some(held) = cell.value {
            stream.add(first_hour, held, cell.since, hour);
        }
        cell.before = before;
        cell.value = value;
        cell.since = hour;
        if let (Some(before), Some(now)) = (before, value)
            && before != now
        {
            let fold = stream.at(bucket_of(hour, 0), first_hour, now);
            fold.changes += 1;
            fold.falls += u64::from(now < before);
        }
    }

    pub fn hour(&mut self, hour: i32, table: &dyn Cells, log: &TableHour, status: Status, view: Option<&[u32]>) {
        let hour = i64::from(hour);
        if let Shape::Log(facets) = &self.shape {
            if available(status) {
                let (bucket, counts) = ((bucket_of(hour, 0) - self.first_bucket) as u32, &mut self.counts);
                let mut count = |slot: u32| {
                    let mut key = 0u128;
                    for (at, facet) in facets.iter().enumerate().take(4) {
                        let value = match *facet {
                            Facet::Column(column) => table.number(column, slot).map_or(u32::MAX, |value| value as u32),
                            Facet::Key(shift) => table.key(slot).map_or(u32::MAX, |key| (key >> shift) as u32),
                        };
                        key |= u128::from(value) << (32 * (3 - at));
                    }
                    counts.entry(key).or_default().add(bucket);
                };
                if log.whole || log.appended > 0 {
                    table.each_row(&mut count);
                } else {
                    log.inserted.iter().for_each(|&slot| count(slot));
                }
            }
            return;
        }
        let now_available = available(status);
        let truncated = matches!(status, Status::Truncated { .. });
        let everything = log.whole || now_available != self.available || truncated || self.truncated;
        self.available = now_available;
        self.truncated = truncated;
        let hour_of = Applied {
            table,
            view,
            available: now_available,
            hour,
        };
        let (tracks, columns) = (&mut self.tracks, &self.columns);
        if everything {
            for at in 0..tracks.keys.len() {
                let key = tracks.keys[at];
                hour_of.update(tracks, columns, key, table.slot(key), u64::MAX);
            }
            let mut fresh = Vec::new();
            table.each_row(&mut |slot| {
                if let Some(key) = table.key(slot)
                    && !tracks.by_key.contains_key(&key)
                {
                    fresh.push((key, slot));
                }
            });
            for (key, slot) in fresh {
                hour_of.update(tracks, columns, key, Some(slot), u64::MAX);
            }
            return;
        }
        if !now_available {
            return;
        }
        for &(_, key) in &log.deleted {
            if table.slot(key).is_none() {
                hour_of.update(tracks, columns, key, None, u64::MAX);
            }
        }
        for &slot in &log.inserted {
            if let Some(key) = table.key(slot) {
                hour_of.update(tracks, columns, key, Some(slot), u64::MAX);
            }
        }
        let mut cells = log.written.iter().peekable();
        while let Some(&(slot, column)) = cells.next() {
            let mut mask = if column < 64 { 1u64 << column } else { 0 };
            while let Some(&&(_, column)) = cells.peek().filter(|(held, _)| *held == slot) {
                if column < 64 {
                    mask |= 1 << column;
                }
                cells.next();
            }
            if let Some(key) = table.key(slot) {
                hour_of.update(tracks, columns, key, Some(slot), mask);
            }
        }
    }

    pub fn finish(self, end: i32) -> io::Result<Option<Raw>> {
        let end = i64::from(end);
        let buckets = (bucket_of(end - 1, 0) - self.first_bucket + 1) as u32;
        let end_bucket = self.first_bucket + i64::from(buckets);
        let mut directory = Directory {
            level: 0,
            compressed: true,
            first_hour: self.first_hour,
            hours: (end - self.first_hour) as u32,
            first_bucket: self.first_bucket,
            buckets,
            keys: 0,
            shape: self.shape.code(),
            columns: match &self.shape {
                Shape::Values(columns) => columns.iter().map(|column| (column.source.id(), column.aggregates, column.rank)).collect(),
                Shape::Lifetimes => Vec::new(),
                Shape::Log(_) => vec![(0, 0, 0)],
            },
            frames: Vec::new(),
        };
        if let Shape::Log(_) = &self.shape {
            let mut counts = self.counts;
            let mut keys: Vec<u128> = counts.keys().copied().collect();
            if keys.is_empty() {
                return Ok(None);
            }
            keys.sort_unstable();
            directory.keys = keys.len() as u32;
            let life = life_tokens(&vec![Life::default(); buckets as usize], &directory);
            let mut lives = Vec::with_capacity(keys.len() * (life.len() + 1));
            let mut frame = Vec::new();
            for key in &keys {
                put_prefixed(&mut lives, &life);
                put_prefixed(&mut frame, &counts.remove(key).expect("a key counted").finish(buckets));
            }
            return build(directory, &keys, &lives, vec![frame]).map(Some);
        }
        let mut tracks = self.tracks;
        let changes = std::mem::take(&mut tracks.changes).split(tracks.keys.len())?;
        let mut order: Vec<u32> = (0..tracks.keys.len() as u32).collect();
        order.sort_unstable_by_key(|&at| tracks.keys[at as usize]);
        let mut keys = Vec::new();
        let mut lives = Vec::new();
        let mut frames = vec![Vec::new(); self.columns.len()];
        for at in order {
            let at = at as usize;
            let Some((life, tokens)) = tracks.replay(at, changes.track(at), &self.columns, end, end_bucket)? else { continue };
            keys.push(tracks.keys[at]);
            put_prefixed(&mut lives, &life);
            for (frame, tokens) in frames.iter_mut().zip(tokens) {
                put_prefixed(frame, &tokens);
            }
        }
        if keys.is_empty() {
            return Ok(None);
        }
        directory.keys = keys.len() as u32;
        build(directory, &keys, &lives, frames).map(Some)
    }
}

struct Tracks {
    first_hour: i64,
    width: usize,
    words: usize,
    keys: Vec<u128>,
    by_key: HashMap<u128, u32, foldhash::fast::RandomState>,
    values: Vec<i64>,
    present: Vec<u64>,
    opened: Vec<u64>,
    covered: Vec<u64>,
    changes: Changes,
}

impl Tracks {
    fn new(first_hour: i64, width: usize) -> Self {
        Self {
            first_hour,
            width,
            words: width.div_ceil(64),
            keys: Vec::new(),
            by_key: HashMap::default(),
            values: Vec::new(),
            present: Vec::new(),
            opened: Vec::new(),
            covered: Vec::new(),
            changes: Changes::default(),
        }
    }

    fn reserve(&mut self, keys: usize) {
        self.keys.reserve_exact(keys);
        self.by_key.reserve(keys);
        self.values.reserve_exact(keys * self.width);
        self.present.reserve_exact(keys * self.words);
        self.opened.reserve_exact(keys * self.words);
        self.covered.reserve_exact(keys.div_ceil(32));
    }

    fn at(&mut self, key: u128) -> usize {
        if let Some(&at) = self.by_key.get(&key) {
            return at as usize;
        }
        let at = self.keys.len();
        self.keys.push(key);
        self.by_key.insert(key, at as u32);
        self.values.resize(self.values.len() + self.width, 0);
        self.present.resize(self.present.len() + self.words, 0);
        self.opened.resize(self.opened.len() + self.words, 0);
        if at % 32 == 0 {
            self.covered.push(0);
        }
        at
    }

    fn open(&mut self, at: usize, values: impl Iterator<Item = Option<i64>>) {
        self.covered[at / 32] |= 3 << (at % 32 * 2);
        for (column, value) in values.enumerate() {
            let (word, bit) = self.bit(at, column);
            if let Some(value) = value {
                self.values[at * self.width + column] = value;
                self.present[word] |= bit;
            }
        }
    }

    fn bit(&self, at: usize, column: usize) -> (usize, u64) {
        (at * self.words + column / 64, 1 << (column % 64))
    }

    fn value(&self, at: usize, column: usize) -> Option<i64> {
        let (word, bit) = self.bit(at, column);
        (self.present[word] & bit != 0).then(|| self.values[at * self.width + column])
    }

    fn is_covered(&self, at: usize, shift: usize) -> bool {
        self.covered[at / 32] >> (at % 32 * 2 + shift) & 1 != 0
    }

    fn offset(&self, hour: i64) -> u32 {
        (hour - self.first_hour) as u32
    }

    fn cover(&mut self, at: usize, covered: bool, hour: i64) {
        if self.is_covered(at, 0) == covered {
            return;
        }
        self.covered[at / 32] ^= 1 << (at % 32 * 2);
        let offset = self.offset(hour);
        self.changes.head(offset, if covered { COVERED } else { UNCOVERED }, 0, at as u32);
        self.changes.ended();
    }

    fn set(&mut self, at: usize, column: usize, value: Option<i64>, hour: i64) {
        let now = self.value(at, column);
        if now == value {
            return;
        }
        let (word, bit) = self.bit(at, column);
        let (offset, track) = (self.offset(hour), at as u32);
        if self.opened[word] & bit == 0 {
            self.opened[word] |= bit;
            match now {
                Some(now) => {
                    self.changes.head(offset, BEGAN, column, track);
                    put_varint(&mut self.changes.open, zig(now));
                }
                None => self.changes.head(offset, BEGAN_NONE, column, track),
            }
        }
        match value {
            Some(value) => {
                self.changes.head(offset, WROTE, column, track);
                put_varint(&mut self.changes.open, zig(value.wrapping_sub(now.unwrap_or(0))));
                self.values[at * self.width + column] = value;
                self.present[word] |= bit;
            }
            None => {
                self.changes.head(offset, CLEARED, column, track);
                self.present[word] &= !bit;
            }
        }
        self.changes.ended();
    }

    fn replay(&self, at: usize, changes: &[u8], columns: &[Column], end: i64, end_bucket: i64) -> io::Result<Option<(Vec<u8>, Vec<Vec<u8>>)>> {
        let first_hour = self.first_hour;
        let first_bucket = bucket_of(first_hour, 0);
        let mut track = Track {
            covered: self.is_covered(at, 1),
            since: first_hour,
            life: LifeStream::new(first_bucket),
            cells: (0..self.width)
                .map(|column| {
                    let value = self.value(at, column);
                    Cell_ { value, since: first_hour, before: value }
                })
                .collect(),
            streams: columns.iter().map(|column| Stream::new(first_bucket, column.aggregates)).collect(),
        };
        let malformed = || invalid("A summary builder's changes are malformed");
        let mut reader = Reader::new(changes);
        while reader.left() > 0 {
            let hour = first_hour + i64::from(reader.varint_u32()?);
            let head = reader.varint()?;
            let kind = head & ((1 << KIND_BITS) - 1);
            if kind == COVERED || kind == UNCOVERED {
                Builder::cover(first_hour, &mut track, kind == COVERED, hour);
                continue;
            }
            let column = usize::try_from(head >> KIND_BITS).map_err(|_| malformed())?;
            let (Some(cell), Some(stream)) = (track.cells.get_mut(column), track.streams.get_mut(column)) else {
                return Err(malformed());
            };
            match kind {
                WROTE => {
                    let value = unzig(reader.varint()?).wrapping_add(cell.value.unwrap_or(0));
                    Builder::set(first_hour, stream, cell, Some(value), hour);
                }
                CLEARED => Builder::set(first_hour, stream, cell, None, hour),
                BEGAN => {
                    let value = Some(unzig(reader.varint()?));
                    *cell = Cell_ { value, since: first_hour, before: value };
                }
                BEGAN_NONE => *cell = Cell_ { value: None, since: first_hour, before: None },
                _ => return Err(malformed()),
            }
        }
        Builder::cover(first_hour, &mut track, false, end);
        for (cell, stream) in track.cells.iter_mut().zip(&mut track.streams) {
            Builder::set(first_hour, stream, cell, None, end);
        }
        if !track.life.any {
            return Ok(None);
        }
        let life = track.life.finish(end_bucket, first_hour, end);
        let tokens = track.streams.into_iter().map(|stream| stream.finish(end_bucket, first_hour, end)).collect();
        Ok(Some((life, tokens)))
    }

    fn held_bytes(&self) -> usize {
        self.keys.capacity() * size_of::<u128>()
            + self.by_key.capacity() * (size_of::<(u128, u32)>() + 1)
            + self.values.capacity() * size_of::<i64>()
            + (self.present.capacity() + self.opened.capacity() + self.covered.capacity()) * size_of::<u64>()
            + self.changes.held_bytes()
    }
}

struct Applied<'a> {
    table: &'a dyn Cells,
    view: Option<&'a [u32]>,
    available: bool,
    hour: i64,
}

impl Applied<'_> {
    fn update(&self, tracks: &mut Tracks, columns: &[Column], key: u128, slot: Option<u32>, mask: u64) {
        let at = tracks.at(key);
        let read = slot.filter(|&slot| self.available && self.view.is_none_or(|view| view.binary_search(&slot).is_ok()));
        tracks.cover(at, read.is_some(), self.hour);
        for (index, column) in columns.iter().enumerate() {
            if read.is_some() && !column.source.reads(mask) && tracks.value(at, index).is_some() {
                continue;
            }
            let value = read.and_then(|slot| column.source.value(self.table, slot));
            tracks.set(at, index, value, self.hour);
        }
    }
}


pub fn fold(pieces: &[&[u8]], first_hour: i64, hours: u32) -> io::Result<Option<Raw>> {
    let read: Vec<Piece<'_>> = pieces.iter().map(|bytes| Piece::read(bytes)).collect::<io::Result<_>>()?;
    let keys = merged(&read)?;
    fold_keys(&read, &keys, first_hour, hours)
}

fn merged(read: &[Piece<'_>]) -> io::Result<Vec<u128>> {
    if read.iter().any(|piece| !piece.keys.is_sorted_by(|before, after| before < after)) {
        return Err(invalid("A summary piece's keys do not ascend"));
    }
    let mut keys = Vec::with_capacity(Merge::new(read).count());
    keys.extend(Merge::new(read));
    Ok(keys)
}

struct Merge<'a> {
    heads: BinaryHeap<Reverse<(u128, usize)>>,
    rests: Vec<std::slice::Iter<'a, u128>>,
    last: Option<u128>,
}

impl<'a> Merge<'a> {
    fn new(read: &'a [Piece<'_>]) -> Self {
        let mut rests: Vec<_> = read.iter().map(|piece| piece.keys.iter()).collect();
        let heads = rests.iter_mut().enumerate().filter_map(|(at, rest)| rest.next().map(|&key| Reverse((key, at)))).collect();
        Self { heads, rests, last: None }
    }
}

impl Iterator for Merge<'_> {
    type Item = u128;

    fn next(&mut self) -> Option<u128> {
        loop {
            let mut least = self.heads.peek_mut()?;
            let Reverse((key, at)) = *least;
            match self.rests[at].next() {
                Some(&after) => *least = Reverse((after, at)),
                None => _ = PeekMut::pop(least),
            }
            if self.last.replace(key) != Some(key) {
                return Some(key);
            }
        }
    }
}

fn fold_keys(read: &[Piece<'_>], keys: &[u128], first_hour: i64, hours: u32) -> io::Result<Option<Raw>> {
    let Some(first) = read.first() else {
        return Ok(None);
    };
    let level = first.directory.level;
    let shape = first.directory.shape;
    let columns = first.directory.columns.clone();
    let next = first_hour + i64::from(hours);
    let mut after = first_hour;
    for piece in read {
        let end = piece.directory.first_hour + i64::from(piece.directory.hours);
        if piece.directory.level != level
            || piece.directory.shape != shape
            || piece.directory.columns != columns
            || piece.directory.first_hour < after
            || end > next
        {
            return Err(invalid("Folded summary pieces do not follow one another within their span"));
        }
        after = end;
    }
    let up = level + 1;
    let first_bucket = bucket_of(first_hour, up);
    let buckets = (bucket_of(next - 1, up) - first_bucket + 1) as usize;
    let directory = Directory {
        level: up,
        compressed: true,
        first_hour,
        hours,
        first_bucket,
        buckets: buckets as u32,
        keys: keys.len() as u32,
        shape,
        columns: columns.clone(),
        frames: Vec::new(),
    };
    let (size, upper_size) = (bucket_hours(level), bucket_hours(up));
    let place = |piece: &Piece<'_>, at: usize| -> (usize, u32) {
        let bucket = piece.directory.first_bucket + at as i64;
        let start = bucket * size;
        let upper = bucket_of(start, up);
        ((upper - first_bucket) as usize, (start - upper * upper_size) as u32)
    };
    let meets = |piece: &Piece<'_>, from: u32, count: u64, visit: &mut dyn FnMut(usize, i64, i64)| {
        let (first, last) = (piece.directory.first_hour, piece.directory.first_hour + i64::from(piece.directory.hours));
        let low = piece.directory.first_bucket + i64::from(from);
        let high = low + count as i64;
        for upper in low.div_euclid(FANOUT)..=(high - 1).div_euclid(FANOUT) {
            let start = (low.max(upper * FANOUT) * size).max(first);
            let end = (high.min((upper + 1) * FANOUT) * size).min(last);
            if end > start {
                visit((upper - first_bucket) as usize, start, end);
            }
        }
    };
    let spans = |count: u64, at: u32, buckets: u32| -> io::Result<u32> {
        u32::try_from(count)
            .ok()
            .filter(|&count| count <= buckets - at)
            .ok_or_else(|| invalid("A key's tokens pass its piece's buckets"))
    };
    let mut life_frame = Vec::new();
    let mut life = vec![Life::default(); buckets];
    for key in keys {
        life.fill(Life::default());
        for piece in read {
            let Some(held) = piece.key_at(*key) else { continue };
            let pieces_buckets = piece.directory.buckets;
            let mut lives = piece.lives(held);
            let mut at = 0;
            while at < pieces_buckets {
                match lives.span()? {
                    LifeSpan::Empty(count) => at += spans(count, at, pieces_buckets)?,
                    LifeSpan::Full(count) => {
                        let count = spans(count, at, pieces_buckets)?;
                        meets(piece, at, u64::from(count), &mut |upper, start, end| {
                            let base = (first_bucket + upper as i64) * upper_size;
                            let target = &mut life[upper];
                            let lived = Life {
                                hours: (end - start) as u32,
                                first: (start - base) as u32,
                                end: (end - base) as u32,
                            };
                            target.merge(&lived, 0);
                        });
                        at += count;
                    }
                    LifeSpan::Part(lived) => {
                        let (upper, offset) = place(piece, at as usize);
                        life[upper].merge(&lived, offset);
                        at += 1;
                    }
                }
            }
            if !lives.finished() {
                return Err(invalid("A key's life passes its piece's buckets"));
            }
        }
        put_prefixed(&mut life_frame, &life_tokens(&life, &directory));
    }
    let mut frames = Vec::with_capacity(columns.len());
    let mut folds: Vec<Option<Fold>> = vec![None; buckets];
    for column in 0..columns.len() {
        let decoded: Vec<ColumnFrame> = read.iter().map(|piece| piece.column(column)).collect::<io::Result<_>>()?;
        let mut frame = Vec::new();
        for key in keys {
            if shape == 2 {
                let mut counts = vec![0u64; buckets];
                for (piece, frame) in read.iter().zip(&decoded) {
                    if let Some(held) = piece.key_at(*key) {
                        for (at, count) in frame.counts(piece, held)?.into_iter().enumerate() {
                            counts[place(piece, at).0] += count;
                        }
                    }
                }
                put_prefixed(&mut frame, &Counts::of(&counts));
                continue;
            }
            folds.fill(None);
            let add = |slot: &mut Option<Fold>, fold: Fold| match slot {
                Some(earlier) => earlier.merge(&fold),
                None => *slot = Some(fold),
            };
            for (piece, frame) in read.iter().zip(&decoded) {
                let Some(held) = piece.key_at(*key) else { continue };
                let pieces_buckets = piece.directory.buckets;
                let mut reader = frame.tokens(held);
                let mut at = 0;
                while at < pieces_buckets {
                    let index = piece.directory.first_bucket + i64::from(at);
                    match reader.span(index, piece.directory.covered(at))? {
                        Span::None(count) => at += spans(count, at, pieces_buckets)?,
                        Span::One(fold) => {
                            add(&mut folds[place(piece, at as usize).0], fold);
                            at += 1;
                        }
                        Span::Same(value, count) => {
                            let count = spans(count, at, pieces_buckets)?;
                            meets(piece, at, u64::from(count), &mut |upper, start, end| {
                                add(&mut folds[upper], Fold::constant(value, (end - start) as u32, first_bucket + upper as i64));
                            });
                            at += count;
                        }
                    }
                }
                if !reader.exhausted() {
                    return Err(invalid("A key's tokens pass its piece's buckets"));
                }
            }
            let mut writer = TokenWriter::new(columns[column].1);
            for (at, fold) in folds.iter().enumerate() {
                writer.bucket(fold.as_ref(), directory.covered(at as u32));
            }
            put_prefixed(&mut frame, &writer.finish());
        }
        frames.push(frame);
    }
    build(directory, keys, &life_frame, frames).map(Some)
}

pub mod cost {
    use super::{BASE_HOURS, CHANGE_ROOM, CHUNK, Counts, FANOUT, Shape};

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Load {
        pub keys: u64,
        pub changes: u64,
    }

    pub const CHANGE: u64 = 5;
    pub const OPEN: u64 = 8;
    pub const SHRINK: u64 = 2;
    pub const TOKEN: u64 = 10;
    pub const PACKED: u64 = 4;
    const READ_KEY: u64 = 32;
    const BUCKETS: u64 = 720 / BASE_HOURS as u64 + 1;

    const fn hashed(entry: u64) -> u64 {
        (entry + 1) * 8 * 3 / (7 * 2)
    }

    fn columns(shape: &Shape) -> (u64, u64) {
        match shape {
            Shape::Values(columns) => (columns.len() as u64, columns.iter().map(|column| u64::from(column.rank)).sum()),
            Shape::Lifetimes | Shape::Log(_) => (0, 0),
        }
    }

    fn written(shape: &Shape, load: Load) -> u64 {
        let (width, _) = columns(shape);
        match load.changes {
            0 => 0,
            changes => load.keys * width.max(1) * (changes * CHANGE + OPEN),
        }
    }

    pub fn held(shape: &Shape, load: Load) -> u64 {
        let Load { keys, changes } = load;
        if let Shape::Log(_) = shape {
            let counts = size_of::<(u128, Counts)>() as u64;
            return keys * (hashed(counts) + 8 + 6 * changes.min(BUCKETS) * 3 / 2);
        }
        let (width, _) = columns(shape);
        let per_key = size_of::<u128>() as u64 + 8 * width + 16 * width.div_ceil(64) + 1 + hashed(size_of::<(u128, u32)>() as u64);
        let chunks = written(shape, load) / CHUNK as u64 + 1;
        let open = if changes > 0 { (CHUNK + CHANGE_ROOM) as u64 } else { 0 };
        keys * per_key + written(shape, load) / SHRINK + chunks * size_of::<(Vec<u8>, Option<usize>)>() as u64 * 2 + open
    }

    pub fn piece(shape: &Shape, load: Load) -> u64 {
        let (width, ranked) = columns(shape);
        let tokens = match shape {
            Shape::Log(_) => column(shape, load),
            _ => width * column(shape, load),
        };
        tokens + load.keys * 6 + BUCKETS * (3 * width + ranked)
    }

    pub fn finishing(shape: &Shape, load: Load) -> u64 {
        let split = written(shape, load) * 6 / 5 + load.keys * size_of::<usize>() as u64;
        let piece = piece(shape, load);
        split + piece + piece / PACKED
    }

    fn column(shape: &Shape, load: Load) -> u64 {
        let tokens = match shape {
            Shape::Lifetimes => return 0,
            Shape::Values(_) => (load.changes + 1).min(BUCKETS),
            Shape::Log(_) => load.changes.min(BUCKETS),
        };
        load.keys * (2 + TOKEN * tokens)
    }

    pub fn folding(shape: &Shape, load: Load) -> u64 {
        let piece = piece(shape, load);
        let merging = size_of::<(u128, usize)>() + size_of::<std::slice::Iter<'static, u128>>();
        let merged = load.keys * size_of::<u128>() as u64 + FANOUT as u64 * merging as u64;
        FANOUT as u64 * (piece / PACKED + load.keys * READ_KEY + column(shape, load)) + merged + piece + piece / PACKED
    }

    pub fn peak(tables: &[(Shape, Load)]) -> u64 {
        let mut after: u64 = tables.iter().map(|(shape, load)| held(shape, *load)).sum();
        let (mut waiting, mut most) = (0, 0);
        for (shape, load) in tables {
            let own = held(shape, *load);
            after -= own;
            most = most.max(after + own + waiting + finishing(shape, *load)).max(after + waiting + folding(shape, *load));
            waiting += 2 * (piece(shape, *load) / PACKED);
        }
        most
    }
}
