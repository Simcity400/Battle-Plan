use std::cmp::Ordering;
use std::io;
use std::sync::Arc;

pub type Hour = i32;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Key(Bytes);

impl Key {
    pub fn from_varint(bytes: &[u8]) -> io::Result<Self> {
        Self::from_varint_in(&crate::Memory::global(), bytes)
    }

    pub fn from_varint_in(memory: &crate::Memory, bytes: &[u8]) -> io::Result<Self> {
        if bytes.is_empty()
            || bytes.last().unwrap() & 0x80 != 0
            || bytes[..bytes.len() - 1].iter().any(|b| b & 0x80 == 0)
            || (bytes.len() > 1 && bytes.last() == Some(&0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "noncanonical unsigned varint key",
            ));
        }
        Ok(Self(Bytes::copy_from(
            memory,
            crate::Owner::Metadata,
            bytes,
        )?))
    }

    pub fn varint(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn shared_varint(bytes: Bytes) -> Self {
        Self(bytes)
    }

    pub fn from_u64_in(memory: &crate::Memory, mut value: u64) -> io::Result<Self> {
        let mut bytes = [0; 10];
        let mut len = 0;
        loop {
            bytes[len] = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                bytes[len] |= 0x80;
            }
            len += 1;
            if value == 0 {
                break;
            }
        }
        Self::from_varint_in(memory, &bytes[..len])
    }
}

impl From<u64> for Key {
    fn from(value: u64) -> Self {
        Self::from_u64_in(&crate::Memory::global(), value)
            .expect("key exceeds playback working set")
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.len().cmp(&other.0.len()).then_with(|| {
            self.0
                .iter()
                .rev()
                .map(|b| b & 0x7f)
                .cmp(other.0.iter().rev().map(|b| b & 0x7f))
        })
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Address {
    pub key: Key,
    pub field: Key,
}

#[derive(Clone, Debug)]
pub enum KeyFilter {
    All,
    Keys(Arc<[Address]>),
}

impl KeyFilter {
    pub fn sorted(keys: Arc<[Address]>) -> io::Result<Self> {
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "filter keys must be unique and ordered",
            ));
        }
        Ok(Self::Keys(keys))
    }

    pub fn contains(&self, key: &Address) -> bool {
        match self {
            Self::All => true,
            Self::Keys(keys) => keys.binary_search(key).is_ok(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hours {
    pub first: Hour,
    pub last: Hour,
}

impl Hours {
    pub fn new(first: Hour, last: Hour) -> io::Result<Self> {
        if first > last {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reversed hour span",
            ));
        }
        Ok(Self { first, last })
    }
    pub fn contains(self, hour: Hour) -> bool {
        self.first <= hour && hour <= self.last
    }
    pub fn intersects(self, other: Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let first = self.first.max(other.first);
        let last = self.last.min(other.last);
        (first <= last).then_some(Self { first, last })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingKind {
    Unknown,
    Refused,
    Lost,
    NotRecorded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Missing {
    pub kind: MissingKind,
    pub reason: Text,
    pub hours: Hours,
}

struct Allocation {
    bytes: crate::Buffer<u8>,
    _memory: crate::Reservation,
}

#[derive(Clone)]
pub struct Bytes {
    allocation: Arc<Allocation>,
    first: usize,
    len: usize,
}

impl Bytes {
    pub(crate) const fn allocation_bytes() -> usize {
        std::mem::size_of::<Allocation>() + 2 * std::mem::size_of::<usize>()
    }
    pub fn copy_from(
        memory: &crate::Memory,
        owner: crate::Owner,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut buffer = crate::Buffer::new(memory, owner, bytes.len())?;
        for byte in bytes {
            buffer.push(*byte);
        }
        Self::from_buffer(memory, owner, buffer)
    }
    pub fn from_buffer(
        memory: &crate::Memory,
        owner: crate::Owner,
        bytes: crate::Buffer<u8>,
    ) -> io::Result<Self> {
        let reservation = memory.reserve(
            owner,
            std::mem::size_of::<Allocation>() + 2 * std::mem::size_of::<usize>(),
        )?;
        let len = bytes.len();
        Ok(Self {
            allocation: Arc::new(Allocation {
                bytes,
                _memory: reservation,
            }),
            first: 0,
            len,
        })
    }
    pub fn slice(&self, first: usize, len: usize) -> io::Result<Self> {
        if first.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "value slice out of bounds",
            ));
        }
        Ok(Self {
            allocation: self.allocation.clone(),
            first: self.first + first,
            len,
        })
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.allocation.bytes[self.first..self.first + self.len]
    }
}

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Bytes {}

impl std::hash::Hash for Bytes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_slice(), state);
    }
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Text(Bytes);

