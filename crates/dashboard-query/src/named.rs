use serde::de::{
    self, Deserialize, DeserializeSeed, EnumAccess, Error as _, IntoDeserializer, MapAccess, SeqAccess, VariantAccess, Visitor,
    value::BorrowedStrDeserializer,
};
use serde_json::{Error, Value};

pub fn named<'a, T: Deserialize<'a>>(value: &'a Value) -> Result<T, Error> {
    T::deserialize(Named(value))
}

pub fn named_fields<'a, T: Deserialize<'a>>(value: &'a Value) -> Result<T, Error> {
    let Some(fields) = value.as_object() else { return named(value) };
    T::deserialize(de::value::MapAccessDeserializer::new(Entries { fields: fields.iter(), value: None, name_errors: true }))
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

#[derive(Clone, Copy)]
pub struct Named<'a>(pub &'a Value);

impl<'de> de::Deserializer<'de> for Named<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::Number(number) => match (number.as_u64(), number.as_i64(), number.as_f64()) {
                (Some(value), _, _) => visitor.visit_u64(value),
                (None, Some(value), _) => visitor.visit_i64(value),
                (None, None, Some(value)) => visitor.visit_f64(value),
                (None, None, None) => Err(Error::custom("a number out of range")),
            },
            Value::String(text) => visitor.visit_borrowed_str(text),
            Value::Array(items) => {
                let mut seq = Items(items.iter());
                let value = visitor.visit_seq(&mut seq)?;
                match seq.0.len() {
                    0 => Ok(value),
                    left => Err(Error::invalid_length(items.len() - left, &"fewer elements")),
                }
            }
            Value::Object(fields) => visitor.visit_map(Entries { fields: fields.iter(), value: None, name_errors: false }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(self, name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Object(_) => self.deserialize_any(visitor),
            other => Err(Error::custom(format!("{name} names its fields in an object, not {}", kind(other)))),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(self, name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(text) => visitor.visit_enum(BorrowedStrDeserializer::<Error>::new(text)),
            Value::Object(fields) if fields.len() == 1 => {
                let (variant, value) = fields.iter().next().ok_or_else(|| Error::custom("an empty variant"))?;
                visitor.visit_enum(Variant { name: variant, value })
            }
            other => Err(Error::custom(format!("{name} is a name or a one-field object, not {}", kind(other)))),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map identifier
    }
}

struct Items<'a>(std::slice::Iter<'a, Value>);

impl<'de> SeqAccess<'de> for Items<'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Error> {
        self.0.next().map(|value| seed.deserialize(Named(value))).transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct Entries<'a> {
    fields: serde_json::map::Iter<'a>,
    value: Option<(&'a str, &'a Value)>,
    name_errors: bool,
}

impl<'de> MapAccess<'de> for Entries<'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        let Some((key, value)) = self.fields.next() else {
            return Ok(None);
        };
        self.value = Some((key, value));
        seed.deserialize(Key(key)).map(Some)
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, Error> {
        let (key, value) = self.value.take().ok_or_else(|| Error::custom("a value without its key"))?;
        seed.deserialize(Named(value)).map_err(|error| if self.name_errors { Error::custom(format!("{key}: {error}")) } else { error })
    }
}

struct Key<'a>(&'a str);

macro_rules! parsed {
    ($($method:ident: $kind:ty => $visit:ident),* $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            visitor.$visit(self.0.parse::<$kind>().map_err(|_| Error::custom(format!("map key {} is not a number", self.0)))?)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for Key<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_borrowed_str(self.0)
    }

    parsed! {
        deserialize_i8: i64 => visit_i64, deserialize_i16: i64 => visit_i64, deserialize_i32: i64 => visit_i64, deserialize_i64: i64 => visit_i64,
        deserialize_u8: u64 => visit_u64, deserialize_u16: u64 => visit_u64, deserialize_u32: u64 => visit_u64, deserialize_u64: u64 => visit_u64,
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, Error> {
        visitor.visit_enum(self.0.into_deserializer())
    }

    serde::forward_to_deserialize_any! {
        bool i128 u128 f32 f64 char str string bytes byte_buf option unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

struct Variant<'a> {
    name: &'a String,
    value: &'a Value,
}

impl<'de> EnumAccess<'de> for Variant<'de> {
    type Error = Error;
    type Variant = Named<'de>;

    fn variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<(T::Value, Named<'de>), Error> {
        Ok((seed.deserialize(BorrowedStrDeserializer::<Error>::new(self.name))?, Named(self.value)))
    }
}

impl<'de> VariantAccess<'de> for Named<'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.0 {
            Value::Null => Ok(()),
            other => Err(Error::custom(format!("a unit variant carries no value, not {}", kind(other)))),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_seq(self, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_struct(self, "a struct variant", fields, visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Asked {
        branch_id: String,
        hour: i32,
        within: Vec<(u32, Vec<Span>)>,
        slots: std::collections::BTreeMap<u32, String>,
        key: Option<Key2>,
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct Span {
        first_hour: i32,
        last_hour: i32,
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "camelCase")]
    enum Key2 {
        Home,
        At { hour: i32 },
    }

    #[test]
    fn a_request_names_its_fields_and_a_positional_struct_is_refused() {
        let asked = serde_json::json!({ "branchId": "1", "hour": 104, "within": [[1, [{ "firstHour": 100, "lastHour": 109 }]]], "slots": { "3": "GER" }, "key": { "at": { "hour": 105 } } });
        let read: Asked = named(&asked).unwrap();
        assert_eq!((read.hour, read.within[0].1[0].last_hour, read.slots[&3].as_str(), read.key), (104, 109, "GER", Some(Key2::At { hour: 105 })));
        assert_eq!(named::<Asked>(&serde_json::json!({ "branchId": "1", "hour": 104, "within": [], "slots": {}, "key": "home" })).unwrap().key, Some(Key2::Home));
        let positional = serde_json::json!(["1", 104, [], {}, null]);
        assert!(serde_json::from_value::<Asked>(positional.clone()).is_ok(), "serde_json alone takes a list for a struct");
        assert!(named::<Asked>(&positional).unwrap_err().to_string().contains("names its fields in an object"));
        let nested = serde_json::json!({ "branchId": "1", "hour": 104, "within": [[1, [[100, 120]]]], "slots": {}, "key": null });
        assert!(named::<Asked>(&nested).unwrap_err().to_string().contains("names its fields in an object"), "a nested span is named too");
    }
}
