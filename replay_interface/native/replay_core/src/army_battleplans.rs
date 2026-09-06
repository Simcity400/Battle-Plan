
use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

// Spelled once, in shared ground: the XMAP15 producer's plan census must EQUAL
// the visual set this view yields, and the two live in workspaces that cannot
// depend on each other. See portable_render_stream::XMAP15_D54_ORDER_ROW_FIELDS.
const MAX_ARMY_ID: u32 = portable_render_stream::XMAP15_D54_MAX_ARMY_ID;
const MAX_ORDER_INSTANCE_ID: u32 = portable_render_stream::XMAP15_D54_MAX_ORDER_INSTANCE_ID;
const MAX_MAP_ID: u32 = portable_render_stream::XMAP15_D54_MAX_MAP_ID;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArmyBattleplanOwnerKind {
    Army,
    ArmyGroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArmyBattleplanOwnerKey {
    pub kind: ArmyBattleplanOwnerKind,
    pub id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArmyBattleplanOrderKey {
    pub owner: ArmyBattleplanOwnerKey,
    pub engine_order_instance_id: u32,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmyBattleplanGeometry {
    PathProvinces(Vec<u32>),
    Paradrop {
        source_province_id: u32,
        target_province_id: u32,
    },
    TargetStates(Vec<u32>),
    Unavailable,
    Unsupported,
}

/// on to pick an order's `maparrows.txt` definition. Half a set is no set: the
/// producer refuses all six unless every byte read back 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmyBattleplanStyle {
    pub move_special: bool,
    pub withdraw: bool,
    pub fallback: bool,
    pub virtual_line: bool,
    pub owner_override: bool,
    pub owner_grouped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyBattleplanOrderExact {
    pub key: ArmyBattleplanOrderKey,
    pub slot_index: u32,
    pub order_type: i32,
    pub geometry: ArmyBattleplanGeometry,
    pub assigned_division_ids: Vec<u32>,
    pub active: bool,
    /// `None` where the producer could not read the whole predicate set for
    /// this order; never a default, because every default is a real arrow.
    pub style: Option<ArmyBattleplanStyle>,
    pub linked_orders: portable_render_stream::D54OrderedOrderLinks,
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

    pub fn orders_for_owner(
        &self,
        owner: ArmyBattleplanOwnerKey,
    ) -> impl Iterator<Item = &ArmyBattleplanOrderExact> {
        self.orders
            .iter()
            .filter(move |order| order.key.owner == owner)
    }

    pub fn orders_for_army(
        &self,
        army_id: u32,
    ) -> impl Iterator<Item = &ArmyBattleplanOrderExact> {
        self.orders_for_owner(ArmyBattleplanOwnerKey {
            kind: ArmyBattleplanOwnerKind::Army,
            id: army_id,
        })
    }

    pub fn order(
        &self,
        owner: ArmyBattleplanOwnerKey,
        engine_order_instance_id: u32,
        generation: u64,
    ) -> Option<&ArmyBattleplanOrderExact> {
        let key = ArmyBattleplanOrderKey {
            owner,
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
    pub fn army_battleplans(&self) -> ArmyBattleplansView {
        let manager = self.army_orders_group_manager();
        parse_army_battleplans(self.raw(), &manager)
    }
}

fn parse_army_battleplans(
    raw: &impl crate::WorldRoot,
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
            // A withheld hour still carries the producer's own omission counts.
            // Dropping them here left every consumer with "unavailable" and no
            // way to say which of the fold's refusals this hour hit.
            return ArmyBattleplansView::unavailable(withheld_hour_diagnostic(domain));
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
        let owner_admitted = match order.key.owner.kind {
            ArmyBattleplanOwnerKind::Army => manager.army(order.key.owner.id).is_some(),
            ArmyBattleplanOwnerKind::ArmyGroup => {
                manager.owner_identities_complete && manager.has_army_group(order.key.owner.id)
            }
        };
        if !owner_admitted {
            return ArmyBattleplansView::unavailable(Some(format!(
                "battleplan order row {index} owner is absent from complete manager identity census"
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

/// What the producer said about the hour it withheld, where it said anything.
fn withheld_hour_diagnostic(domain: &Map<String, Value>) -> Option<String> {
    let counts = [
        ("non-exact", "omitted_non_exact_rows_this_hour"),
        ("invalid-path", "path_geometry_omitted_rows_this_hour"),
        ("invalid-identity", "identity_or_active_omitted_rows_this_hour"),
    ]
    .into_iter()
    .filter_map(|(label, key)| {
        let value = required_u32(domain, key)?;
        (value != 0).then(|| format!("{value} {label}"))
    })
    .collect::<Vec<_>>();
    if counts.is_empty() {
        return None;
    }
    Some(format!(
        "the producer omitted {} order row(s) this hour without serialising their identity, so          the fold withheld the whole hour rather than let the accumulator keep rows it could not          confirm",
        counts.join(", ")
    ))
}

fn parse_order(row: &Map<String, Value>, _index: usize) -> Option<ArmyBattleplanOrderExact> {
    const FIELDS: [&str; 19] = portable_render_stream::XMAP15_D54_ORDER_ROW_FIELDS;
    let typed = row.len() == FIELDS.len() + 2
        && FIELDS.iter().all(|field| row.contains_key(*field))
        && row.contains_key("owner_kind")
        && row.contains_key("owner_id");
    if !typed {
        return None;
    }
    let owner = {
        let id = required_u32(row, "owner_id")?;
        match row.get("owner_kind")?.as_str()? {
            "army" if required_u32(row, "army_id") == Some(id) => ArmyBattleplanOwnerKey {
                kind: ArmyBattleplanOwnerKind::Army,
                id,
            },
            "army_group" if row.get("army_id")?.is_null() => ArmyBattleplanOwnerKey {
                kind: ArmyBattleplanOwnerKind::ArmyGroup,
                id,
            },
            _ => return None,
        }
    };
    let engine_order_instance_id = required_u32(row, "engine_order_instance_id")?;
    let generation = row.get("generation")?.as_u64()?;
    let slot_index = required_u32(row, "slot_index")?;
    let order_type_i64 = row.get("order_type")?.as_i64()?;
    let order_type = i32::try_from(order_type_i64).ok()?;
    if owner.id == 0
        || owner.id > MAX_ARMY_ID
        || engine_order_instance_id == 0
        || engine_order_instance_id > MAX_ORDER_INSTANCE_ID
        || slot_index >= portable_render_stream::XMAP15_D54_SLOT_INDEX_COUNT
        || row.get("execution_status")?.as_str()? != "exact"
        || row.get("assigned_units_status")?.as_str()? != "exact"
        || row.get("order_linkage_status")?.as_str()? != "exact"
        || row.get("active_status")?.as_str()? != "exact"
    {
        return None;
    }
    let style = parse_style(row)?;
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
            owner,
            engine_order_instance_id,
            generation,
        },
        slot_index,
        order_type,
        geometry,
        assigned_division_ids,
        active,
        style,
        linked_orders: portable_render_stream::parse_d54_order_links(row.get("linked_orders")?)?,
    })
}

/// `Some(None)` is the producer's own refusal; `None` is a malformed carrier.
fn parse_style(row: &Map<String, Value>) -> Option<Option<ArmyBattleplanStyle>> {
    let flags = required_u32(row, "style_flags")?;
    let has = |bit: u32| flags & bit != 0;
    match row.get("style_status")?.as_str()? {
        "exact" => {
            if flags & !portable_render_stream::XMAP15_D54_STYLE_FLAG_MASK != 0 {
                return None;
            }
            Some(Some(ArmyBattleplanStyle {
                move_special: has(portable_render_stream::XMAP15_D54_STYLE_MOVE_SPECIAL),
                withdraw: has(portable_render_stream::XMAP15_D54_STYLE_WITHDRAW),
                fallback: has(portable_render_stream::XMAP15_D54_STYLE_FALLBACK),
                virtual_line: has(portable_render_stream::XMAP15_D54_STYLE_VIRTUAL),
                owner_override: has(portable_render_stream::XMAP15_D54_STYLE_OWNER_OVERRIDE),
                owner_grouped: has(portable_render_stream::XMAP15_D54_STYLE_OWNER_GROUPED),
            }))
        }
        "validation_failed" | "unknown" | "overflow" | "unsupported" | "fixture_required"
        | "cold_preflight_only" if flags == 0 => Some(None),
        _ => None,
    }
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
            army_group_ids: Vec::new(),
            owner_identities_complete: true,
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
            "owner_kind": "army",
            "owner_id": 17,
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
            "style_flags": 0,
            "style_status": "exact",
            "linked_orders": {"version": 5, "front_binding": {"status": 0, "words": null}, "root_status": 0, "root_positions": [null, null, null], "status": 2, "predecessors": [], "successors": [], "virtual_predecessors": [], "virtual_successors": []}
        })
    }

    fn owned_line_order(owner_id: u64) -> Value {
        let mut order = line_order();
        order["owner_id"] = json!(owner_id);
        order["army_id"] = json!(owner_id);
        order
    }

    #[test]
    fn linked_order_truth_states_are_per_row_and_version_is_mandatory() {
        for status in 0..=2 {
            let mut row = line_order();
            row["linked_orders"]["status"] = json!(status);
            let view = parse_army_battleplans(&raw(row), &manager());
            assert_eq!(view.status, ViewStatus::Present);
            assert_eq!(view.orders[0].linked_orders.status, status);
        }
        let mut row = line_order();
        row.as_object_mut().unwrap().remove("linked_orders");
        assert_eq!(parse_army_battleplans(&raw(row), &manager()).status, ViewStatus::DomainUnavailable);
        let mut row = line_order();
        row["linked_orders"]["root_status"] = json!(1);
        row["linked_orders"]["root_positions"] = json!([0, null, 2]);
        let measured = parse_army_battleplans(&raw(row.clone()), &manager());
        assert_eq!(measured.orders[0].linked_orders.root_positions, [Some(0), None, Some(2)]);
        for field in ["root_status", "root_positions"] {
            let mut missing = row.clone(); missing["linked_orders"].as_object_mut().unwrap().remove(field);
            assert!(parse_army_battleplans(&raw(missing), &manager()).orders.is_empty());
        }
        row["linked_orders"]["root_positions"] = json!([null, null, null]);
        assert_eq!(parse_army_battleplans(&raw(row.clone()), &manager()).orders[0].linked_orders.root_status, 1);
        row["linked_orders"]["root_positions"] = json!([256, null, null]);
        assert!(parse_army_battleplans(&raw(row), &manager()).orders.is_empty());
        let mut row = line_order(); row["linked_orders"]["version"] = json!(3);
        assert_eq!(parse_army_battleplans(&raw(row), &manager()).status, ViewStatus::DomainUnavailable);
    }

    #[test]
    fn virtual_link_vectors_keep_order_generation_and_require_current_grammar() {
        let a = json!({"owner_kind": 0, "owner_id": 17, "generation": 4, "engine_order_instance_id": 2025});
        let b = json!({"owner_kind": 1, "owner_id": 23, "generation": 5, "engine_order_instance_id": 2026});
        let mut row = line_order();
        row["linked_orders"]["status"] = json!(1);
        row["linked_orders"]["virtual_predecessors"] = json!([b, a]);
        row["linked_orders"]["virtual_successors"] = json!([a, b]);
        let view = parse_army_battleplans(&raw(row.clone()), &manager());
        assert_eq!(view.status, ViewStatus::Present);
        let links = &view.orders[0].linked_orders;
        assert_eq!(links.virtual_predecessors.iter().map(|id| id.generation).collect::<Vec<_>>(), [5, 4]);
        assert_eq!(links.virtual_successors.iter().map(|id| id.owner_kind).collect::<Vec<_>>(), [0, 1]);
        for field in ["virtual_predecessors", "virtual_successors"] {
            let mut missing = row.clone(); missing["linked_orders"].as_object_mut().unwrap().remove(field);
            assert_eq!(parse_army_battleplans(&raw(missing), &manager()).status, ViewStatus::DomainUnavailable);
            let mut duplicate = row.clone(); duplicate["linked_orders"][field] = json!([a, a]);
            assert_eq!(parse_army_battleplans(&raw(duplicate), &manager()).status, ViewStatus::DomainUnavailable);
        }
    }

    #[test]
    fn current_order_front_binding_keeps_absent_unavailable_and_exact_read_distinct() {
        for (status, words) in [(0, None), (1, Some([0, 0, 7])), (2, Some([77, 123, 7]))] {
            let mut row = line_order();
            row["linked_orders"]["front_binding"] = json!({"status": status, "words": words});
            let view = parse_army_battleplans(&raw(row), &manager());
            assert_eq!(view.status, ViewStatus::Present);
            assert_eq!(view.orders[0].linked_orders.front_binding,
                portable_render_stream::D54FrontBinding { status, words });
        }
        let mut missing = line_order();
        missing["linked_orders"].as_object_mut().unwrap().remove("front_binding");
        assert_eq!(parse_army_battleplans(&raw(missing), &manager()).status, ViewStatus::DomainUnavailable);
    }

    #[test]
    fn admits_stable_identity_ordered_membership_geometry_and_inactive_state() {
        let view = parse_army_battleplans(&raw(line_order()), &manager());
        assert_eq!(view.status, ViewStatus::Present);
        let order = view
            .order(
                ArmyBattleplanOwnerKey {
                    kind: ArmyBattleplanOwnerKind::Army,
                    id: 17,
                },
                2025,
                4,
            )
            .unwrap();
        assert_eq!(order.assigned_division_ids, vec![301, 205]);
        assert_eq!(
            order.geometry,
            ArmyBattleplanGeometry::PathProvinces(vec![101, 102])
        );
        assert!(!order.active);
    }

    /// kills: this view's admission boundary drifting off the constants the
    /// XMAP15 producer builds its census from.
    ///
    /// `map_viewer/src/production_features/xmap15.rs::cross_check_d54_and_units`
    /// requires the producer's plan count to EQUAL this view's visual set, and
    /// the producer lives in a workspace that cannot depend on this one, so the
    /// bounds are declared once in `portable_render_stream`. Both sides reading
    /// one constant is not enough on its own: this pins the BEHAVIOUR to it, so
    /// a bound moved in shared ground without moving the check here fails.
    #[test]
    fn the_admission_boundary_is_exactly_the_shared_producer_constants() {
        for (field, bound) in [
            (
                "engine_order_instance_id",
                portable_render_stream::XMAP15_D54_MAX_ORDER_INSTANCE_ID,
            ),
            ("army_id", portable_render_stream::XMAP15_D54_MAX_ARMY_ID),
        ] {
            // The manager admits both spellings of the owner, so what the two
            // cases below differ on is the bound and nothing else.
            let mut census = manager();
            census.armies.push(ArmyOrdersGroupScalarExact {
                army_id: portable_render_stream::XMAP15_D54_MAX_ARMY_ID,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            });
            census.registered_entry_count = Some(2);
            let mut at_bound = line_order();
            at_bound[field] = json!(bound);
            if field == "army_id" {
                at_bound["owner_id"] = json!(bound);
            }
            assert_eq!(
                parse_army_battleplans(&raw(at_bound.clone()), &census).status,
                ViewStatus::Present,
                "{field} at the bound is admitted"
            );
            let mut past = at_bound;
            past[field] = json!(u64::from(bound) + 1);
            if field == "army_id" {
                past["owner_id"] = json!(u64::from(bound) + 1);
            }
            assert_eq!(
                parse_army_battleplans(&raw(past), &census).status,
                ViewStatus::DomainUnavailable,
                "{field} one past the bound costs the whole view"
            );
        }

        let mut at_bound = line_order();
        at_bound["path_provinces"] = json!([portable_render_stream::XMAP15_D54_MAX_MAP_ID]);
        assert_eq!(
            parse_army_battleplans(&raw(at_bound), &manager()).status,
            ViewStatus::Present
        );
        let mut past = line_order();
        past["path_provinces"] =
            json!([u64::from(portable_render_stream::XMAP15_D54_MAX_MAP_ID) + 1]);
        assert_eq!(
            parse_army_battleplans(&raw(past), &manager()).status,
            ViewStatus::DomainUnavailable
        );

        let mut at_bound = line_order();
        at_bound["slot_index"] = json!(portable_render_stream::XMAP15_D54_SLOT_INDEX_COUNT - 1);
        assert_eq!(
            parse_army_battleplans(&raw(at_bound), &manager()).status,
            ViewStatus::Present
        );
        let mut past = line_order();
        past["slot_index"] = json!(portable_render_stream::XMAP15_D54_SLOT_INDEX_COUNT);
        assert_eq!(
            parse_army_battleplans(&raw(past), &manager()).status,
            ViewStatus::DomainUnavailable
        );
    }

    /// kills: dropping the producer's own omission counts on a withheld hour,
    /// which left every consumer with "unavailable" and no cause to name.
    #[test]
    fn a_withheld_hour_carries_the_producers_own_omission_counts() {
        let mut raw = raw(line_order());
        let domain = raw["order_of_battle"]["extra"]["army_battleplans_depth_live"]
            .as_object_mut()
            .expect("d54 carrier");
        domain.insert("known".into(), json!(false));
        domain.insert("omitted_non_exact_rows_this_hour".into(), json!(742));
        let view = parse_army_battleplans(&raw, &manager());
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        let diagnostic = view.diagnostic.expect("a withheld hour names its cause");
        assert!(diagnostic.contains("742 non-exact"), "{diagnostic}");

        // A withheld hour that omitted nothing has nothing to name, and must
        // not invent a count.
        let mut silent = raw.clone();
        silent["order_of_battle"]["extra"]["army_battleplans_depth_live"]
            ["omitted_non_exact_rows_this_hour"] = json!(0);
        assert_eq!(
            parse_army_battleplans(&silent, &manager()).diagnostic,
            None
        );
    }

    #[test]
    fn rejects_fabricated_semantics_and_orphan_armies() {
        let mut fabricated = line_order();
        fabricated["style_flags"] =
            json!(portable_render_stream::XMAP15_D54_STYLE_FLAG_MASK + 1);
        let view = parse_army_battleplans(&raw(fabricated), &manager());
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.orders.is_empty());

        let orphan = owned_line_order(18);
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

    #[test]
    fn typed_army_group_owner_requires_complete_manager_identity_census() {
        let mut typed = line_order();
        typed["army_id"] = Value::Null;
        typed["owner_kind"] = json!("army_group");
        typed["owner_id"] = json!(23);

        let mut exact_manager = manager();
        exact_manager.registered_entry_count = Some(2);
        exact_manager.army_group_count = Some(1);
        exact_manager.army_group_ids = vec![23];
        let view = parse_army_battleplans(&raw(typed.clone()), &exact_manager);
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(
            view.orders[0].key.owner,
            ArmyBattleplanOwnerKey {
                kind: ArmyBattleplanOwnerKind::ArmyGroup,
                id: 23,
            }
        );

        exact_manager.army_group_ids.clear();
        exact_manager.owner_identities_complete = false;
        let rejected = parse_army_battleplans(&raw(typed), &exact_manager);
        assert_eq!(rejected.status, ViewStatus::DomainUnavailable);
        assert!(rejected.orders.is_empty());
    }
}
