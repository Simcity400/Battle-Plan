use crate::{
    codec::{Binary, Decode, Encode},
    invalid,
};
use std::io;

pub type KeyHandle = u64;
pub type PageId = u64;
pub const NONE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub first: i32,
    pub last: i32,
}
impl Span {
    pub fn new(first: i32, last: i32) -> io::Result<Self> {
        if first > last {
            return Err(invalid("reversed series extent"));
        }
        Ok(Self { first, last })
    }
    pub fn contains(self, hour: i32) -> bool {
        self.first <= hour && hour <= self.last
    }
    pub fn intersect(self, other: Self) -> Option<Self> {
        let first = self.first.max(other.first);
        let last = self.last.min(other.last);
        (first <= last).then_some(Self { first, last })
    }
    pub fn hours(self) -> u64 {
        (i64::from(self.last) - i64::from(self.first) + 1) as u64
    }
}
impl Binary for Span {
    const SIZE: usize = 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        Self::new(self.first, self.last)?;
        e.i32(self.first)?;
        e.i32(self.last)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Self::new(d.i32()?, d.i32()?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub span: Span,
    pub value: i128,
    pub break_before: bool,
    pub supplying_boundary: bool,
    pub canonical: u64,
    pub sequence: u64,
}
impl Binary for Run {
    const SIZE: usize = 41;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.span.encode(e)?;
        e.i128(self.value)?;
        e.u8(u8::from(self.break_before) | u8::from(self.supplying_boundary) << 1)?;
        e.u64(self.canonical)?;
        e.u64(self.sequence)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let span = Span::decode(d)?;
        let value = d.i128()?;
        let flags = d.u8()?;
        if flags > 3 {
            return Err(invalid("invalid run flags"));
        }
        Ok(Self {
            span,
            value,
            break_before: flags & 1 != 0,
            supplying_boundary: flags & 2 != 0,
            canonical: d.u64()?,
            sequence: d.u64()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knownness {
    Known,
    Unknown(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub span: Span,
    pub known: Knownness,
    pub supplying_read: Option<i32>,
    pub holder: Option<u64>,
    pub identity_first: Option<i32>,
}
impl Binary for Evidence {
    const SIZE: usize = 33;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.span.encode(e)?;
        e.u8(u8::from(matches!(self.known, Knownness::Known))
            | u8::from(self.supplying_read.is_some()) << 1
            | u8::from(self.holder.is_some()) << 2
            | u8::from(self.identity_first.is_some()) << 3)?;
        e.u64(match self.known {
            Knownness::Known => 0,
            Knownness::Unknown(v) => v,
        })?;
        e.i32(self.supplying_read.unwrap_or(0))?;
        e.u64(self.holder.unwrap_or(0))?;
        e.i32(self.identity_first.unwrap_or(0))
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let span = Span::decode(d)?;
        let f = d.u8()?;
        if f > 15 {
            return Err(invalid("evidence flags"));
        }
        let reason = d.u64()?;
        let read = d.i32()?;
        let holder = d.u64()?;
        let first = d.i32()?;
        Ok(Self {
            span,
            known: if f & 1 != 0 {
                Knownness::Known
            } else {
                Knownness::Unknown(reason)
            },
            supplying_read: (f & 2 != 0).then_some(read),
            holder: (f & 4 != 0).then_some(holder),
            identity_first: (f & 8 != 0).then_some(first),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Value { run: Run, evidence: Evidence },
    KnownAbsent(Evidence),
    Unknown(Option<Evidence>),
    Pending,
}

/// A reference into the family's variable-width heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blob {
    pub at: u64,
    pub len: u64,
}
impl Blob {
    pub const EMPTY: Self = Self { at: NONE, len: 0 };
}
impl Binary for Blob {
    const SIZE: usize = 16;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        if (self.len == 0) != (self.at == NONE) {
            return Err(invalid("invalid variable-width reference"));
        }
        e.u64(self.at)?;
        e.u64(self.len)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let v = Self {
            at: d.u64()?,
            len: d.u64()?,
        };
        if (v.len == 0) != (v.at == NONE) || (v.len != 0 && v.at.checked_add(v.len).is_none()) {
            return Err(invalid("blob reference/length mismatch"));
        }
        Ok(v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cell {
    Null,
    Integer(i128),
    Unsigned(u64),
    Number(f64),
    Boolean(bool),
    Text(Blob),
    List(Blob),
}
impl Binary for Cell {
    const SIZE: usize = 17;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        let mut bytes = [0; 16];
        let mut d = Encode::new(&mut bytes);
        let tag = match self {
            Self::Null => 0,
            Self::Integer(v) => {
                d.i128(*v)?;
                1
            }
            Self::Unsigned(v) => {
                d.u64(*v)?;
                2
            }
            Self::Number(v) => {
                if !v.is_finite() {
                    return Err(invalid("nonfinite event field"));
                }
                d.f64(*v)?;
                3
            }
            Self::Boolean(v) => {
                d.u8(u8::from(*v))?;
                4
            }
            Self::Text(v) => {
                v.encode(&mut d)?;
                5
            }
            Self::List(v) => {
                v.encode(&mut d)?;
                6
            }
        };
        e.u8(tag)?;
        e.bytes(&bytes)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let tag = d.u8()?;
        let mut v = Decode::new(d.bytes(16)?);
        match tag {
            0 => Ok(Self::Null),
            1 => Ok(Self::Integer(v.i128()?)),
            2 => Ok(Self::Unsigned(v.u64()?)),
            3 => Ok(Self::Number(v.f64()?)),
            4 => match v.u8()? {
                0 => Ok(Self::Boolean(false)),
                1 => Ok(Self::Boolean(true)),
                _ => Err(invalid("invalid boolean")),
            },
            5 => Ok(Self::Text(Blob::decode(&mut v)?)),
            6 => {
                let b = Blob::decode(&mut v)?;
                if b.len % Self::SIZE as u64 != 0 {
                    return Err(invalid("child table length"));
                }
                Ok(Self::List(b))
            }
            _ => Err(invalid("unknown cell type")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySpec {
    pub schema: u64,
    pub actor: u64,
    pub measure: u64,
    pub dimensions: Blob,
}
impl Binary for KeySpec {
    const SIZE: usize = 40;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.schema)?;
        e.u64(self.actor)?;
        e.u64(self.measure)?;
        self.dimensions.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            schema: d.u64()?,
            actor: d.u64()?,
            measure: d.u64()?,
            dimensions: Blob::decode(d)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDefinition {
    pub id: KeyHandle,
    pub spec: KeySpec,
    pub label: Blob,
    pub first_seen: u64,
}
impl Binary for KeyDefinition {
    const SIZE: usize = 72;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.id)?;
        self.spec.encode(e)?;
        self.label.encode(e)?;
        e.u64(self.first_seen)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            id: d.u64()?,
            spec: KeySpec::decode(d)?,
            label: Blob::decode(d)?,
            first_seen: d.u64()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub key: KeyHandle,
    pub native_hour: i32,
    pub read_hour: Option<i32>,
    pub sequence: u64,
    pub kind: u64,
    pub fields: Blob,
}
impl Binary for Event {
    const SIZE: usize = 49;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.key)?;
        e.i32(self.native_hour)?;
        e.u8(u8::from(self.read_hour.is_some()))?;
        e.i32(self.read_hour.unwrap_or(0))?;
        e.u64(self.sequence)?;
        e.u64(self.kind)?;
        self.fields.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let key = d.u64()?;
        let native_hour = d.i32()?;
        let f = d.u8()?;
        if f > 1 {
            return Err(invalid("event read validity"));
        }
        let read = d.i32()?;
        let v = Self {
            key,
            native_hour,
            read_hour: (f == 1).then_some(read),
            sequence: d.u64()?,
            kind: d.u64()?,
            fields: Blob::decode(d)?,
        };
        if v.fields.len % Cell::SIZE as u64 != 0 {
            return Err(invalid("event field table length"));
        }
        Ok(v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub source: [u8; 32],
    pub content: [u8; 32],
    pub branch: [u8; 32],
    pub schema: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publication {
    pub generation: u64,
    pub segment: u64,
    pub accepted: Span,
    pub reached: i32,
    pub refusal: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Exact,
    OrderedContributions,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
    Home,
    End,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventOrder {
    Native,
    Publication,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Approximation {
    pub span: Span,
    pub value: f64,
}
