//! Typed replay view for the exact a729 domain-54 orders-group manager.
//!
//! The recorder places this pointer-free carrier under
//! `order_of_battle.extra.army_battleplans_depth_live.orders_group_manager`.
//! This module is deliberately strict: legacy snapshots remain readable, but
//! they yield `DomainUnavailable`; malformed or contradictory authority never
//! becomes a partial measured view.

use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const ORDERS_GROUP_MANAGER_AUTHORITY: &str = "registered_reference_type_53";
pub const MAX_EXECUTION_TYPE: u32 = 2;
pub const MAX_COHESION_TYPE: u32 = 3;
pub const MAX_LEADER_PROXIMITY: u32 = 4;
const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_REGISTERED_ENTRY_COUNT: u32 = 262_144;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupScalarExact {
    pub army_id: u32,
    pub execution_type: u32,
    pub cohesion_type: u32,
    pub leader_proximity: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupManagerView {
    pub status: ViewStatus,
    pub authority: Option<String>,
    pub complete: bool,
    pub registered_entry_count: Option<u32>,
    pub army_group_count: Option<u32>,
    pub armies: Vec<ArmyOrdersGroupScalarExact>,
    /// Structural rejection reason. This is empty for a valid measured view
    /// and for an honestly unavailable legacy/unknown carrier.
    pub diagnostic: Option<String>,
}

impl ArmyOrdersGroupManagerView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            authority: None,
            complete: false,
            registered_entry_count: None,
            army_group_count: None,
            armies: Vec::new(),
            diagnostic: None,
        }
    }

    fn unavailable(diagnostic: Option<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            authority: None,
            complete: false,
            registered_entry_count: None,
            army_group_count: None,
            armies: Vec::new(),
            diagnostic,
        }
    }

    pub fn army(&self, army_id: u32) -> Option<&ArmyOrdersGroupScalarExact> {
        self.armies
            .binary_search_by_key(&army_id, |row| row.army_id)
            .ok()
            .map(|index| &self.armies[index])
    }
}

impl WorldFrame {
    /// Complete registered type-53 army census and exact command-backed
    /// execution/cohesion/leader-proximity ordinals for this replay hour.
    pub fn army_orders_group_manager(&self) -> ArmyOrdersGroupManagerView {
        parse_orders_group_manager(self.raw())
    }
}

