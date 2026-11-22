use serde::de::{self, DeserializeOwned, Visitor};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HourRole {
    Undated,
    Read,
    Start,
    End,
    Bucket,
}

impl HourRole {
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Undated => None,
            Self::Read => Some("read"),
            Self::Start => Some("start"),
            Self::End => Some("end"),
            Self::Bucket => Some("bucket"),
        }
    }
}

pub type Fields = &'static [(&'static str, HourRole)];

pub fn field_head(path: &str) -> &str {
    let end = path.find(['.', '[']).unwrap_or(path.len());
    &path[..end]
}

#[derive(Debug)]
struct Probed(Option<&'static [&'static str]>);

impl fmt::Display for Probed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("request fields probed")
    }
}

impl std::error::Error for Probed {}

impl de::Error for Probed {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self(None)
    }
}

struct Probe;

impl<'de> de::Deserializer<'de> for Probe {
    type Error = Probed;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Probed> {
        Err(Probed(None))
    }

    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, fields: &'static [&'static str], _visitor: V) -> Result<V::Value, Probed> {
        Err(Probed(Some(fields)))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

pub fn struct_fields<T: DeserializeOwned>() -> Option<&'static [&'static str]> {
    match T::deserialize(Probe) {
        Err(Probed(fields)) => fields,
        Ok(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    #[allow(dead_code, reason = "only the field names are read")]
    struct Asked {
        branch_id: String,
        first_hour: i32,
        #[serde(rename = "to")]
        last: i32,
    }

    #[test]
    fn a_request_struct_names_its_serialized_fields() {
        assert_eq!(struct_fields::<Asked>(), Some(&["branchId", "firstHour", "to"][..]));
        assert_eq!(struct_fields::<i32>(), None);
        assert_eq!(field_head("reads[].hour"), "reads");
        assert_eq!(field_head("members.spans"), "members");
        assert_eq!(field_head("hour"), "hour");
    }
}
