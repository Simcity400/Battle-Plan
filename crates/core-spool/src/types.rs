use crate::invalid;
use std::{io, marker::PhantomData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Rows = 0,
    Columns = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None = 0,
    Lz4 = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind(pub(crate) u8);
#[expect(non_upper_case_globals, reason = "the kinds are named as the enum variants they stand for")]
impl Kind {
    pub const Appeared: Self = Self(0);
    pub const Changed: Self = Self(1);
    pub const Gone: Self = Self(2);
    pub const Unknown: Self = Self(3);
    pub const Refused: Self = Self(4);
    pub const Status: Self = Self(5);
    pub const Read: Self = Self(6);
    pub const SegmentStart: Self = Self(7);
    pub const Discontinuity: Self = Self(8);
    pub const VisibleKey: Self = Self(9);
    pub const RowStart: Self = Self(10);
    pub const Acquired: Self = Self(11);
    pub const Supplied: Self = Self(12);
    pub const Observed: Self = Self(13);
    pub const Reread: Self = Self(14);
    pub const Lost: Self = Self(15);
    pub const NotRecorded: Self = Self(16);
    pub fn code(self) -> u8 {
        self.0
    }
    pub(crate) fn evidence(self) -> bool {
        matches!(self.0, 3..=5 | 15..=16)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Type {
    Unit,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    Bool,
    OptionalI32,
    OptionalU32,
    OptionalI64,
    OptionalU64,
    Bytes,
    Text,
    U32List,
    U64List,
    Evidence,
}
impl Type {
    pub fn width(self) -> usize {
        match self {
            Self::Unit => 0,
            Self::U8 | Self::I8 | Self::Bool => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 => 4,
            Self::U64 | Self::I64 => 8,
            Self::OptionalI32 | Self::OptionalU32 => 5,
            Self::OptionalI64 | Self::OptionalU64 => 9,
            Self::Bytes | Self::U32List | Self::U64List => 16,
            Self::Text => 21,
            Self::Evidence => 43,
        }
    }
    pub(crate) fn parse(tag: u8) -> io::Result<Self> {
        const ALL: [Type; 19] = [
            Type::Unit,
            Type::U8,
            Type::I8,
            Type::U16,
            Type::I16,
            Type::U32,
            Type::I32,
            Type::U64,
            Type::I64,
            Type::Bool,
            Type::OptionalI32,
            Type::OptionalU32,
            Type::OptionalI64,
            Type::OptionalU64,
            Type::Bytes,
            Type::Text,
            Type::U32List,
            Type::U64List,
            Type::Evidence,
        ];
        ALL.get(tag as usize)
            .copied()
            .ok_or_else(|| invalid("unknown typed lane"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blob {
    pub offset: u64,
    pub len: u64,
}
impl Blob {
    pub(crate) fn decode(bytes: &[u8]) -> Self {
        Self {
            offset: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            len: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Evidence<'a> {
    pub code: u8,
    pub reason: &'a [u8],
    pub first: i64,
    pub last: i64,
    pub sample: Option<i64>,
    pub missing: Option<u64>,
}
pub enum Value<'a> {
    Unit,
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Bytes(&'a [u8]),
    Text { id: u32, text: Option<&'a [u8]> },
    U32List(&'a [u32]),
    U64List(&'a [u64]),
    Evidence(Evidence<'a>),
}
pub struct Record<'a> {
    pub hour: i64,
    pub kind: Kind,
    pub table: u32,
    pub field: u32,
    pub key: &'a [u8],
    pub value: Value<'a>,
}

mod sealed {
    pub trait Sealed {}
}
pub trait Native: sealed::Sealed + Copy {
    const TYPE: Type;
    fn decode(bytes: &[u8]) -> Self;
}
macro_rules! native {
    ($ty:ty, $tag:ident) => {
        impl sealed::Sealed for $ty {}
        impl Native for $ty {
            const TYPE: Type = Type::$tag;
            fn decode(bytes: &[u8]) -> Self {
                <$ty>::from_le_bytes(bytes.try_into().unwrap())
            }
        }
    };
}
native!(u8, U8);
native!(i8, I8);
native!(u16, U16);
native!(i16, I16);
native!(u32, U32);
native!(i32, I32);
native!(u64, U64);
native!(i64, I64);
impl sealed::Sealed for bool {}
impl Native for bool {
    const TYPE: Type = Type::Bool;
    fn decode(b: &[u8]) -> Self {
        b[0] != 0
    }
}
impl sealed::Sealed for () {}
impl Native for () {
    const TYPE: Type = Type::Unit;
    fn decode(_: &[u8]) {}
}
macro_rules! optional {
    ($ty:ty, $tag:ident) => {
        impl sealed::Sealed for Option<$ty> {}
        impl Native for Option<$ty> {
            const TYPE: Type = Type::$tag;
            fn decode(b: &[u8]) -> Self {
                (b[0] != 0).then(|| <$ty>::from_le_bytes(b[1..].try_into().unwrap()))
            }
        }
    };
}
optional!(i32, OptionalI32);
optional!(u32, OptionalU32);
optional!(i64, OptionalI64);
optional!(u64, OptionalU64);

#[derive(Clone, Copy)]
pub struct KeyId<'a>(pub(crate) &'a [u8]);
impl KeyId<'_> {
    pub fn bytes(&self) -> &[u8] {
        self.0
    }
    pub fn index(self) -> u64 {
        let mut b = [0; 8];
        b[..self.0.len()].copy_from_slice(self.0);
        u64::from_le_bytes(b)
    }
}
#[derive(Clone, Copy)]
pub struct Lane<'a> {
    pub hour: i64,
    pub table: u32,
    pub field: u32,
    pub value_type: Type,
    pub(crate) data: &'a [u8],
    pub(crate) count: usize,
    pub(crate) ids: usize,
    pub(crate) orders: usize,
    pub(crate) base_order: u64,
    pub(crate) layout: Layout,
}
impl<'a> Lane<'a> {
    pub fn len(self) -> usize {
        self.count
    }
    pub fn is_empty(self) -> bool {
        self.count == 0
    }
    fn part(self, index: usize, start: usize, width: usize) -> &'a [u8] {
        let stride = self.orders + 1 + self.ids + self.value_type.width();
        let at = match self.layout {
            Layout::Rows => index * stride + start,
            Layout::Columns => start * self.count + index * width,
        };
        &self.data[at..at + width]
    }
    pub fn record(self, index: usize) -> Option<RecordView<'a>> {
        if index >= self.count {
            return None;
        }
        Some(RecordView {
            order: {
                let mut bytes = [0; 8];
                bytes[..self.orders].copy_from_slice(self.part(index, 0, self.orders));
                self.base_order.saturating_add(u64::from_le_bytes(bytes))
            },
            kind: Kind(self.part(index, self.orders, 1)[0]),
            key: KeyId(self.part(index, self.orders + 1, self.ids)),
            value_type: self.value_type,
            value: self.part(index, self.orders + 1 + self.ids, self.value_type.width()),
        })
    }
    pub fn typed<T: Native>(self) -> io::Result<TypedLane<'a, T>> {
        if self.value_type != T::TYPE {
            return Err(invalid("requested native type differs from lane"));
        }
        Ok(TypedLane {
            lane: self,
            _type: PhantomData,
        })
    }
    pub fn records(self) -> impl ExactSizeIterator<Item = RecordView<'a>> {
        (0..self.count).map(move |i| self.record(i).unwrap())
    }
}
pub struct RecordView<'a> {
    pub order: u64,
    pub kind: Kind,
    pub key: KeyId<'a>,
    pub value_type: Type,
    pub value: &'a [u8],
}
impl RecordView<'_> {
    pub fn blob(&self) -> Option<Blob> {
        match self.value_type {
            Type::Bytes | Type::U32List | Type::U64List => Some(Blob::decode(self.value)),
            Type::Text => (self.value[4] != 0).then(|| Blob::decode(&self.value[5..])),
            _ => None,
        }
    }
}
pub struct TypedLane<'a, T> {
    lane: Lane<'a>,
    _type: PhantomData<T>,
}
impl<'a, T: Native> TypedLane<'a, T> {
    pub fn iter(self) -> impl ExactSizeIterator<Item = (u64, Kind, KeyId<'a>, T)> {
        self.lane
            .records()
            .map(|r| (r.order, r.kind, r.key, T::decode(r.value)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRef {
    pub id: u32,
    pub text: Blob,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct U32ListRef(pub Blob);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct U64ListRef(pub Blob);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvidenceRef {
    pub code: u8,
    pub reason: u64,
    pub first: i64,
    pub last: i64,
    pub sample: Option<i64>,
    pub missing: Option<u64>,
}
impl sealed::Sealed for Blob {}
impl Native for Blob {
    const TYPE: Type = Type::Bytes;
    fn decode(b: &[u8]) -> Self {
        Self::decode(b)
    }
}
impl sealed::Sealed for TextRef {}
impl Native for TextRef {
    const TYPE: Type = Type::Text;
    fn decode(b: &[u8]) -> Self {
        Self {
            id: u32::from_le_bytes(b[..4].try_into().unwrap()),
            text: Blob::decode(&b[5..]),
        }
    }
}
impl sealed::Sealed for U32ListRef {}
impl Native for U32ListRef {
    const TYPE: Type = Type::U32List;
    fn decode(b: &[u8]) -> Self {
        Self(Blob::decode(b))
    }
}
impl sealed::Sealed for U64ListRef {}
impl Native for U64ListRef {
    const TYPE: Type = Type::U64List;
    fn decode(b: &[u8]) -> Self {
        Self(Blob::decode(b))
    }
}
impl sealed::Sealed for EvidenceRef {}
impl Native for EvidenceRef {
    const TYPE: Type = Type::Evidence;
    fn decode(b: &[u8]) -> Self {
        Self {
            code: b[0],
            reason: u64::from_le_bytes(b[1..9].try_into().unwrap()),
            first: i64::from_le_bytes(b[9..17].try_into().unwrap()),
            last: i64::from_le_bytes(b[17..25].try_into().unwrap()),
            sample: (b[25] != 0).then(|| i64::from_le_bytes(b[26..34].try_into().unwrap())),
            missing: (b[34] != 0).then(|| u64::from_le_bytes(b[35..43].try_into().unwrap())),
        }
    }
}
