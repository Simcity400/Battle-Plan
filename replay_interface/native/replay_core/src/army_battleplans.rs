//! Typed replay view for exact a729 domain-54 battleplan orders.
//!
//! The recorder folds the authenticated compact carrier under
//! `order_of_battle.extra.army_battleplans_depth_live`. This parser deliberately
//! rejects partial identities, invented legacy per-order stance/aggressiveness
//! values, and geometry outside the statically proven type-specific laws.

use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
const MAX_MAP_ID: u32 = 65_535;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArmyBattleplanOrderKey {
    pub army_id: u32,
    pub engine_order_instance_id: u32,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmyBattleplanGeometry {
    /// Type 1 move, type 2 line, or type 3 invasion ordered province geometry.
    PathProvinces(Vec<u32>),
    /// Type 4 paradrop endpoints: first +0x70 province and +0xB8 target.
    Paradrop {
        source_province_id: u32,
        target_province_id: u32,
    },
    /// Type 5 area-defense target-state membership.
    TargetStates(Vec<u32>),
    /// A supported type-1/type-2/type-3/type-4 order whose current geometry observation was
    /// not exact. No empty path is promoted as measured geometry.
    Unavailable,
    /// An unknown/modded type. No geometry is inferred.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyBattleplanOrderExact {
    pub key: ArmyBattleplanOrderKey,
    pub slot_index: u32,
    pub order_type: i32,
    pub geometry: ArmyBattleplanGeometry,
    pub assigned_division_ids: Vec<u32>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyBattleplansView {
    pub status: ViewStatus,
    pub baseline_seen: bool,
    pub omitted_non_exact_rows_this_hour: Option<u32>,
    pub path_geometry_omitted_rows_this_hour: Option<u32>,
    pub identity_or_active_omitted_rows_this_hour: Option<u32>,
    pub orders: Vec<ArmyBattleplanOrderExact>,
    pub diagnostic: Option<String>,
}

impl ArmyBattleplansView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            baseline_seen: false,
            omitted_non_exact_rows_this_hour: None,
            path_geometry_omitted_rows_this_hour: None,
            identity_or_active_omitted_rows_this_hour: None,
            orders: Vec::new(),
            diagnostic: None,
        }
    }

    fn unavailable(diagnostic: Option<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            baseline_seen: false,
            omitted_non_exact_rows_this_hour: None,
            path_geometry_omitted_rows_this_hour: None,
            identity_or_active_omitted_rows_this_hour: None,
            orders: Vec::new(),
            diagnostic,
        }
    }

    pub fn orders_for_army(
        &self,
        army_id: u32,
    ) -> impl Iterator<Item = &ArmyBattleplanOrderExact> {
        self.orders
            .iter()
            .filter(move |order| order.key.army_id == army_id)
    }

    pub fn order(
        &self,
        army_id: u32,
        engine_order_instance_id: u32,
        generation: u64,
    ) -> Option<&ArmyBattleplanOrderExact> {
        let key = ArmyBattleplanOrderKey {
            army_id,
            engine_order_instance_id,
            generation,
        };
        self.orders
            .binary_search_by_key(&key, |order| order.key.clone())
            .ok()
            .map(|index| &self.orders[index])
    }
}

impl WorldFrame {
    /// Exact stable order instances, ordered membership, supported geometry,
    /// and active/execution state for this replay hour.
    pub fn army_battleplans(&self) -> ArmyBattleplansView {
        let manager = self.army_orders_group_manager();
        parse_army_battleplans(self.raw(), &manager)
    }
}

