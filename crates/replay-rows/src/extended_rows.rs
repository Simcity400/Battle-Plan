use super::*;
use crate::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NameOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub programmatic: u8,
    pub custom: Vec<u8>,
    pub group: Vec<u8>,
}
value_struct!(NameOperands { kind: i32, order: i32, ordered: u8, programmatic: u8, custom: Vec<u8>, group: Vec<u8> });
opaque_value!(Option<NameOperands>);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionRise {
    pub hour: i32,
    pub count: u32,
}
value_struct!(ConstructionRise { hour: i32, count: u32 });
impl OpaqueValue for ConstructionRise {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandEquipmentLoss {
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(LandEquipmentLoss {
    variant: u64,
    amount_raw: i64
});
impl OpaqueValue for LandEquipmentLoss {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OutputPurpose {
    #[default]
    Unknown = 0,
    New = 1,
    Converted = 2,
    Refit = 3,
}

impl OutputPurpose {
    pub const fn raw(self) -> u8 {
        self as u8
    }

    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::Unknown,
            1 => Self::New,
            2 => Self::Converted,
            3 => Self::Refit,
            _ => return None,
        })
    }
}

#[path = "extended_rows/packet_hour.rs"]
mod packet_hour;
pub use packet_hour::*;

opaque_value!(Option<Vec<u8>>);
opaque_value!(Option<Vec<u32>>);
opaque_value!(Option<std::sync::Arc<Vec<u32>>>);
opaque_value!(Option<Vec<Vec<u32>>>);
opaque_value!(Option<Vec<u64>>);
opaque_value!(Vec<i64>);
opaque_value!(Vec<i32>);
opaque_value!(Option<Option<i32>>);
opaque_value!(Option<i16>);

opaque_value!(Option<bool>);
opaque_value!(Option<[i64; 7]>);
opaque_value!(Option<Vec<Option<i32>>>);
opaque_value!(Option<[Option<bool>; 2]>);
opaque_value!(std::sync::Arc<Vec<u64>>);
opaque_value!(Option<[i64; 2]>);
opaque_value!(Option<Option<i64>>);
opaque_value!(Option<Vec<Option<i64>>>);
opaque_value!(Option<[u64; 2]>);
opaque_value!(Option<Vec<i16>>);
opaque_value!(Option<[Option<i64>; 5]>);
opaque_value!(Option<[u64; 3]>);
opaque_value!([u8; 3]);
opaque_value!([u8; 2]);
opaque_value!([Vec<u8>; 2]);
opaque_value!(Option<[i64; 3]>);
opaque_value!([i64; 2]);
opaque_value!([i64; 3]);
opaque_value!([i64; 4]);
opaque_value!([i64; 5]);
opaque_value!(RetiredKept<[i64; 2]>);
opaque_value!(RetiredKept<[i64; 4]>);
opaque_value!(RetiredKept<[i64; 5]>);
opaque_value!([i64; 6]);
opaque_value!([[i64; 6]; 6]);
opaque_value!([u32; 2]);
opaque_value!([u32; 4]);
opaque_value!(Option<[i64; 5]>);
opaque_value!(Option<[Option<i64>; 10]>);
opaque_value!(Option<[u8; 2]>);
opaque_value!(Option<[Option<i64>; 4]>);
opaque_value!(Vec<Vec<u32>>);
opaque_value!(Option<[u8; 3]>);
opaque_value!([i64; 7]);
opaque_value!([u8; 4]);
opaque_value!(Vec<[i32; 2]>);
opaque_value!(Option<Option<u32>>);
opaque_value!(Option<Vec<Vec<u8>>>);
opaque_value!(Option<std::sync::Arc<[Vec<u64>; 5]>>);
opaque_value!(Option<u8>);
opaque_value!(Option<[Option<i64>; 2]>);
opaque_value!(Option<[Option<i64>; 1]>);

#[cfg(any(test, feature = "test-support"))]
pub fn round_trip<T: RowCodec + std::fmt::Debug + PartialEq>(row: &T) -> usize {
    let mut bytes = Vec::new();
    encode_row(row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(&decode_row::<T>(0, &mut input).unwrap(), row);
    input.finish().unwrap();
    bytes.len()
}