impl Text {
    pub fn copy_from(memory: &crate::Memory, owner: crate::Owner, text: &str) -> io::Result<Self> {
        Ok(Self(Bytes::copy_from(memory, owner, text.as_bytes())?))
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.0.as_slice()).unwrap()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Present(Bytes),
    Number(f64),
    Fixed(i128),
    Accumulating(Accumulation),
    End,
    Missing(Missing),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Provenance {
    pub supplying_hour: Option<Hour>,
    pub holder: Option<Key>,
    pub identity_first_hour: Option<Hour>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub hour: Hour,
    pub value: Value,
    pub provenance: Provenance,
    pub break_before: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub hour: Hour,
    pub address: Address,
    pub record: Record,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Coverage {
    pub hours: Hours,
    pub missing: Option<Missing>,
    pub accepted_before: u64,
}

impl Coverage {
    pub fn accepted_through(&self, hour: Hour) -> u64 {
        self.accepted_before
            + if self.missing.is_none() && hour >= self.hours.first {
                (i64::from(hour.min(self.hours.last)) - i64::from(self.hours.first) + 1) as u64
            } else {
                0
            }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Total {
    Known(i64),
    Missing(Missing),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Accumulation {
    pub total: Total,
    pub rate: i64,
    pub anchor_hour: Hour,
    pub accepted_at_anchor: u64,
    pub rate_began: Hour,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissingRef<'a> {
    pub kind: MissingKind,
    pub reason: &'a str,
    pub hours: Hours,
}

impl Missing {
    pub fn borrowed(&self) -> MissingRef<'_> {
        MissingRef {
            kind: self.kind,
            reason: self.reason.as_str(),
            hours: self.hours,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TotalRef<'a> {
    Known(i64),
    Missing(MissingRef<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccumulationRef<'a> {
    pub total: TotalRef<'a>,
    pub rate: i64,
    pub anchor_hour: Hour,
    pub accepted_at_anchor: u64,
    pub rate_began: Hour,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValueRef<'a> {
    Present(&'a [u8]),
    Number(f64),
    Fixed(i128),
    Accumulating(AccumulationRef<'a>),
    End,
    Missing(MissingRef<'a>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProvenanceRef<'a> {
    pub supplying_hour: Option<Hour>,
    pub holder: Option<&'a [u8]>,
    pub identity_first_hour: Option<Hour>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordRef<'a> {
    pub hour: Hour,
    pub value: ValueRef<'a>,
    pub provenance: ProvenanceRef<'a>,
    pub break_before: bool,
}

impl Record {
    pub fn borrowed(&self) -> RecordRef<'_> {
        RecordRef {
            hour: self.hour,
            value: match &self.value {
                Value::Present(bytes) => ValueRef::Present(bytes),
                Value::Number(value) => ValueRef::Number(*value),
                Value::Fixed(value) => ValueRef::Fixed(*value),
                Value::Accumulating(value) => ValueRef::Accumulating(AccumulationRef {
                    total: match &value.total {
                        Total::Known(total) => TotalRef::Known(*total),
                        Total::Missing(missing) => TotalRef::Missing(missing.borrowed()),
                    },
                    rate: value.rate,
                    anchor_hour: value.anchor_hour,
                    accepted_at_anchor: value.accepted_at_anchor,
                    rate_began: value.rate_began,
                }),
                Value::End => ValueRef::End,
                Value::Missing(missing) => ValueRef::Missing(missing.borrowed()),
            },
            provenance: ProvenanceRef {
                supplying_hour: self.provenance.supplying_hour,
                holder: self.provenance.holder.as_ref().map(Key::varint),
                identity_first_hour: self.provenance.identity_first_hour,
            },
            break_before: self.break_before,
        }
    }
}

impl<'a> RecordRef<'a> {
    pub fn evaluated(mut self, accepted: u64) -> Self {
        if let ValueRef::Accumulating(rate) = self.value {
            self.value = match rate.total {
                TotalRef::Known(total) => ValueRef::Fixed(
                    i128::from(total)
                        + i128::from(rate.rate)
                            * (i128::from(accepted) - i128::from(rate.accepted_at_anchor)),
                ),
                TotalRef::Missing(missing) => ValueRef::Missing(missing),
            };
        }
        self
    }
}
