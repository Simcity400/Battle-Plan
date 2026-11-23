use super::*;

pub trait Native: Sized {
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode(kind: Kind, input: &mut Decoder<'_>) -> io::Result<Self>;
}

#[macro_export]
#[doc(hidden)]
macro_rules! native {
    ($($ty:ty),+ $(,)?) => {
        $(impl $crate::rows::checkpoint::Native for $ty {
            fn encode(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                let field = $crate::rows::NativeValue::field(self);
                $crate::__allocation::reserve(output, field.encoded_bytes()?)?;
                field.encode(output)
            }
            fn decode(kind: $crate::rows::Kind, input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                $crate::rows::NativeValue::from_field(kind.decode(input)?)
            }
        })+
    };
}
pub use crate::native;

native!(
    u8,
    i8,
    u16,
    i16,
    u32,
    i32,
    u64,
    i64,
    bool,
    String,
    Option<i64>,
    Option<u32>,
    Option<i32>,
    Option<u64>,
    Vec<u8>,
    Vec<u32>,
    Vec<u64>,
    PowerCurrency,
    PowerOperation,
    PowerCause,
    PowerSite,
    PowerObjectKind,
    PowerOriginKind,
    PowerDailyCostKind,
    GovernmentCause,
    PowerAction,
    EquipmentDestructionCause,
    EquipmentDestructionCoverage,
    DivisionDeletionDisposition,
    Option<EquipmentDestructionCause>,
    BuildingBombingDamageCause,
    RetiredI64,
    RetiredI32,
    RetiredU8,
    RetiredBool,
    RetiredU32,
    RetiredU64,
    RetiredOptionalI64,
    RetiredU32List,
);

impl<T: Native> Native for Recorded<T> {
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(&matches!(self, Self::Value(_)), output)?;
        if let Self::Value(value) = self {
            Native::encode(value, output)?;
        }
        Ok(())
    }
    fn decode(kind: Kind, input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.bool()? {
            T::decode(kind, input).map(Self::Value)
        } else {
            Ok(Self::NotRecorded)
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! opaque {
    ($ty:ty) => {
        $crate::opaque!(@impl [] $ty);
    };
    (@impl [$($generic:tt)*] $ty:ty) => {
        impl $($generic)* $crate::rows::checkpoint::Native for $ty {
            fn encode(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $crate::checkpoint::values::CheckpointValue::encode(self, output)
            }
            fn decode(
                _kind: $crate::rows::Kind,
                input: &mut $crate::codec::Decoder<'_>,
            ) -> std::io::Result<Self> {
                $crate::checkpoint::values::CheckpointValue::decode(input)
            }
        }
    };
}
pub use crate::opaque;
