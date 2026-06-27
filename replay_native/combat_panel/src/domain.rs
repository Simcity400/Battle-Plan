//! Pull the COMBAT domain out of a snapshot + build the controller / province-name
//! resolvers the aggregator needs, preserving no-fabrication at the boundary.
//!
//! Controller resolution prefers `map.control_by_province` (the live province →
//! controller table) and falls back to `provinces.<id>.controller`; both held the
//! SAME tag for all 13 sample combats. A province with no controller entry yields
//! `None` → the defender holding tag becomes Unavailable (never fabricated).

use serde_json::Value as Json;
use std::collections::HashMap;

/// The combat domain extracted from one snapshot, with the resolvers baked in.
#[derive(Debug, Clone, Default)]
pub struct CombatDomain {
    /// The raw `combats[]` array (kept verbatim for aggregation).
    pub combats: Vec<Json>,
    /// province id -> controller TAG (from map.control_by_province ∪ provinces.*).
    pub controller: HashMap<i64, String>,
}

impl CombatDomain {
    /// Build from a snapshot's top-level JSON object (`snapshot.rest` plus header).
    /// Accepts the whole snapshot value; reads `combats`, `map.control_by_province`,
    /// and `provinces`. Missing domains yield an empty domain — callers render "—".
    pub fn from_snapshot_value(snap: &Json) -> CombatDomain {
        let combats = snap
            .get("combats")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();

        let mut controller: HashMap<i64, String> = HashMap::new();

        // (1) provinces.<id>.controller  (fallback / first pass)
        if let Some(provs) = snap.get("provinces").and_then(Json::as_object) {
            for (pid, pv) in provs {
                if let Ok(id) = pid.parse::<i64>() {
                    if let Some(c) = pv.get("controller").and_then(Json::as_str) {
                        controller.insert(id, c.to_string());
                    }
                }
            }
        }
        // (2) map.control_by_province  (preferred; overwrites the fallback)
        if let Some(cbp) = snap
            .get("map")
            .and_then(|m| m.get("control_by_province"))
            .and_then(Json::as_object)
        {
            for (pid, cv) in cbp {
                if let Ok(id) = pid.parse::<i64>() {
                    if let Some(c) = cv.as_str() {
                        controller.insert(id, c.to_string());
                    }
                }
            }
        }

        CombatDomain { combats, controller }
    }

    /// Resolve a province id -> its controller TAG (the DEFENDER holding tag).
    /// `None` when no controller is recorded — never fabricated.
    pub fn controller_of(&self, province: i64) -> Option<String> {
        self.controller.get(&province).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_combats_and_controllers() {
        let snap = json!({
            "combats": [
                { "combat_id":"a", "kind":"land", "location_province":12740,
                  "attackers":[[1,4713]], "defenders":[[2,4713]],
                  "combat_width":null, "modifiers":{"duration":44} }
            ],
            "provinces": { "12740": { "controller": "ENG" } },
            "map": { "control_by_province": { "12740": "ENG", "13240": "ITA" } }
        });
        let d = CombatDomain::from_snapshot_value(&snap);
        assert_eq!(d.combats.len(), 1);
        assert_eq!(d.controller_of(12740).as_deref(), Some("ENG"));
        assert_eq!(d.controller_of(13240).as_deref(), Some("ITA"));
        assert_eq!(d.controller_of(99999), None);
    }

    #[test]
    fn control_by_province_overrides_provinces_fallback() {
        // if the two disagree, the live control table wins (it is the controller of
        // record); both agreed in the sample, but the precedence is deterministic.
        let snap = json!({
            "combats": [],
            "provinces": { "5": { "controller": "FRA" } },
            "map": { "control_by_province": { "5": "GER" } }
        });
        let d = CombatDomain::from_snapshot_value(&snap);
        assert_eq!(d.controller_of(5).as_deref(), Some("GER"));
    }

    #[test]
    fn absent_domains_are_empty_not_fabricated() {
        let d = CombatDomain::from_snapshot_value(&json!({}));
        assert!(d.combats.is_empty());
        assert!(d.controller_of(1).is_none());
    }
}
