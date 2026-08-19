use crate::refusal::Refusal;
use crate::xmap17::{Xmap17FactionLabelsExactV1, FACTION_CARRIER_KEY};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarKey {
    pub key: &'static str,
    pub value: Value,
    pub refusal: Option<Refusal>,
}

fn emit<T: serde::Serialize>(key: &'static str, carrier: Result<&T, Refusal>) -> SidecarKey {
    match carrier {
        Ok(value) => match serde_json::to_value(value) {
            Ok(value) => SidecarKey {
                key,
                value,
                refusal: None,
            },
            Err(error) => SidecarKey {
                key,
                value: Value::Null,
                refusal: Some(Refusal::new(
                    "fold_emission.carrier_not_serializable",
                    error.to_string(),
                )),
            },
        },
        Err(refusal) => SidecarKey {
            key,
            value: Value::Null,
            refusal: Some(refusal),
        },
    }
}

pub fn emit_faction_labels(carrier: Result<&Xmap17FactionLabelsExactV1, Refusal>) -> SidecarKey {
    emit(FACTION_CARRIER_KEY, carrier)
}

pub fn insert_sidecars(frame: &mut Map<String, Value>, keys: Vec<SidecarKey>) {
    for entry in keys {
        frame.insert(entry.key.to_string(), entry.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_read_emits_an_explicit_null_under_the_same_key() {
        let entry = emit_faction_labels(Err(Refusal::new("gamestate.unavailable", "no gamestate")));
        assert_eq!(entry.key, "_xmap17_faction_labels_exact");
        assert_eq!(entry.value, Value::Null);
        assert_eq!(entry.refusal.unwrap().reason, "gamestate.unavailable");

        let mut object = Map::new();
        insert_sidecars(
            &mut object,
            vec![emit_faction_labels(Err(Refusal::new("x.y", "z")))],
        );
        assert!(object.contains_key("_xmap17_faction_labels_exact"));
        assert!(object["_xmap17_faction_labels_exact"].is_null());
    }
}
