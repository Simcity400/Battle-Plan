pub use replay_data::codec::{Decoder, invalid_static as invalid, put_count64, put_fixed};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

pub trait Product: Sized {
    const MIN_BYTES: usize;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()>;
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self>;
    fn schema(schema: &mut Schema);
    fn encode_slice(values: &[Self], out: &mut Vec<u8>) -> io::Result<()> {
        for value in values {
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode_many(input: &mut Decoder<'_>, count: usize) -> io::Result<Vec<Self>> {
        if Self::MIN_BYTES == 0 || count > input.remaining() / Self::MIN_BYTES {
            return Err(invalid("product items exceed remaining payload"));
        }
        let mut values = engine_allocation::vec(count)?;
        for _ in 0..count {
            values.push(Self::decode(input)?);
        }
        Ok(values)
    }
}

pub struct Schema(Sha256);
impl Schema {
    pub fn field(&mut self, name: &str) {
        self.0.update((name.len() as u64).to_le_bytes());
        self.0.update(name.as_bytes());
    }
    pub fn tag(&mut self, tag: u8) {
        self.0.update([tag]);
    }
}

pub fn schema_digest<T: Product>() -> [u8; 32] {
    let mut schema = Schema(Sha256::new());
    schema.field("product-codec/le/u64-counts/float-bits/u8-tags/v1");
    T::schema(&mut schema);
    schema.0.finalize().into()
}

pub fn encode<T: Product>(value: &T) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    value.encode(&mut out)?;
    Ok(out)
}

pub fn decode<T: Product>(bytes: &[u8]) -> io::Result<T> {
    let mut input = Decoder::new(bytes);
    let value = T::decode(&mut input)?;
    input.finish()?;
    Ok(value)
}

macro_rules! number {
    ($($ty:ty => $read:ident),* $(,)?) => {$(
        impl Product for $ty {
            const MIN_BYTES: usize = size_of::<Self>();
            fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
                put_fixed(out, &self.to_le_bytes())
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> { input.$read() }
            fn schema(schema: &mut Schema) { schema.field(stringify!($ty)); }
        }
    )*};
}
number!(u16 => u16, u32 => u32, u64 => u64, u128 => u128,
        i8 => i8, i16 => i16, i32 => i32, i64 => i64, i128 => i128);

impl Product for u8 {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_fixed(out, &[*self])
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        input.u8()
    }
    fn schema(schema: &mut Schema) {
        schema.field("u8");
    }
    fn encode_slice(values: &[Self], out: &mut Vec<u8>) -> io::Result<()> {
        put_fixed(out, values)
    }
    fn decode_many(input: &mut Decoder<'_>, count: usize) -> io::Result<Vec<Self>> {
        engine_allocation::copy_vec(input.take(count)?)
    }
}

macro_rules! float {
    ($ty:ty, $bits:ty) => {
        impl Product for $ty {
            const MIN_BYTES: usize = size_of::<Self>();
            fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
                self.to_bits().encode(out)
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                Ok(Self::from_bits(<$bits>::decode(input)?))
            }
            fn schema(schema: &mut Schema) {
                schema.field(stringify!($ty));
            }
        }
    };
}
float!(f32, u32);
float!(f64, u64);

