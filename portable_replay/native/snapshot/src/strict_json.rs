//! Recursive duplicate-key-rejecting JSON decoding.

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::fmt;

/// Decode one complete JSON value while rejecting repeated members at every depth.
///
/// The returned value is built by this pass; callers must not ordinarily decode
/// the bytes first or decode them a second time for semantic use.
pub fn from_slice(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);
struct StrictValueSeed;
struct StrictValueVisitor;

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = StrictValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }
}

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object members")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(v)))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(v.into())))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(v.into())))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(v.to_owned())))
    }
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(v)))
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }
    fn visit_some<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        StrictValue::deserialize(deserializer)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element_seed(StrictValueSeed)? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate JSON member {key:?}")));
            }
            values.insert(key, map.next_value_seed(StrictValueSeed)?.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::from_slice;

    #[test]
    fn rejects_duplicates_recursively_and_names_the_key() {
        for (json, key) in [
            (
                br#"{"session_manifest":1,"session_manifest":2}"#.as_slice(),
                "session_manifest",
            ),
            (
                br#"{"dlc":{"enabled":[],"enabled":["x"]}}"#.as_slice(),
                "enabled",
            ),
            (br#"[{"role":"a","role":"b"}]"#.as_slice(), "role"),
        ] {
            let error = from_slice(json).unwrap_err().to_string();
            assert!(error.contains("duplicate JSON member"), "{error}");
            assert!(error.contains(key), "{error}");
        }
    }

    #[test]
    fn returns_the_value_built_by_the_strict_pass() {
        let value = from_slice(br#"{"a":[null,true,1,"x"],"b":{"c":2}}"#).unwrap();
        assert_eq!(value["a"][3], "x");
        assert_eq!(value["b"]["c"], 2);
    }
}
