//! Explicit native values inside a nested field, without row or delta framing.
//!
//! Collections carry a u32 element count, options carry a strict 0/1 tag, and
//! fixed arrays carry neither. Struct fields are written in declaration order.
//! No Rust padding, addresses or intermediate encoded buffers are involved.

use std::io;

use crate::codec::{Decoder, invalid_static as invalid};
use engine::allocation;

pub(crate) trait Value: Sized {
    /// Lower bound for every encoding of this type, including absent/empty values.
    const MINIMUM_ENCODED_BYTES: usize;
    #[cfg(test)]
    const STRUCT_FIELDS: usize = 0;

    fn encoded_bytes(&self) -> io::Result<usize>;
    fn recorded_bytes(&self) -> io::Result<usize> {
        Value::encoded_bytes(self)
    }
    fn encoded_many_bytes(values: &[Self]) -> io::Result<usize> {
        values.iter().try_fold(0, |size, value| {
            allocation::add(size, Value::encoded_bytes(value)?)
        })
    }
    fn try_clone(&self) -> io::Result<Self>;
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, Value::encoded_bytes(self)?)?;
        self.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self>;
    fn not_recorded() -> io::Result<Self> {
        Err(invalid("uncatalogued absent nested field"))
    }
    fn visit_field_gaps(&self, _visit: &mut dyn FnMut(&'static str)) {}

    /// The default handles structured elements; byte values override it to copy once.
    fn encode_many(values: &[Self], output: &mut Vec<u8>) -> io::Result<()> {
        for value in values {
            value.encode_reserved(output)?;
        }
        Ok(())
    }

    /// Validate the declared count before allocating the final owning collection.
    fn decode_many(count: usize, input: &mut Decoder<'_>) -> io::Result<Vec<Self>> {
        check_count(count, Self::MINIMUM_ENCODED_BYTES, input.remaining())?;
        let mut values = allocation::vec(count)?;
        for _ in 0..count {
            values.push(Self::decode(input)?);
        }
        Ok(values)
    }
}

impl<T: Value> Value for crate::rows::Recorded<T> {
    fn recorded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Value(value) => Value::recorded_bytes(value),
            Self::NotRecorded => Ok(0),
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = T::MINIMUM_ENCODED_BYTES;
    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Value(value) => Value::encoded_bytes(value),
            Self::NotRecorded => Err(invalid("nested field not recorded")),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Value(value) => value.try_clone().map(Self::Value),
            Self::NotRecorded => Ok(Self::NotRecorded),
        }
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Value(value) => value.encode_reserved(output),
            Self::NotRecorded => Err(invalid("nested field not recorded")),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        T::decode(input).map(Self::Value)
    }
    fn not_recorded() -> io::Result<Self> {
        Ok(Self::NotRecorded)
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        if let Self::Value(value) = self {
            value.visit_field_gaps(visit);
        }
    }
}

/// Object-safe adapter for a borrowed native complex field. Its caller owns any
/// outer byte-length prefix and can write/fill that prefix around this direct append.
pub(crate) trait EncodeValue {
    fn encoded_bytes(&self) -> io::Result<usize>;
    fn recorded_bytes(&self) -> io::Result<usize>;
    fn encode_value(&self, output: &mut Vec<u8>) -> io::Result<()>;
}

impl<T: Value> EncodeValue for T {
    fn encoded_bytes(&self) -> io::Result<usize> {
        Value::encoded_bytes(self)
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        Value::recorded_bytes(self)
    }
    fn encode_value(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.encode(output)
    }
}

fn check_count(count: usize, minimum_item_bytes: usize, remaining: usize) -> io::Result<()> {
    if count != 0 && (minimum_item_bytes == 0 || count > remaining / minimum_item_bytes) {
        return Err(invalid(
            "nested collection length exceeds remaining payload",
        ));
    }
    Ok(())
}

macro_rules! integer {
    ($($ty:ty),+ $(,)?) => {
        $(impl Value for $ty {
            const MINIMUM_ENCODED_BYTES: usize = size_of::<Self>();

            fn encoded_bytes(&self) -> io::Result<usize> { Ok(size_of::<Self>()) }
            fn encoded_many_bytes(values: &[Self]) -> io::Result<usize> { allocation::mul(values.len(), size_of::<Self>()) }
            fn try_clone(&self) -> io::Result<Self> { Ok(*self) }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                output.extend_from_slice(&self.to_le_bytes());
                Ok(())
            }

            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                Ok(Self::from_le_bytes(
                    input.take(size_of::<Self>())?.try_into().unwrap(),
                ))
            }
        })+
    };
}