fn parse_army_battleplans(
    raw: &Map<String, Value>,
    manager: &crate::ArmyOrdersGroupManagerView,
) -> ArmyBattleplansView {
    let Some(oob) = raw.get("order_of_battle") else {
        return ArmyBattleplansView::absent();
    };
    let Some(oob) = oob.as_object() else {
        return ArmyBattleplansView::unavailable(Some("order_of_battle is not an object".into()));
    };
    let Some(extra) = oob.get("extra") else {
        return ArmyBattleplansView::absent();
    };
    let Some(extra) = extra.as_object() else {
        return ArmyBattleplansView::unavailable(Some(
            "order_of_battle.extra is not an object".into(),
        ));
    };
    let Some(domain) = extra.get("army_battleplans_depth_live") else {
        return ArmyBattleplansView::absent();
    };
    let Some(domain) = domain.as_object() else {
        return ArmyBattleplansView::unavailable(Some(
            "army_battleplans_depth_live is not an object".into(),
        ));
    };
    match domain.get("known") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) | None | Some(Value::Null) => {
            return ArmyBattleplansView::unavailable(None);
        }
        Some(_) => {
            return ArmyBattleplansView::unavailable(Some(
                "army_battleplans_depth_live.known is not bool".into(),
            ));
        }
    }
    if manager.status != ViewStatus::Present || !manager.complete {
        return ArmyBattleplansView::unavailable(Some(
            "known battleplan orders lack complete manager authority".into(),
        ));
    }
    let Some(baseline_seen) = domain.get("baseline_seen").and_then(Value::as_bool) else {
        return ArmyBattleplansView::unavailable(Some(
            "army_battleplans_depth_live.baseline_seen is not bool".into(),
        ));
    };
    if !baseline_seen {
        return ArmyBattleplansView::unavailable(Some(
            "known battleplan orders have no full baseline".into(),
        ));
    }
    let Some(omitted_non_exact_rows_this_hour) =
        required_u32(domain, "omitted_non_exact_rows_this_hour")
    else {
        return ArmyBattleplansView::unavailable(Some(
            "battleplan omitted-row count is not u32".into(),
        ));
    };
    let Some(path_geometry_omitted_rows_this_hour) =
        required_u32(domain, "path_geometry_omitted_rows_this_hour")
    else {
        return ArmyBattleplansView::unavailable(Some(
            "battleplan path-omission count is not u32".into(),
        ));
    };
    let Some(identity_or_active_omitted_rows_this_hour) =
        required_u32(domain, "identity_or_active_omitted_rows_this_hour")
    else {
        return ArmyBattleplansView::unavailable(Some(
            "battleplan identity-omission count is not u32".into(),
        ));
    };
    if !domain
        .get("provenance_notes")
        .and_then(Value::as_array)
        .is_some_and(|notes| notes.iter().all(Value::is_string))
    {
        return ArmyBattleplansView::unavailable(Some(
            "battleplan provenance notes are malformed".into(),
        ));
    }
    let Some(rows) = domain.get("orders").and_then(Value::as_array) else {
        return ArmyBattleplansView::unavailable(Some(
            "battleplan orders is not an array".into(),
        ));
    };

    let manager_armies = manager
        .armies
        .iter()
        .map(|row| row.army_id)
        .collect::<BTreeSet<_>>();
    let mut orders = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for (index, value) in rows.iter().enumerate() {
        let Some(row) = value.as_object() else {
            return ArmyBattleplansView::unavailable(Some(format!(
                "battleplan order row {index} is not an object"
            )));
        };
        let Some(order) = parse_order(row, index) else {
            return ArmyBattleplansView::unavailable(Some(format!(
                "battleplan order row {index} violates exact carrier contract"
            )));
        };
        if !manager_armies.contains(&order.key.army_id) {
            return ArmyBattleplansView::unavailable(Some(format!(
                "battleplan order row {index} army is absent from complete manager"
            )));
        }
        if !seen.insert(order.key.clone()) {
            return ArmyBattleplansView::unavailable(Some(format!(
                "battleplan order row {index} duplicates stable identity"
            )));
        }
        orders.push(order);
    }
    orders.sort_by(|a, b| a.key.cmp(&b.key));
    ArmyBattleplansView {
        status: ViewStatus::Present,
        baseline_seen: true,
        omitted_non_exact_rows_this_hour: Some(omitted_non_exact_rows_this_hour),
        path_geometry_omitted_rows_this_hour: Some(path_geometry_omitted_rows_this_hour),
        identity_or_active_omitted_rows_this_hour: Some(
            identity_or_active_omitted_rows_this_hour,
        ),
        orders,
        diagnostic: None,
    }
}