impl Product for usize {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_count64(out, *self)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::try_from(input.u64()?).map_err(|_| invalid("product usize exceeds address space"))
    }
    fn schema(schema: &mut Schema) {
        schema.field("usize/u64");
    }
}
impl Product for bool {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        u8::from(*self).encode(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        input.bool()
    }
    fn schema(schema: &mut Schema) {
        schema.field("bool");
    }
}
impl Product for String {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        replay_data::codec::put_bytes64(out, self.as_bytes())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        input.string64()
    }
    fn schema(schema: &mut Schema) {
        schema.field("String/utf8");
    }
}
impl<T: Product> Product for Option<T> {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.is_some().encode(out)?;
        if let Some(value) = self {
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.bool()? {
            Ok(Some(T::decode(input)?))
        } else {
            Ok(None)
        }
    }
    fn schema(schema: &mut Schema) {
        schema.field("Option");
        T::schema(schema);
    }
}
impl<T: Product> Product for Vec<T> {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_count64(out, self.len())?;
        T::encode_slice(self, out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let count = input.count64(T::MIN_BYTES)?;
        T::decode_many(input, count)
    }
    fn schema(schema: &mut Schema) {
        schema.field("Vec");
        T::schema(schema);
    }
}
impl<K: Product + Ord, V: Product> Product for BTreeMap<K, V> {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_count64(out, self.len())?;
        for (key, value) in self {
            key.encode(out)?;
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let minimum = K::MIN_BYTES
            .checked_add(V::MIN_BYTES)
            .ok_or_else(|| invalid("product map minimum overflows"))?;
        let count = input.count64(minimum)?;
        let mut out = BTreeMap::new();
        for _ in 0..count {
            let key = K::decode(input)?;
            if out
                .last_key_value()
                .is_some_and(|(previous, _)| previous >= &key)
            {
                return Err(invalid("product map keys are not strictly ordered"));
            }
            out.insert(key, V::decode(input)?);
        }
        Ok(out)
    }
    fn schema(schema: &mut Schema) {
        schema.field("BTreeMap");
        K::schema(schema);
        V::schema(schema);
    }
}
impl<T: Product + Ord> Product for BTreeSet<T> {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_count64(out, self.len())?;
        for value in self {
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let count = input.count64(T::MIN_BYTES)?;
        let mut values = BTreeSet::new();
        for _ in 0..count {
            let value = T::decode(input)?;
            if values.last().is_some_and(|last| *last >= value) {
                return Err(invalid("product set members are not strictly ordered"));
            }
            values.insert(value);
        }
        Ok(values)
    }
    fn schema(schema: &mut Schema) {
        schema.field("BTreeSet");
        T::schema(schema);
    }
}
impl<T: Product> Product for Arc<T> {
    const MIN_BYTES: usize = T::MIN_BYTES;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (**self).encode(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Arc::new(T::decode(input)?))
    }
    fn schema(schema: &mut Schema) {
        schema.field("Arc");
        T::schema(schema);
    }
}
impl<T: Product> Product for Arc<[T]> {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        put_count64(out, self.len())?;
        T::encode_slice(self, out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Vec::<T>::decode(input)?.into())
    }
    fn schema(schema: &mut Schema) {
        schema.field("Arc/ slice");
        T::schema(schema);
    }
}
impl<T: Product, const N: usize> Product for [T; N] {
    const MIN_BYTES: usize = if N == 0 {
        1
    } else {
        T::MIN_BYTES.saturating_mul(N)
    };
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if N == 0 {
            return put_fixed(out, &[0]);
        }
        T::encode_slice(self, out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if N == 0 {
            if input.u8()? != 0 {
                return Err(invalid("unknown empty array marker"));
            }
            return Vec::<T>::new()
                .try_into()
                .map_err(|_| invalid("product array length differs"));
        }
        if T::MIN_BYTES == 0 || N > input.remaining() / T::MIN_BYTES {
            return Err(invalid("product array exceeds remaining payload"));
        }
        T::decode_many(input, N)?
            .try_into()
            .map_err(|_| invalid("product array length differs"))
    }
    fn schema(schema: &mut Schema) {
        schema.field("array");
        schema.field(&N.to_string());
        T::schema(schema);
    }
}

impl Product for () {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        0_u8.encode(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.u8()? != 0 {
            return Err(invalid("unknown unit product marker"));
        }
        Ok(())
    }
    fn schema(schema: &mut Schema) {
        schema.field("unit");
    }
}
impl<T: Product> Product for Box<T> {
    const MIN_BYTES: usize = T::MIN_BYTES;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (**self).encode(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Box::new(T::decode(input)?))
    }
    fn schema(schema: &mut Schema) {
        schema.field("Box");
        T::schema(schema);
    }
}
impl<T: Product, E: Product> Product for Result<T, E> {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Ok(value) => {
                0_u8.encode(out)?;
                value.encode(out)
            }
            Err(value) => {
                1_u8.encode(out)?;
                value.encode(out)
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Ok(T::decode(input)?)),
            1 => Ok(Err(E::decode(input)?)),
            _ => Err(invalid("unknown result product tag")),
        }
    }
    fn schema(schema: &mut Schema) {
        schema.field("Result");
        T::schema(schema);
        E::schema(schema);
    }
}
macro_rules! tuples {
    ($(($($name:ident),+)),+ $(,)?) => { $(
        impl<$($name: Product),+> Product for ($($name,)+) {
            const MIN_BYTES: usize = 0 $(+ $name::MIN_BYTES)+;
            #[expect(non_snake_case, reason = "the tuple's elements are bound by their type parameter names")]
            fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
                let ($($name,)+) = self; $($name.encode(out)?;)+ Ok(())
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> { Ok(($($name::decode(input)?,)+)) }
            fn schema(schema: &mut Schema) { schema.field("tuple"); $($name::schema(schema);)+ }
        }
    )+ };
}
tuples!((A, B), (A, B, C), (A, B, C, D), (A, B, C, D, E));