integer!(i8, u16, i16, u32, i32, u64, i64);

impl Value for u8 {
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn encoded_many_bytes(values: &[Self]) -> io::Result<usize> {
        Ok(values.len())
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        output.push(*self);
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        input.u8()
    }

    fn encode_many(values: &[Self], output: &mut Vec<u8>) -> io::Result<()> {
        output.extend_from_slice(values);
        Ok(())
    }

    fn decode_many(count: usize, input: &mut Decoder<'_>) -> io::Result<Vec<Self>> {
        let bytes = input.take(count)?;
        let mut values = allocation::vec(count)?;
        values.extend_from_slice(bytes);
        Ok(values)
    }
}

impl Value for bool {
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn encoded_many_bytes(values: &[Self]) -> io::Result<usize> {
        Ok(values.len())
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        output.push(u8::from(*self));
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        input.bool()
    }
}

impl<T: Value> Value for Option<T> {
    fn recorded_bytes(&self) -> io::Result<usize> {
        allocation::add(1, self.as_ref().map_or(Ok(0), Value::recorded_bytes)?)
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        allocation::add(
            1,
            match self {
                Some(value) => Value::encoded_bytes(value)?,
                None => 0,
            },
        )
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.as_ref().map(Value::try_clone).transpose()
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        if let Some(value) = self {
            value.visit_field_gaps(visit);
        }
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            None => output.push(0),
            Some(value) => {
                output.push(1);
                value.encode_reserved(output)?;
            }
        }
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(None),
            1 => T::decode(input).map(Some),
            _ => Err(invalid("invalid nested option tag")),
        }
    }
}

impl<T: Value> Value for Vec<T> {
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.iter().try_fold(4, |size, value| {
            allocation::add(size, Value::recorded_bytes(value)?)
        })
    }
    const MINIMUM_ENCODED_BYTES: usize = 4;
    fn encoded_bytes(&self) -> io::Result<usize> {
        allocation::add(4, T::encoded_many_bytes(self)?)
    }
    fn try_clone(&self) -> io::Result<Self> {
        let mut values = allocation::vec(self.len())?;
        for value in self {
            values.push(Value::try_clone(value)?);
        }
        Ok(values)
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        for value in self {
            value.visit_field_gaps(visit);
        }
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let count = u32::try_from(self.len())
            .map_err(|_| invalid("nested collection count exceeds u32"))?;
        // A nonempty zero-width collection cannot be bounded by its payload.
        // Zero-length fixed arrays remain usable as fields and in empty vectors.
        if count != 0 && T::MINIMUM_ENCODED_BYTES == 0 {
            return Err(invalid(
                "nonempty nested collection has zero-width elements",
            ));
        }
        output.extend_from_slice(&count.to_le_bytes());
        T::encode_many(self, output)
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let count = input.u32()? as usize;
        T::decode_many(count, input)
    }
}

impl<T: Value, const N: usize> Value for [T; N] {
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.iter().try_fold(0, |size, value| {
            allocation::add(size, Value::recorded_bytes(value)?)
        })
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        for value in self {
            value.visit_field_gaps(visit);
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = T::MINIMUM_ENCODED_BYTES.saturating_mul(N);

    fn encoded_bytes(&self) -> io::Result<usize> {
        T::encoded_many_bytes(self)
    }
    fn try_clone(&self) -> io::Result<Self> {
        let mut values = std::array::from_fn::<_, N, _>(|_| None);
        for (target, value) in values.iter_mut().zip(self) {
            *target = Some(Value::try_clone(value)?);
        }
        Ok(values.map(|value| value.expect("all array elements cloned")))
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        T::encode_many(self, output)
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if Self::MINIMUM_ENCODED_BYTES > input.remaining() {
            return Err(invalid("nested array exceeds remaining payload"));
        }
        // Safe stack construction also drops initialized elements if a later
        // element fails. No temporary Vec, unsafe initialization or Copy bound.
        let mut values = std::array::from_fn::<_, N, _>(|_| None);
        for value in &mut values {
            *value = Some(T::decode(input)?);
        }
        Ok(values.map(|value| value.expect("all array elements initialized")))
    }
}

/// Nested values use their own catalog, independent of the parent's field count.
pub(crate) fn decode_appended<T: Value>(
    record: crate::rows::compatibility::RecordKind,
    name: &str,
    input: &mut Decoder<'_>,
) -> io::Result<T> {
    let field = record
        .last_change()
        .appended
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| invalid("uncatalogued nested field"))?;
    if input.format < field.introduced {
        T::not_recorded()
    } else {
        T::decode(input)
    }
}

