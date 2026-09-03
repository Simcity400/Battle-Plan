//! The three-state leaf a typed replay view is read through, and the wire form
//! a shared record serializes it with.
//!
//! `Absent` is a key the document does not carry, `Unobtained` is an explicit
//! `null`, and `Measured` is a value. The replay has always read every leaf
//! this way; it lives here so a record the recorder WRITES and the replay
//! READS can be one struct in the crate both trees depend on, with the same
//! three states on both sides of the file.
//!
//! The wire form is only defined for a leaf that sits in a struct field:
//! `Absent` has no JSON spelling of its own, so a record field is declared
//! with `skip_serializing_if = "Leaf::is_absent"` and `default`, which is what
//! [`crate::combat_records`]'s record macro writes on every field. Serializing
//! a bare `Absent` is refused rather than spelled `null`, because `null` is a
//! different claim.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaf<T> {
    Absent,
    Unobtained,
    Measured(T),
}

impl<T> Leaf<T> {
    pub fn is_measured(&self) -> bool {
        matches!(self, Leaf::Measured(_))
    }

    pub fn is_absent(&self) -> bool {
        matches!(self, Leaf::Absent)
    }

    pub fn measured(&self) -> Option<&T> {
        match self {
            Leaf::Measured(v) => Some(v),
            _ => None,
        }
    }

    /// The recorder's spelling of an optional reading: `None` is `null` on the
    /// wire, never an omitted key.
    pub fn from_option(value: Option<T>) -> Self {
        match value {
            Some(value) => Leaf::Measured(value),
            None => Leaf::Unobtained,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Leaf<U> {
        match self {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(value) => Leaf::Measured(f(value)),
        }
    }
}

impl<T> Default for Leaf<T> {
    fn default() -> Self {
        Leaf::Absent
    }
}

impl<T: Serialize> Serialize for Leaf<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Leaf::Absent => Err(serde::ser::Error::custom(
                "an Absent leaf has no wire form; its field must be declared with \
                 skip_serializing_if = \"Leaf::is_absent\"",
            )),
            Leaf::Unobtained => serializer.serialize_none(),
            Leaf::Measured(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Leaf<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Leaf::from_option)
    }
}

/// The wire form of a leaf whose `null` is a MEASUREMENT of nothing.
///
/// A land combat's `tactic` and `leader_id` are read by the engine and can be
/// read as "none": the recorder writes `null` for that, and the replay has
/// always read it as `Measured(None)`. So on these fields `null` cannot also
/// mean `Unobtained`, and the type says so: serializing an `Unobtained` one is
/// refused, and `null` deserializes to `Measured(None)`. Declared per field
/// with `#[serde(with = "nullable")]`.
pub mod nullable {
    use super::Leaf;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        leaf: &Leaf<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match leaf {
            Leaf::Absent => Err(serde::ser::Error::custom(
                "an Absent leaf has no wire form; its field must be declared with \
                 skip_serializing_if = \"Leaf::is_absent\"",
            )),
            Leaf::Unobtained => Err(serde::ser::Error::custom(
                "a nullable leaf's null means measured-none, so it cannot carry Unobtained",
            )),
            Leaf::Measured(None) => serializer.serialize_none(),
            Leaf::Measured(Some(value)) => value.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Leaf<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Leaf::Measured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        #[serde(default, skip_serializing_if = "Leaf::is_absent")]
        plain: Leaf<i64>,
        #[serde(default, skip_serializing_if = "Leaf::is_absent", with = "nullable")]
        nullable: Leaf<Option<i64>>,
    }

    #[test]
    fn the_three_states_have_three_wire_spellings_and_read_back() {
        let cases = [
            (Leaf::Absent, json!({})),
            (Leaf::Unobtained, json!({"plain": null})),
            (Leaf::Measured(7), json!({"plain": 7})),
        ];
        for (plain, wire) in cases {
            let probe = Probe {
                plain: plain.clone(),
                nullable: Leaf::Absent,
            };
            assert_eq!(serde_json::to_value(&probe).unwrap(), wire);
            let read: Probe = serde_json::from_value(wire).unwrap();
            assert_eq!(read.plain, plain);
            assert_eq!(read.nullable, Leaf::Absent);
        }
    }

    #[test]
    fn a_nullable_leaf_reads_null_as_measured_none_and_refuses_unobtained() {
        let read: Probe = serde_json::from_value(json!({"nullable": null})).unwrap();
        assert_eq!(read.nullable, Leaf::Measured(None));
        let read: Probe = serde_json::from_value(json!({"nullable": 3})).unwrap();
        assert_eq!(read.nullable, Leaf::Measured(Some(3)));
        let written = serde_json::to_value(Probe {
            plain: Leaf::Absent,
            nullable: Leaf::Measured(None),
        })
        .unwrap();
        assert_eq!(written, json!({"nullable": null}));
        let refused = serde_json::to_value(Probe {
            plain: Leaf::Absent,
            nullable: Leaf::Unobtained,
        });
        assert!(refused.unwrap_err().to_string().contains("measured-none"));
    }

    #[test]
    fn a_bare_absent_leaf_is_refused_rather_than_spelled_null() {
        let error = serde_json::to_value(Leaf::<i64>::Absent).unwrap_err();
        assert!(error.to_string().contains("no wire form"));
        assert_eq!(
            serde_json::to_value(vec![Leaf::Unobtained, Leaf::Measured(1)]).unwrap(),
            Value::Array(vec![Value::Null, json!(1)])
        );
    }

    #[test]
    fn a_wrong_typed_value_is_an_error_not_a_silent_state() {
        let error = serde_json::from_value::<Probe>(json!({"plain": "seven"})).unwrap_err();
        assert!(error.to_string().contains("invalid type"), "{error}");
    }
}