fn parse_orders_group_manager(raw: &Map<String, Value>) -> ArmyOrdersGroupManagerView {
    let Some(oob) = raw.get("order_of_battle") else {
        return ArmyOrdersGroupManagerView::absent();
    };
    let Some(oob) = oob.as_object() else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "order_of_battle is not an object".into(),
        ));
    };
    let Some(extra) = oob.get("extra") else {
        return ArmyOrdersGroupManagerView::absent();
    };
    let Some(extra) = extra.as_object() else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "order_of_battle.extra is not an object".into(),
        ));
    };
    let Some(domain) = extra.get("army_battleplans_depth_live") else {
        return ArmyOrdersGroupManagerView::absent();
    };
    let Some(domain) = domain.as_object() else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "army_battleplans_depth_live is not an object".into(),
        ));
    };
    match domain.get("known") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) | None | Some(Value::Null) => {
            return ArmyOrdersGroupManagerView::unavailable(None);
        }
        Some(_) => {
            return ArmyOrdersGroupManagerView::unavailable(Some(
                "army_battleplans_depth_live.known is not bool".into(),
            ));
        }
    }
    let Some(manager) = domain.get("orders_group_manager") else {
        return ArmyOrdersGroupManagerView::unavailable(None);
    };
    let Some(manager) = manager.as_object() else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders_group_manager is not an object".into(),
        ));
    };
    const MANAGER_FIELDS: [&str; 6] = [
        "known",
        "complete",
        "source",
        "registered_entry_count",
        "army_group_count",
        "armies",
    ];
    if manager.len() != MANAGER_FIELDS.len()
        || MANAGER_FIELDS.iter().any(|field| !manager.contains_key(*field))
    {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders_group_manager has noncanonical field set".into(),
        ));
    }
    match manager.get("known") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            let canonical_unavailable = manager
                .get("complete")
                .and_then(Value::as_bool)
                == Some(false)
                && manager
                    .get("source")
                    .is_none_or(Value::is_null)
                && manager
                    .get("registered_entry_count")
                    .is_none_or(Value::is_null)
                && manager
                    .get("army_group_count")
                    .is_none_or(Value::is_null)
                && manager
                    .get("armies")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty);
            return ArmyOrdersGroupManagerView::unavailable((!canonical_unavailable).then_some(
                "unavailable orders-group manager carries contradictory authority".into(),
            ));
        }
        None | Some(Value::Null) => return ArmyOrdersGroupManagerView::unavailable(None),
        Some(_) => {
            return ArmyOrdersGroupManagerView::unavailable(Some(
                "orders_group_manager.known is not bool".into(),
            ));
        }
    }
    if manager.get("complete").and_then(Value::as_bool) != Some(true) {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "known orders-group manager is not complete".into(),
        ));
    }
    if manager.get("source").and_then(Value::as_str) != Some(ORDERS_GROUP_MANAGER_AUTHORITY) {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager has unknown authority".into(),
        ));
    }

    let Some(registered_entry_count) = manager
        .get("registered_entry_count")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager registered count is not u32".into(),
        ));
    };
    let Some(army_group_count) = manager
        .get("army_group_count")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager army-group count is not u32".into(),
        ));
    };
    if registered_entry_count > MAX_REGISTERED_ENTRY_COUNT {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager registered count exceeds exact table bound".into(),
        ));
    }
    let Some(rows) = manager.get("armies").and_then(Value::as_array) else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager armies is not an array".into(),
        ));
    };
    let Ok(army_count) = u32::try_from(rows.len()) else {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager army count overflows u32".into(),
        ));
    };
    if army_count.checked_add(army_group_count) != Some(registered_entry_count) {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager registered count mismatch".into(),
        ));
    }

    let mut armies = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    let mut previous_id = 0u32;
    for (index, value) in rows.iter().enumerate() {
        let Some(row) = value.as_object() else {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} is not an object"
            )));
        };
        const ROW_FIELDS: [&str; 4] = [
            "army_id",
            "execution_type",
            "cohesion_type",
            "leader_proximity",
        ];
        if row.len() != ROW_FIELDS.len()
            || ROW_FIELDS.iter().any(|field| !row.contains_key(*field))
        {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} has noncanonical field set"
            )));
        }
        let Some(army_id) = required_u32(row, "army_id") else {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} has invalid army_id"
            )));
        };
        let Some(execution_type) = required_u32(row, "execution_type") else {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} has invalid execution_type"
            )));
        };
        let Some(cohesion_type) = required_u32(row, "cohesion_type") else {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} has invalid cohesion_type"
            )));
        };
        let Some(leader_proximity) = required_u32(row, "leader_proximity") else {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} has invalid leader_proximity"
            )));
        };
        if army_id == 0
            || army_id > MAX_ARMY_ID
            || army_id <= previous_id
            || !seen.insert(army_id)
        {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} is not strict sorted identity"
            )));
        }
        if execution_type > MAX_EXECUTION_TYPE
            || cohesion_type > MAX_COHESION_TYPE
            || leader_proximity > MAX_LEADER_PROXIMITY
        {
            return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                "orders-group manager army row {index} scalar is outside exact enum"
            )));
        }
        previous_id = army_id;
        armies.push(ArmyOrdersGroupScalarExact {
            army_id,
            execution_type,
            cohesion_type,
            leader_proximity,
        });
    }

    ArmyOrdersGroupManagerView {
        status: ViewStatus::Present,
        authority: Some(ORDERS_GROUP_MANAGER_AUTHORITY.into()),
        complete: true,
        registered_entry_count: Some(registered_entry_count),
        army_group_count: Some(army_group_count),
        armies,
        diagnostic: None,
    }
}

fn required_u32(row: &Map<String, Value>, key: &str) -> Option<u32> {
    row.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn admits_complete_zero_order_army_rows_and_exact_scalars() {
        let view = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "orders_group_manager": {
                    "known": true,
                    "complete": true,
                    "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 3,
                    "army_group_count": 1,
                    "armies": [
                        {"army_id": 17, "execution_type": 0, "cohesion_type": 1, "leader_proximity": 2},
                        {"army_id": 18, "execution_type": 2, "cohesion_type": 3, "leader_proximity": 4}
                    ]
                }
            }}}
        })));
        assert_eq!(view.status, ViewStatus::Present);
        assert!(view.complete);
        assert_eq!(view.army(18).unwrap().leader_proximity, 4);
    }

    #[test]
    fn malformed_or_legacy_manager_fails_closed() {
        let malformed = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "orders_group_manager": {
                    "known": true,
                    "complete": true,
                    "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 1,
                    "army_group_count": 0,
                    "armies": [{"army_id": 17, "execution_type": 3, "cohesion_type": 1, "leader_proximity": 2}]
                }
            }}}
        })));
        assert_eq!(malformed.status, ViewStatus::DomainUnavailable);
        assert!(malformed.armies.is_empty());
        assert!(malformed.diagnostic.is_some());

        let legacy = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {"known": true}}}
        })));
        assert_eq!(legacy.status, ViewStatus::DomainUnavailable);
        assert!(legacy.diagnostic.is_none());

        let extra_field = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "orders_group_manager": {
                    "known": true,
                    "complete": true,
                    "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 1,
                    "army_group_count": 0,
                    "armies": [{"army_id": 17, "execution_type": 2, "cohesion_type": 1, "leader_proximity": 2}],
                    "object_ptr": "0x200000000"
                }
            }}}
        })));
        assert_eq!(extra_field.status, ViewStatus::DomainUnavailable);
        assert!(extra_field
            .diagnostic
            .as_deref()
            .is_some_and(|reason| reason.contains("noncanonical field set")));
    }
}