/// Implement a declared struct's wire field order without encoding its layout.
/// Example: value_struct!(Entry { name: Vec<u8>, amount: i64 });
macro_rules! value_struct {
    ($ty:ty { $($field:ident : $field_ty:ty),* $(,)? } $(appended $record:ident { $($appended:ident : $appended_ty:ty),* $(,)? })?) => {
        impl $crate::value::Value for $ty {
            #[cfg(test)]
            const STRUCT_FIELDS: usize = [$(stringify!($field)),*].len()
                $(+ [$(stringify!($appended)),*].len())?;
            const MINIMUM_ENCODED_BYTES: usize = 0_usize
                $(.saturating_add(<$field_ty as $crate::value::Value>::MINIMUM_ENCODED_BYTES))*;

            fn recorded_bytes(&self) -> std::io::Result<usize> {
                let size = 0;
                $(let size = engine::allocation::add(size, <$field_ty as $crate::value::Value>::recorded_bytes(&self.$field)?)?;)*
                $($(let size = engine::allocation::add(size, <$appended_ty as $crate::value::Value>::recorded_bytes(&self.$appended)?)?;)*)?
                Ok(size)
            }
            fn encoded_bytes(&self) -> std::io::Result<usize> {
                let size = 0;
                $(let size = engine::allocation::add(size, <$field_ty as $crate::value::Value>::encoded_bytes(&self.$field)?)?;)*
                $($(let size = engine::allocation::add(size, <$appended_ty as $crate::value::Value>::encoded_bytes(&self.$appended)?)?;)*)?
                Ok(size)
            }
            fn try_clone(&self) -> std::io::Result<Self> {
                Ok(Self { $($field: <$field_ty as $crate::value::Value>::try_clone(&self.$field)?,)*
                    $($($appended: <$appended_ty as $crate::value::Value>::try_clone(&self.$appended)?,)*)? })
            }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $(<$field_ty as $crate::value::Value>::encode_reserved(&self.$field, output)?;)*
                $($(<$appended_ty as $crate::value::Value>::encode_reserved(&self.$appended, output)?;)*)?
                Ok(())
            }

            fn decode(input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                Ok(Self {
                    $($field: <$field_ty as $crate::value::Value>::decode(input)?,)*
                    $($($appended: $crate::value::decode_appended::<$appended_ty>(
                        $crate::rows::compatibility::RecordKind::$record,
                        concat!(stringify!($record), ".", stringify!($appended)), input,
                    )?,)*)?
                })
            }
            fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                $(<$field_ty as $crate::value::Value>::visit_field_gaps(&self.$field, visit);)*
                $($(if matches!(self.$appended, $crate::rows::Recorded::NotRecorded) {
                    visit(concat!(stringify!($record), ".", stringify!($appended)));
                } else {
                    <$appended_ty as $crate::value::Value>::visit_field_gaps(&self.$appended, visit);
                })*)?
            }
        }
    };
}

