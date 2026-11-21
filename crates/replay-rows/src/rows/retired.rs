use super::*;
use crate::rows::compatibility::RETIRED_INPUTS_FORMAT;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Retired;

impl Value for Retired {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(0)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self)
    }
    fn encode_reserved(&self, _output: &mut Vec<u8>) -> io::Result<()> {
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            input.take(input.remaining())?;
        }
        Ok(Self)
    }
}
opaque_value!(Retired);

#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RetiredNested<T>(pub std::marker::PhantomData<fn() -> T>);

impl<T> Default for RetiredNested<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}
impl<T> Clone for RetiredNested<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for RetiredNested<T> {}
impl<T> PartialEq for RetiredNested<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl<T> Eq for RetiredNested<T> {}
impl<T> std::fmt::Debug for RetiredNested<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RetiredNested")
    }
}

fn historical_writer() -> bool {
    historical_stamp().is_some()
}

impl<T: Value + Default> Value for RetiredNested<T> {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        if historical_writer() {
            T::default().encoded_bytes()
        } else {
            Ok(0)
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        if historical_writer() {
            T::default().encode_reserved(output)
        } else {
            Ok(())
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            T::decode(input)?;
        }
        Ok(Self::default())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RetiredKept<T>(pub Option<T>);

impl<T> Default for RetiredKept<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> RetiredKept<T> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn older_value(value: T) -> Self {
        Self(Some(value))
    }
    pub fn older(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T: Value + Default> Value for RetiredKept<T> {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.0)
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        if historical_writer() {
            match &self.0 {
                Some(value) => value.encoded_bytes(),
                None => T::default().encoded_bytes(),
            }
        } else {
            Ok(0)
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.0.as_ref().map(Value::try_clone).transpose().map(Self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        if historical_writer() {
            match &self.0 {
                Some(value) => value.encode_reserved(output),
                None => T::default().encode_reserved(output),
            }
        } else {
            Ok(())
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            T::decode(input).map(|value| Self(Some(value)))
        } else {
            Ok(Self(None))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RetiredI64(pub i64);

#[cfg(any(test, feature = "test-support"))]
impl RetiredI64 {
    pub const fn holding(value: i64) -> Self {
        Self(value)
    }
}

impl NativeValue for RetiredI64 {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn field(&self) -> Field<'_> {
        Field::I64(self.0)
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::I64(value) => Ok(Self(value)),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

macro_rules! retired_fixed {
    ($(#[$doc:meta])* $name:ident($ty:ty), $kind:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
        pub struct $name(pub $ty);

        #[cfg(any(test, feature = "test-support"))]
        impl $name {
            pub const fn holding(value: $ty) -> Self {
                Self(value)
            }
        }

        impl NativeValue for $name {
            fn heap_bytes(&self) -> usize {
                0
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(*self)
            }
            fn field(&self) -> Field<'_> {
                Field::$kind(self.0)
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => Ok(Self(value)),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
retired_fixed!(
    RetiredI32(i32),
    I32
);
retired_fixed!(
    RetiredU8(u8),
    U8
);
retired_fixed!(
    RetiredBool(bool),
    Bool
);
retired_fixed!(
    RetiredU32(u32),
    U32
);
retired_fixed!(
    RetiredU64(u64),
    U64
);
retired_fixed!(
    RetiredOptionalI64(Option<i64>),
    OptionalI64
);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RetiredU32List(pub Vec<u32>);

#[cfg(any(test, feature = "test-support"))]
impl RetiredU32List {
    pub fn holding(value: Vec<u32>) -> Self {
        Self(value)
    }
}

impl NativeValue for RetiredU32List {
    fn heap_bytes(&self) -> usize {
        self.0.capacity() * size_of::<u32>()
    }
    fn try_clone(&self) -> io::Result<Self> {
        allocation::copy_vec(&self.0).map(Self)
    }
    fn field(&self) -> Field<'_> {
        Field::U32List(List::Native(&self.0))
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U32List(value) => value.to_vec().map(Self),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