fn parse_order(row: &Map<String, Value>, _index: usize) -> Option<ArmyBattleplanOrderExact> {
    const FIELDS: [&str; 17] = [
        "army_id",
        "engine_order_instance_id",
        "generation",
        "slot_index",
        "order_type",
        "target_states",
        "path_provinces",
        "target_declared_count",
        "target_status",
        "geometry_status",
        "execution_status",
        "assigned_division_ids",
        "assigned_units_status",
        "order_linkage_status",
        "active",
        "active_status",
        "stance_status",
    ];
    // `aggressiveness_status` is required in addition to the compact array so
    // legacy or pointer-bearing extensions cannot be mistaken for this schema.
    if row.len() != FIELDS.len() + 1
        || FIELDS.iter().any(|field| !row.contains_key(*field))
        || !row.contains_key("aggressiveness_status")
    {
        return None;
    }
    let army_id = required_u32(row, "army_id")?;
    let engine_order_instance_id = required_u32(row, "engine_order_instance_id")?;
    let generation = row.get("generation")?.as_u64()?;
    let slot_index = required_u32(row, "slot_index")?;
    let order_type_i64 = row.get("order_type")?.as_i64()?;
    let order_type = i32::try_from(order_type_i64).ok()?;
    if army_id == 0
        || army_id > MAX_ARMY_ID
        || engine_order_instance_id == 0
        || engine_order_instance_id > MAX_ORDER_INSTANCE_ID
        || slot_index >= 64
        || row.get("execution_status")?.as_str()? != "exact"
        || row.get("assigned_units_status")?.as_str()? != "exact"
        || row.get("order_linkage_status")?.as_str()? != "exact"
        || row.get("active_status")?.as_str()? != "exact"
        || row.get("stance_status")?.as_str()? != "unknown"
        || row.get("aggressiveness_status")?.as_str()? != "unknown"
    {
        return None;
    }
    let active = row.get("active")?.as_bool()?;
    let target_states = strict_id_array(row.get("target_states")?, MAX_MAP_ID)?;
    let path_provinces = strict_id_array(row.get("path_provinces")?, MAX_MAP_ID)?;
    let assigned_division_ids = strict_id_array(row.get("assigned_division_ids")?, MAX_ARMY_ID)?;
    let geometry = match order_type {
        1 | 2 | 3 => {
            if !target_states.is_empty()
                || !row.get("target_declared_count")?.is_null()
                || row.get("target_status")?.as_str()? != "unsupported"
            {
                return None;
            }
            match (row.get("geometry_status")?.as_str()?, path_provinces.is_empty()) {
                ("exact", false) => ArmyBattleplanGeometry::PathProvinces(path_provinces),
                ("fixture_required", true) => ArmyBattleplanGeometry::Unavailable,
                _ => return None,
            }
        }
        4 => {
            if !target_states.is_empty()
                || !row.get("target_declared_count")?.is_null()
                || row.get("target_status")?.as_str()? != "unsupported"
                || row.get("geometry_status")?.as_str()? != "exact"
                || path_provinces.len() != 2
            {
                return None;
            }
            ArmyBattleplanGeometry::Paradrop {
                source_province_id: path_provinces[0],
                target_province_id: path_provinces[1],
            }
        }
        5 => {
            if !path_provinces.is_empty()
                || required_u32(row, "target_declared_count")?
                    != u32::try_from(target_states.len()).ok()?
                || row.get("target_status")?.as_str()? != "exact"
                || row.get("geometry_status")?.as_str()? != "exact"
            {
                return None;
            }
            ArmyBattleplanGeometry::TargetStates(target_states)
        }
        _ => {
            if !target_states.is_empty()
                || !path_provinces.is_empty()
                || !row.get("target_declared_count")?.is_null()
                || row.get("target_status")?.as_str()? != "unsupported"
                || row.get("geometry_status")?.as_str()? != "fixture_required"
            {
                return None;
            }
            ArmyBattleplanGeometry::Unsupported
        }
    };
    Some(ArmyBattleplanOrderExact {
        key: ArmyBattleplanOrderKey {
            army_id,
            engine_order_instance_id,
            generation,
        },
        slot_index,
        order_type,
        geometry,
        assigned_division_ids,
        active,
    })
}