pub(crate) use value_struct;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Reading {
        name: Vec<u8>,
        flags: [bool; 2],
        raw: i64,
        samples: Vec<Option<[i16; 2]>>,
    }

    value_struct!(Reading {
        name: Vec<u8>,
        flags: [bool; 2],
        raw: i64,
        samples: Vec<Option<[i16; 2]>>,
    });

    #[test]
    fn primitive_values_have_exact_little_endian_bytes() {
        fn check<T: Value + std::fmt::Debug + PartialEq>(value: T, expected: &[u8]) {
            let mut bytes = Vec::new();
            value.encode(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
            assert_eq!(bytes.len(), T::MINIMUM_ENCODED_BYTES);
            let mut input = Decoder::new(&bytes);
            assert_eq!(T::decode(&mut input).unwrap(), value);
            input.finish().unwrap();
        }
        check(false, &[0]);
        check(true, &[1]);
        check(u8::MAX, &[0xff]);
        check(i8::MIN, &[0x80]);
        check(0xabcd_u16, &[0xcd, 0xab]);
        check(i16::MIN, &[0, 0x80]);
        check(0x1234_abcd_u32, &[0xcd, 0xab, 0x34, 0x12]);
        check(i32::MIN, &[0, 0, 0, 0x80]);
        check(u64::MAX, &[0xff; 8]);
        check(i64::MIN, &[0, 0, 0, 0, 0, 0, 0, 0x80]);
        check(i64::MAX, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
    }

    #[test]
    fn nested_structs_preserve_order_raw_bytes_and_optional_native_extrema() {
        let rows = vec![
            Reading {
                name: vec![0xff, 0, 0xc3, 0x28],
                flags: [true, false],
                raw: i64::MIN,
                samples: vec![None, Some([i16::MIN, i16::MAX]), None],
            },
            Reading {
                name: vec![0xff, 0, 0xc3, 0x28],
                flags: [false, true],
                raw: i64::MAX,
                samples: vec![Some([0, -1]), Some([0, -1])],
            },
        ];
        assert_eq!(Reading::MINIMUM_ENCODED_BYTES, 18);
        let mut encoded = vec![0xa5];
        let borrowed: &dyn EncodeValue = &rows;
        borrowed.encode_value(&mut encoded).unwrap();
        assert_eq!(&encoded[..9], &[0xa5, 2, 0, 0, 0, 4, 0, 0, 0]);
        let payload_end = encoded.len();
        encoded.push(0x5a);
        let mut input = Decoder::new(&encoded[1..]);
        assert_eq!(Vec::<Reading>::decode(&mut input).unwrap(), rows);
        assert_eq!(input.u8().unwrap(), 0x5a);
        input.finish().unwrap();
        for end in 1..payload_end {
            assert!(Vec::<Reading>::decode(&mut Decoder::new(&encoded[1..end])).is_err());
        }
    }

    #[test]
    fn hostile_counts_invalid_tags_and_partial_elements_are_rejected() {
        let excessive = u32::MAX.to_le_bytes();
        assert!(Vec::<u8>::decode(&mut Decoder::new(&excessive)).is_err());
        assert!(Vec::<Reading>::decode(&mut Decoder::new(&excessive)).is_err());
        assert!(
            Vec::<Vec<i64>>::decode(&mut Decoder::new(&[1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,]))
                .is_err()
        );
        assert!(
            Vec::<[i64; 2]>::decode(&mut Decoder::new(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,]))
                .is_err()
        );
        for tag in [2, 127, 255] {
            assert!(bool::decode(&mut Decoder::new(&[tag])).is_err());
            assert!(Option::<i64>::decode(&mut Decoder::new(&[tag])).is_err());
        }
        assert!(Option::<i64>::decode(&mut Decoder::new(&[1, 0, 0])).is_err());
        assert!(<[Option<i64>; 2]>::decode(&mut Decoder::new(&[0, 2])).is_err());

        // An absent option consumes its own tag, never an absent payload or the
        // following field. A present zero is observably different from absence.
        let mut input = Decoder::new(&[0, 0xff]);
        assert_eq!(Option::<u64>::decode(&mut input).unwrap(), None);
        assert_eq!(input.u8().unwrap(), 0xff);
        let mut bytes = Vec::new();
        Some(0_i64).encode(&mut bytes).unwrap();
        assert_eq!(bytes, [1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn empty_arrays_and_collections_remain_distinct_and_bounded() {
        let empty_array: [i64; 0] = [];
        let mut bytes = Vec::new();
        empty_array.encode(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(
            <[i64; 0]>::decode(&mut Decoder::new(&[])).unwrap(),
            [0_i64; 0]
        );

        Vec::<[u8; 0]>::new().encode(&mut bytes).unwrap();
        assert_eq!(bytes, [0; 4]);
        assert!(
            Vec::<[u8; 0]>::decode(&mut Decoder::new(&bytes))
                .unwrap()
                .is_empty()
        );
        assert!(vec![[0_u8; 0]].encode(&mut Vec::new()).is_err());
        assert!(Vec::<[u8; 0]>::decode(&mut Decoder::new(&[1, 0, 0, 0])).is_err());
    }
}
