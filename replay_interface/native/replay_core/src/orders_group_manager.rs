
use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

// Spelled once, in shared ground: this census is what XMAP13's formation rows
// must cover exactly and what every d54 order's owner is admitted against, and
// the producer that mints both lives in a workspace that cannot depend on this
// one. See portable_render_stream::XMAP13_ORDERS_GROUP_MANAGER_AUTHORITY.
pub const ORDERS_GROUP_MANAGER_AUTHORITY: &str =
    portable_render_stream::XMAP13_ORDERS_GROUP_MANAGER_AUTHORITY;
pub const MAX_TYPED_EXECUTION_TYPE: u32 = 2;
pub const MAX_COHESION_TYPE: u32 = portable_render_stream::XMAP13_MANAGER_MAX_COHESION_TYPE;
pub const MAX_LEADER_PROXIMITY: u32 = portable_render_stream::XMAP13_MANAGER_MAX_LEADER_PROXIMITY;
const MAX_ARMY_ID: u32 = portable_render_stream::XMAP13_MANAGER_MAX_ARMY_ID;
const MAX_REGISTERED_ENTRY_COUNT: u32 =
    portable_render_stream::XMAP13_MANAGER_MAX_REGISTERED_ENTRY_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmyExecutionMode {
    Careful,
    Balanced,
    Aggressive,
    UnlabelledRaw(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmyCohesionMode {
    Flexible,
    Balanced,
    Rigid,
    Immobile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupScalarExact {
    pub army_id: u32,
    pub execution_type: u32,
    pub cohesion_type: u32,
    pub leader_proximity: u32,
}

impl ArmyOrdersGroupScalarExact {
    pub fn execution_mode(&self) -> ArmyExecutionMode {
        match self.execution_type {
            0 => ArmyExecutionMode::Careful,
            1 => ArmyExecutionMode::Balanced,
            2 => ArmyExecutionMode::Aggressive,
            raw => ArmyExecutionMode::UnlabelledRaw(raw),
        }
    }

}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupManagerView {
    pub status: ViewStatus,
    pub authority: Option<String>,
    pub complete: bool,
    pub registered_entry_count: Option<u32>,
    pub army_group_count: Option<u32>,
    pub armies: Vec<ArmyOrdersGroupScalarExact>,
    pub army_group_ids: Vec<u32>,
    pub owner_identities_complete: bool,
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
            army_group_ids: Vec::new(),
            owner_identities_complete: false,
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
            army_group_ids: Vec::new(),
            owner_identities_complete: false,
            diagnostic,
        }
    }

    pub fn army(&self, army_id: u32) -> Option<&ArmyOrdersGroupScalarExact> {
        self.armies
            .binary_search_by_key(&army_id, |row| row.army_id)
            .ok()
            .map(|index| &self.armies[index])
    }

    pub fn has_army_group(&self, army_group_id: u32) -> bool {
        self.army_group_ids.binary_search(&army_group_id).is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmyOwnerColourRefusal {
    Absent,
    Unavailable,
    InvalidCensus,
    OwnerOutsideCensus,
}

impl WorldFrame {
    pub fn army_owner_colour_census(&self) -> Result<portable_render_stream::D54OwnerColourCensus, ArmyOwnerColourRefusal> {
        let manager = self.army_orders_group_manager();
        if manager.status == ViewStatus::DomainAbsent { return Err(ArmyOwnerColourRefusal::Absent); }
        if manager.status != ViewStatus::Present || !manager.complete || !manager.owner_identities_complete {
            return Err(ArmyOwnerColourRefusal::Unavailable);
        }
        let value = self.raw().get("order_of_battle")
            .and_then(|oob| oob.get("extra"))
            .and_then(|extra| extra.get("army_battleplans_depth_live"))
            .and_then(|domain| domain.get("owner_colours"));
        let value = value.ok_or(ArmyOwnerColourRefusal::Absent)?;
        if value.is_null() { return Err(ArmyOwnerColourRefusal::Unavailable); }
        if value.get("rows").and_then(Value::as_array).is_none_or(|rows| rows.len() > 262_144) {
            return Err(ArmyOwnerColourRefusal::InvalidCensus);
        }
        let census: portable_render_stream::D54OwnerColourCensus = serde_json::from_value(value.clone())
            .map_err(|_| ArmyOwnerColourRefusal::InvalidCensus)?;
        census.validate(manager.armies.iter().map(|army| (0, army.army_id))
            .chain(manager.army_group_ids.iter().map(|id| (1, *id))))
            .map_err(|_| ArmyOwnerColourRefusal::InvalidCensus)?;
        Ok(census)
    }

    pub fn army_orders_group_manager(&self) -> ArmyOrdersGroupManagerView {
        parse_orders_group_manager(self.raw())
    }
}

fn parse_orders_group_manager(raw: &impl crate::WorldRoot) -> ArmyOrdersGroupManagerView {
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
    const MANAGER_FIELDS_V1: [&str; 6] = portable_render_stream::XMAP13_MANAGER_FIELDS_V1;
    const MANAGER_FIELDS_V2: [&str; 7] = portable_render_stream::XMAP13_MANAGER_FIELDS_V2;
    let is_v1 = manager.len() == MANAGER_FIELDS_V1.len()
        && MANAGER_FIELDS_V1
            .iter()
            .all(|field| manager.contains_key(*field));
    let is_v2 = manager.len() == MANAGER_FIELDS_V2.len()
        && MANAGER_FIELDS_V2
            .iter()
            .all(|field| manager.contains_key(*field));
    if !is_v1 && !is_v2 {
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
                    .is_some_and(Vec::is_empty)
                && manager
                    .get("army_group_ids")
                    .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty));
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

    let mut army_group_ids = Vec::new();
    if is_v2 {
        let Some(values) = manager.get("army_group_ids").and_then(Value::as_array) else {
            return ArmyOrdersGroupManagerView::unavailable(Some(
                "orders-group manager army-group ids is not an array".into(),
            ));
        };
        if values.len() != army_group_count as usize {
            return ArmyOrdersGroupManagerView::unavailable(Some(
                "orders-group manager army-group identity count mismatch".into(),
            ));
        }
        let mut previous_id = 0u32;
        for (index, value) in values.iter().enumerate() {
            let Some(id) = value.as_u64().and_then(|value| u32::try_from(value).ok()) else {
                return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                    "orders-group manager army-group id {index} is not u32"
                )));
            };
            if id == 0 || id > MAX_ARMY_ID || id <= previous_id {
                return ArmyOrdersGroupManagerView::unavailable(Some(format!(
                    "orders-group manager army-group id {index} is not strict sorted identity"
                )));
            }
            previous_id = id;
            army_group_ids.push(id);
        }
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
        const ROW_FIELDS: [&str; 4] = portable_render_stream::XMAP13_MANAGER_ARMY_ROW_FIELDS;
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
        if cohesion_type > MAX_COHESION_TYPE || leader_proximity > MAX_LEADER_PROXIMITY
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
    if armies
        .iter()
        .any(|army| army_group_ids.binary_search(&army.army_id).is_ok())
    {
        return ArmyOrdersGroupManagerView::unavailable(Some(
            "orders-group manager reuses one type-53 id across owner kinds".into(),
        ));
    }

    ArmyOrdersGroupManagerView {
        status: ViewStatus::Present,
        authority: Some(ORDERS_GROUP_MANAGER_AUTHORITY.into()),
        complete: true,
        registered_entry_count: Some(registered_entry_count),
        army_group_count: Some(army_group_count),
        armies,
        army_group_ids,
        owner_identities_complete: is_v2 || army_group_count == 0,
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
        assert!(!view.owner_identities_complete);
        assert!(view.army_group_ids.is_empty());
    }

    #[test]
    fn v2_admits_strict_complete_army_group_identity_census() {
        let view = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "orders_group_manager": {
                    "known": true,
                    "complete": true,
                    "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 3,
                    "army_group_count": 2,
                    "armies": [
                        {"army_id": 17, "execution_type": 0, "cohesion_type": 1, "leader_proximity": 2}
                    ],
                    "army_group_ids": [23, 24]
                }
            }}}
        })));
        assert_eq!(view.status, ViewStatus::Present);
        assert!(view.owner_identities_complete);
        assert!(view.has_army_group(23));
        assert_eq!(view.army_group_ids, vec![23, 24]);
    }

    /// kills: this view's admission boundary drifting off the constants the
    /// XMAP13 producer builds its formation census from.
    ///
    /// The producer must cover this census EXACTLY
    /// (`xmap13.rs::validate_formation_census`) and XMAP15 admits every d54
    /// order owner against it, and both live in a workspace that cannot depend
    /// on this one - so the bounds are declared once in
    /// `portable_render_stream`. Both sides reading one constant is not enough
    /// on its own: this pins the BEHAVIOUR to it.
    #[test]
    fn the_admission_boundary_is_exactly_the_shared_producer_constants() {
        fn manager(mutate: impl Fn(&mut Value)) -> ViewStatus {
            let mut manager = json!({
                "known": true,
                "complete": true,
                "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                "registered_entry_count": 2,
                "army_group_count": 1,
                "armies": [
                    {"army_id": 17, "execution_type": 0, "cohesion_type": 1, "leader_proximity": 2}
                ],
                "army_group_ids": [23]
            });
            mutate(&mut manager);
            parse_orders_group_manager(&raw(json!({
                "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                    "known": true,
                    "orders_group_manager": manager
                }}}
            })))
            .status
        }

        for (key, bound) in [
            (
                "cohesion_type",
                portable_render_stream::XMAP13_MANAGER_MAX_COHESION_TYPE,
            ),
            (
                "leader_proximity",
                portable_render_stream::XMAP13_MANAGER_MAX_LEADER_PROXIMITY,
            ),
        ] {
            assert_eq!(
                manager(|value| value["armies"][0][key] = json!(bound)),
                ViewStatus::Present,
                "{key} at the bound is admitted"
            );
            assert_eq!(
                manager(|value| value["armies"][0][key] = json!(u64::from(bound) + 1)),
                ViewStatus::DomainUnavailable,
                "{key} one past the bound costs the manager"
            );
        }

        // Each identity kind AT the bound, one case each: they may not collide,
        // so the kind under test takes the bound and the other the value below
        // it. Testing only one of them let a `>` become a `>=` on the other.
        let max_id = portable_render_stream::XMAP13_MANAGER_MAX_ARMY_ID;
        assert_eq!(
            manager(|value| {
                value["armies"][0]["army_id"] = json!(max_id);
                value["army_group_ids"] = json!([max_id - 1]);
            }),
            ViewStatus::Present,
            "an army id at the bound is admitted"
        );
        assert_eq!(
            manager(|value| {
                value["armies"][0]["army_id"] = json!(max_id - 1);
                value["army_group_ids"] = json!([max_id]);
            }),
            ViewStatus::Present,
            "an army-group id at the bound is admitted"
        );
        assert_eq!(
            manager(|value| value["armies"][0]["army_id"] = json!(u64::from(max_id) + 1)),
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            manager(|value| value["army_group_ids"] = json!([u64::from(max_id) + 1])),
            ViewStatus::DomainUnavailable
        );

        // The registered count AT its bound is a census only the v1 shape can
        // state, because v2 would have to enumerate that many identities.
        let max_entries = portable_render_stream::XMAP13_MANAGER_MAX_REGISTERED_ENTRY_COUNT;
        for (label, registered, expected) in [
            ("at the bound", u64::from(max_entries), ViewStatus::Present),
            (
                "one past it",
                u64::from(max_entries) + 1,
                ViewStatus::DomainUnavailable,
            ),
        ] {
            assert_eq!(
                manager(|value| {
                    value
                        .as_object_mut()
                        .expect("manager")
                        .remove("army_group_ids");
                    value["registered_entry_count"] = json!(registered);
                    value["army_group_count"] = json!(registered - 1);
                }),
                expected,
                "the registered count {label}"
            );
        }

        // The authority word and the two canonical field sets are the same
        // spelling on both sides too.
        assert_eq!(
            ORDERS_GROUP_MANAGER_AUTHORITY,
            portable_render_stream::XMAP13_ORDERS_GROUP_MANAGER_AUTHORITY
        );
        assert_eq!(
            manager(|value| value["source"] = json!("registered_reference_type_54")),
            ViewStatus::DomainUnavailable
        );
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
                    "armies": [{"army_id": 17, "execution_type": 2, "cohesion_type": 4, "leader_proximity": 2}]
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

    #[test]
    fn preserves_unlabelled_raw_execution_ordinal_without_inventing_semantics() {
        let view = parse_orders_group_manager(&raw(json!({
            "order_of_battle": {"extra": {"army_battleplans_depth_live": {
                "known": true,
                "orders_group_manager": {
                    "known": true,
                    "complete": true,
                    "source": ORDERS_GROUP_MANAGER_AUTHORITY,
                    "registered_entry_count": 1,
                    "army_group_count": 0,
                    "armies": [
                        {"army_id": 215, "execution_type": 4, "cohesion_type": 0, "leader_proximity": 1}
                    ]
                }
            }}}
        })));
        assert_eq!(view.status, ViewStatus::Present);
        let row = view.army(215).unwrap();
        assert_eq!(row.execution_type, 4);
        assert_eq!(
            row.execution_mode(),
            ArmyExecutionMode::UnlabelledRaw(4)
        );
    }
}
