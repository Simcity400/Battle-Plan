//! Pure fold-sidecar emission over authenticated carriers.
//!
//! Nothing here reads or writes global state. The caller owns insertion into
//! the frame and the transaction boundary around all emitted keys.
//!
//! # The absent/unobtained distinction the consumer already implements
//!
//! `admit_xmap17_faction_labels` distinguishes three states:
//!
//! * key **missing** — `DomainAbsent`, "the hour carried no carrier";
//! * key present and **null** — `DomainUnavailable`, "the recorder explicitly
//!   could not read it";
//! * key carries the carrier — admitted or rejected on its merits.
//!
//! [`emit_faction_labels`] maps a refusal to an explicit `null` under the same
//! key, never to a missing key, and keeps the refusal reason beside it. A
//! refused read that vanished from the frame would be indistinguishable from a
//! recorder build that never had the producer — the "degraded outcomes name the
//! cause" defect.

use crate::refusal::Refusal;
use crate::xmap17::{Xmap17FactionLabelsExactV1, FACTION_CARRIER_KEY};
use serde_json::{Map, Value};

/// One key and the value the fold should insert for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarKey {
    pub key: &'static str,
    pub value: Value,
    /// `Some` when the value is an explicit `null` — the reason the recorder
    /// could not obtain it, for the gap ledger.
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

/// Insert the keys into a frame object. The scaffold lane calls this once.
pub fn insert_sidecars(frame: &mut Map<String, Value>, keys: Vec<SidecarKey>) {
    for entry in keys {
        frame.insert(entry.key.to_string(), entry.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kills: dropping the key on a refusal, which would make a recorder that
    /// tried and failed indistinguishable from one with no producer at all.
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
