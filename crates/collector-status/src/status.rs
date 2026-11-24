use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::fmt;

#[derive(Clone)]
pub struct Empty;
#[derive(Clone)]
pub struct Field<V, T> {
    pub key: &'static str,
    pub value: V,
    pub next: T,
}
pub trait Fields {
    fn field_count(&self) -> usize;
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error>;
}
impl Fields for Empty {
    fn field_count(&self) -> usize {
        0
    }
    fn serialize<M: SerializeMap>(&self, _: &mut M) -> Result<(), M::Error> {
        Ok(())
    }
}
impl<V: Serialize, T: Fields> Fields for Field<V, T> {
    fn field_count(&self) -> usize {
        1 + self.next.field_count()
    }
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry(self.key, &self.value)?;
        self.next.serialize(map)
    }
}

#[derive(Clone)]
pub struct Object<T>(pub T);
#[derive(Serialize)]
#[serde(untagged)]
pub enum Choice<T, U> {
    First(T),
    Second(U),
}
impl<T: Fields> Serialize for Object<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.field_count()))?;
        self.0.serialize(&mut map)?;
        map.end()
    }
}

#[derive(Clone)]
pub struct Joined<T, U>(T, U);
impl<T: Fields, U: Fields> Fields for Joined<T, U> {
    fn field_count(&self) -> usize {
        self.0.field_count() + self.1.field_count()
    }
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        self.0.serialize(map)?;
        self.1.serialize(map)
    }
}
impl<T: Fields> Object<T> {
    pub fn join<U: Fields>(self, other: Object<U>) -> Object<Joined<T, U>> {
        Object(Joined(self.0, other.0))
    }
}

pub struct Sequence<I>(I);
pub fn sequence<I>(iter: I) -> Sequence<I> {
    Sequence(iter)
}

impl<I> Serialize for Sequence<I>
where
    I: Clone + Iterator,
    I::Item: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for item in self.0.clone() {
            seq.serialize_element(&item)?;
        }
        seq.end()
    }
}

pub struct Map<I>(I);
pub fn map<I>(iter: I) -> Map<I> {
    Map(iter)
}
impl<I, K, V> Serialize for Map<I>
where
    I: Clone + Iterator<Item = (K, V)>,
    K: Serialize,
    V: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (key, value) in self.0.clone() {
            map.serialize_entry(&key, &value)?;
        }
        map.end()
    }
}

#[derive(Clone)]
pub struct DebugValue<T>(pub T);

pub struct LossyBytes<'a>(pub &'a [u8]);
impl fmt::Display for LossyBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0;
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(text) => return formatter.write_str(text),
                Err(error) => {
                    formatter.write_str(unsafe {
                        std::str::from_utf8_unchecked(&bytes[..error.valid_up_to()])
                    })?;
                    formatter.write_str("\u{fffd}")?;
                    bytes = error
                        .error_len()
                        .map_or(&[], |len| &bytes[error.valid_up_to() + len..]);
                }
            }
        }
        Ok(())
    }
}
impl Serialize for LossyBytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<T: fmt::Debug> fmt::Display for DebugValue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl<T: fmt::Debug> Serialize for DebugValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[macro_export]
macro_rules! status_fields {
    () => { $crate::status::Empty };
    ($key:literal: { $($nested:tt)* }, $($rest:tt)*) => {
        $crate::status::Field { key: $key,
            value: $crate::status::status!({ $($nested)* }),
            next: $crate::status::status_fields!($($rest)*) }
    };
    ($key:literal: { $($nested:tt)* } $(,)?) => {
        $crate::status::Field { key: $key,
            value: $crate::status::status!({ $($nested)* }),
            next: $crate::status::Empty }
    };
    ($key:literal: $value:expr, $($rest:tt)*) => {
        $crate::status::Field { key: $key, value: $value,
            next: $crate::status::status_fields!($($rest)*) }
    };
    ($key:literal: $value:expr $(,)?) => {
        $crate::status::Field { key: $key, value: $value, next: $crate::status::Empty }
    };
}
#[macro_export]
macro_rules! status {
    ({ $($fields:tt)* }) => { $crate::status::Object($crate::status::status_fields!($($fields)*)) };
}
pub use {status, status_fields};

#[derive(Clone)]
pub struct DisplayValue<T>(pub T);
impl<T: std::fmt::Display> serde::Serialize for DisplayValue<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}
pub fn serialize_into(bytes: &mut Vec<u8>, value: &impl Serialize) -> std::io::Result<()> {
    use std::io::{self, Write};
    struct Writer<'a> {
        bytes: &'a mut Vec<u8>,
        failed: bool,
    }
    impl Write for Writer<'_> {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if !self.failed {
                if engine_allocation::reserve(self.bytes, input.len()).is_err() {
                    self.failed = true;
                } else {
                    self.bytes.extend_from_slice(input);
                }
            }
            Ok(input.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    bytes.clear();
    let mut writer = Writer {
        bytes,
        failed: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.failed {
        writer.bytes.clear();
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    result.map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}
pub fn serialize(value: &impl Serialize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    serialize_into(&mut bytes, value)?;
    Ok(bytes)
}

