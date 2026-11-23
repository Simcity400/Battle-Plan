use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::*,
    value::Value,
};
use engine_allocation as allocation;
use std::io;

pub trait CheckpointValue: Sized {
    const MINIMUM_BYTES: usize;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self>;
}

impl<T: CheckpointValue> CheckpointValue for std::sync::Arc<T> {
    const MINIMUM_BYTES: usize = T::MINIMUM_BYTES;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        CheckpointValue::encode(&**self, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        T::decode(input).map(Self::new)
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! wire_value {
    ($($ty:ty),+ $(,)?) => {
        $(impl $crate::checkpoint::values::CheckpointValue for $ty {
            const MINIMUM_BYTES: usize = <$ty as $crate::value::Value>::MINIMUM_ENCODED_BYTES;
            fn encode(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $crate::value::Value::encode(self, output)
            }
            fn decode(input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                <$ty as $crate::value::Value>::decode(input)
            }
        })+
    };
}
pub use crate::wire_value;

wire_value!(
    u8,
    i8,
    u16,
    i16,
    u32,
    i32,
    u64,
    i64,
    bool,
    PowerCurrency,
    PowerOperation,
    PowerCause,
    PowerSite,
    PowerObjectKind,
    PowerOriginKind,
    PowerDailyCostKind,
    GovernmentCause,
    PowerAction,
    Retired,
    PacketHour,
    StrategicAction,
);

impl<T: Value + Default> CheckpointValue for RetiredNested<T> {
    const MINIMUM_BYTES: usize = 0;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(self, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Value::decode(input)
    }
}

impl<T: Value + Default> CheckpointValue for RetiredKept<T> {
    const MINIMUM_BYTES: usize = 0;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(self, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Value::decode(input)
    }
}

impl<T: CheckpointValue> CheckpointValue for Recorded<T> {
    const MINIMUM_BYTES: usize = 1;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(&matches!(self, Self::Value(_)), output)?;
        if let Self::Value(value) = self {
            CheckpointValue::encode(value, output)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.bool()? {
            T::decode(input).map(Self::Value)
        } else {
            Ok(Self::NotRecorded)
        }
    }
}

impl<T: CheckpointValue> CheckpointValue for Option<T> {
    const MINIMUM_BYTES: usize = 1;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(&self.is_some(), output)?;
        if let Some(value) = self {
            CheckpointValue::encode(value, output)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.bool()? {
            T::decode(input).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl<T: CheckpointValue> CheckpointValue for Vec<T> {
    const MINIMUM_BYTES: usize = 4;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let count =
            u32::try_from(self.len()).map_err(|_| invalid("checkpoint collection exceeds u32"))?;
        if count != 0 && T::MINIMUM_BYTES == 0 {
            return Err(invalid("checkpoint collection has zero-width elements"));
        }
        Value::encode(&count, output)?;
        for value in self {
            CheckpointValue::encode(value, output)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let count = input.u32()? as usize;
        if count != 0 && (T::MINIMUM_BYTES == 0 || count > input.remaining() / T::MINIMUM_BYTES) {
            return Err(invalid("checkpoint collection exceeds remaining payload"));
        }
        let mut values = allocation::vec(count)?;
        for _ in 0..count {
            values.push(T::decode(input)?);
        }
        Ok(values)
    }
}

impl<T: CheckpointValue, const N: usize> CheckpointValue for [T; N] {
    const MINIMUM_BYTES: usize = T::MINIMUM_BYTES.saturating_mul(N);
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        for value in self {
            CheckpointValue::encode(value, output)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if Self::MINIMUM_BYTES > input.remaining() {
            return Err(invalid("checkpoint array exceeds remaining payload"));
        }
        let mut values = std::array::from_fn::<_, N, _>(|_| None);
        for value in &mut values {
            *value = Some(T::decode(input)?);
        }
        Ok(values.map(|value| value.expect("all checkpoint array elements initialized")))
    }
}

impl<T: CheckpointValue> CheckpointValue for Witnessed<T> {
    const MINIMUM_BYTES: usize = 1;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        CheckpointValue::encode(&self.0, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        CheckpointValue::decode(input).map(Self)
    }
}


#[macro_export]
#[doc(hidden)]
macro_rules! structure {
    ($ty:ty { $($field:ident: $field_ty:ty),* $(,)? }) => {
        impl $crate::checkpoint::values::CheckpointValue for $ty {
            const MINIMUM_BYTES: usize = 0_usize
                $(.saturating_add(<$field_ty as $crate::checkpoint::values::CheckpointValue>::MINIMUM_BYTES))*;
            fn encode(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $(<$field_ty as $crate::checkpoint::values::CheckpointValue>::encode(&self.$field, output)?;)*
                Ok(())
            }
            fn decode(input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                Ok(Self { $($field: <$field_ty as $crate::checkpoint::values::CheckpointValue>::decode(input)?,)* })
            }
        }
    };
}
pub use crate::structure;