fn required_u32(row: &Map<String, Value>, key: &str) -> Option<u32> {
    row.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn strict_id_array(value: &Value, max: u32) -> Option<Vec<u32>> {
    let values = value.as_array()?;
    let mut out = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let id = value.as_u64().and_then(|value| u32::try_from(value).ok())?;
        if id == 0 || id > max || !seen.insert(id) {
            return None;
        }
        out.push(id);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArmyOrdersGroupManagerView, ArmyOrdersGroupScalarExact};
    use serde_json::json;

    fn manager() -> ArmyOrdersGroupManagerView {
        ArmyOrdersGroupManagerView {
            status: ViewStatus::Present,
            authority: Some(crate::ORDERS_GROUP_MANAGER_AUTHORITY.into()),
            complete: true,
            registered_entry_count: Some(1),
            army_group_count: Some(0),
            armies: vec![ArmyOrdersGroupScalarExact {
                army_id: 17,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            }],
            diagnostic: None,
        }
    }

    fn raw(order: Value) -> Map<String, Value> {
        json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "baseline_seen": true,
                "omitted_non_exact_rows_this_hour": 0,
                "path_geometry_omitted_rows_this_hour": 0,
                "identity_or_active_omitted_rows_this_hour": 0,
                "provenance_notes": [],
                "orders": [order],
                "orders_group_manager": {}
            }}}
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn line_order() -> Value {
        json!({
            "army_id": 17,
            "engine_order_instance_id": 2025,
            "generation": 4,
            "slot_index": 1,
            "order_type": 2,
            "target_states": [],
            "path_provinces": [101, 102],
            "target_declared_count": null,
            "target_status": "unsupported",
            "geometry_status": "exact",
            "execution_status": "exact",
            "assigned_division_ids": [301, 205],
            "assigned_units_status": "exact",
            "order_linkage_status": "exact",
            "active": false,
            "active_status": "exact",
            "stance_status": "unknown",
            "aggressiveness_status": "unknown"
        })
    }

    #[test]
    fn admits_stable_identity_ordered_membership_geometry_and_inactive_state() {
        let view = parse_army_battleplans(&raw(line_order()), &manager());
        assert_eq!(view.status, ViewStatus::Present);
        let order = view.order(17, 2025, 4).unwrap();
        assert_eq!(order.assigned_division_ids, vec![301, 205]);
        assert_eq!(
            order.geometry,
            ArmyBattleplanGeometry::PathProvinces(vec![101, 102])
        );
        assert!(!order.active);
    }

    #[test]
    fn rejects_fabricated_semantics_and_orphan_armies() {
        let mut fabricated = line_order();
        fabricated["stance_status"] = json!("exact");
        let view = parse_army_battleplans(&raw(fabricated), &manager());
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.orders.is_empty());

        let mut orphan = line_order();
        orphan["army_id"] = json!(18);
        let view = parse_army_battleplans(&raw(orphan), &manager());
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.orders.is_empty());
    }

    #[test]
    fn paradrop_geometry_is_typed_source_and_target() {
        let mut paradrop = line_order();
        paradrop["order_type"] = json!(4);
        paradrop["path_provinces"] = json!([701, 909]);
        let view = parse_army_battleplans(&raw(paradrop), &manager());
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(
            view.orders[0].geometry,
            ArmyBattleplanGeometry::Paradrop {
                source_province_id: 701,
                target_province_id: 909,
            }
        );
    }

    #[test]
    fn unknown_geometry_remains_explicitly_empty() {
        let mut unsupported = line_order();
        unsupported["order_type"] = json!(99);
        unsupported["path_provinces"] = json!([]);
        unsupported["geometry_status"] = json!("fixture_required");
        let view = parse_army_battleplans(&raw(unsupported), &manager());
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(view.orders[0].geometry, ArmyBattleplanGeometry::Unsupported);
    }
}