#[macro_export]
macro_rules! product {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($fv:vis $field:ident: $ty:ty),* $(,)? } $($rest:tt)*) => {
        $(#[$meta])* $vis struct $name { $($fv $field: $ty),* }
        $crate::product! { impl $name { $($field: $ty),* } default {} }
        $crate::product! { $($rest)* }
    };
    (impl $name:ty { $($field:ident: $ty:ty),* $(,)? } default { $($skip:ident: $value:expr),* $(,)? } $($rest:tt)*) => {
        impl $crate::Product for $name {
            const MIN_BYTES: usize = {
                let minimum = 0usize $(+ <$ty as $crate::Product>::MIN_BYTES)*;
                if minimum == 0 { 1 } else { minimum }
            };
            fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
                if 0usize $(+ <$ty as $crate::Product>::MIN_BYTES)* == 0 { $crate::put_fixed(out, &[0])?; }
                $($crate::Product::encode(&self.$field, out)?;)* Ok(())
            }
            fn decode(input: &mut $crate::Decoder<'_>) -> std::io::Result<Self> {
                if 0usize $(+ <$ty as $crate::Product>::MIN_BYTES)* == 0 && input.u8()? != 0 {
                    return Err($crate::invalid("unknown empty product marker"));
                }
                Ok(Self { $($field: <$ty as $crate::Product>::decode(input)?,)* $($skip: $value),* })
            }
            fn schema(schema: &mut $crate::Schema) {
                schema.field("struct"); schema.field(stringify!($name));
                $(schema.field(stringify!($field)); schema.field(stringify!($ty)); <$ty as $crate::Product>::schema(schema);)*
                schema.field("end");
            }
        }
        $crate::product! { $($rest)* }
    };
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($tag:literal => $variant:ident { $($field:ident: $ty:ty),* $(,)? }),* $(,)? } $($rest:tt)*) => {
        $(#[$meta])* $vis enum $name { $($variant { $($field: $ty),* }),* }
        impl $crate::Product for $name {
            const MIN_BYTES: usize = {
                let tags: &[u8] = &[$($tag),*];
                let mut i = 0;
                while i < tags.len() {
                    let mut j = i + 1;
                    while j < tags.len() { assert!(tags[i] != tags[j], "duplicate product enum tag"); j += 1; }
                    i += 1;
                }
                1
            };
            fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
                match self { $(Self::$variant { $($field),* } => {
                    $crate::Product::encode(&($tag as u8), out)?;
                    $($crate::Product::encode($field, out)?;)* Ok(())
                }),* }
            }
            fn decode(input: &mut $crate::Decoder<'_>) -> std::io::Result<Self> {
                match input.u8()? {
                    $($tag => Ok(Self::$variant { $($field: <$ty as $crate::Product>::decode(input)?),* }),)*
                    _ => Err($crate::invalid("unknown product enum tag")),
                }
            }
            fn schema(schema: &mut $crate::Schema) {
                schema.field("enum"); schema.field(stringify!($name));
                $(schema.tag($tag); schema.field(stringify!($variant));
                    $(schema.field(stringify!($field)); schema.field(stringify!($ty)); <$ty as $crate::Product>::schema(schema);)*
                    schema.field("end-variant");)*
                schema.field("end");
            }
        }
        $crate::product! { $($rest)* }
    };
    () => {};
}

#[cfg(test)]
mod tests;
