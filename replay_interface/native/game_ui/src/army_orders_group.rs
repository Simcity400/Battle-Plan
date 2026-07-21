//! Read-only UI binding for exact army orders-group controls.
//!
//! The a729 producer/localization join authenticates execution and cohesion
//! labels. Leader proximity remains a measured raw ordinal.

use replay_core::{
    ArmyBattleplanGeometry, ArmyBattleplanOrderExact, ArmyBattleplanOrderKey, ArmyBattleplansView,
    ArmyCohesionMode, ArmyExecutionMode, ArmyOrdersGroupManagerView, ViewStatus, WorldFrame,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupControls {
    pub manager_status: ViewStatus,
    pub manager_complete: bool,
    pub selected_army_id: Option<u32>,
    /// `Some(false)` is a measured absence because the manager census is
    /// complete. `None` means the manager or selection was unavailable.
    pub selected_army_present: Option<bool>,
    pub execution_type: Option<u32>,
    pub cohesion_type: Option<u32>,
    pub execution_mode: Option<ArmyExecutionMode>,
    pub cohesion_mode: Option<ArmyCohesionMode>,
    pub leader_proximity: Option<u32>,
}

/// Read-only battleplan panel model. Numeric order types remain raw engine
/// ordinals; type-specific geometry is authenticated by replay_core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyBattleplanPanel {
    pub status: ViewStatus,
    pub selected_army_id: Option<u32>,
    /// A measured false is possible only with a complete manager census.
    pub selected_army_present: Option<bool>,
    pub orders: Vec<ArmyBattleplanOrderExact>,
    pub selected_order: Option<ArmyBattleplanOrderExact>,
}

pub fn bind_army_orders_group_controls(
    frame: &WorldFrame,
    selected_army_id: Option<u32>,
) -> ArmyOrdersGroupControls {
    bind_view(&frame.army_orders_group_manager(), selected_army_id)
}

pub fn bind_army_battleplan_panel(
    frame: &WorldFrame,
    selected_army_id: Option<u32>,
    selected_order_key: Option<&ArmyBattleplanOrderKey>,
) -> ArmyBattleplanPanel {
    bind_battleplan_views(
        &frame.army_battleplans(),
        &frame.army_orders_group_manager(),
        selected_army_id,
        selected_order_key,
    )
}

fn bind_battleplan_views(
    battleplans: &ArmyBattleplansView,
    manager: &ArmyOrdersGroupManagerView,
    selected_army_id: Option<u32>,
    selected_order_key: Option<&ArmyBattleplanOrderKey>,
) -> ArmyBattleplanPanel {
    if battleplans.status != ViewStatus::Present
        || manager.status != ViewStatus::Present
        || !manager.complete
    {
        return ArmyBattleplanPanel {
            status: if battleplans.status != ViewStatus::Present {
                battleplans.status
            } else {
                manager.status
            },
            selected_army_id,
            selected_army_present: None,
            orders: Vec::new(),
            selected_order: None,
        };
    }
    let Some(army_id) = selected_army_id else {
        return ArmyBattleplanPanel {
            status: ViewStatus::Present,
            selected_army_id: None,
            selected_army_present: None,
            orders: Vec::new(),
            selected_order: None,
        };
    };
    let army_present = manager.army(army_id).is_some();
    let orders = battleplans
        .orders_for_army(army_id)
        .cloned()
        .collect::<Vec<_>>();
    let selected_order = selected_order_key
        .filter(|key| key.army_id == army_id)
        .and_then(|key| {
            battleplans.order(
                key.army_id,
                key.engine_order_instance_id,
                key.generation,
            )
        })
        .cloned();
    ArmyBattleplanPanel {
        status: ViewStatus::Present,
        selected_army_id: Some(army_id),
        selected_army_present: Some(army_present),
        orders,
        selected_order,
    }
}

fn bind_view(
    manager: &ArmyOrdersGroupManagerView,
    selected_army_id: Option<u32>,
) -> ArmyOrdersGroupControls {
    if manager.status != ViewStatus::Present || !manager.complete {
        return ArmyOrdersGroupControls {
            manager_status: manager.status,
            manager_complete: false,
            selected_army_id,
            selected_army_present: None,
            execution_type: None,
            cohesion_type: None,
            execution_mode: None,
            cohesion_mode: None,
            leader_proximity: None,
        };
    }
    let Some(army_id) = selected_army_id else {
        return ArmyOrdersGroupControls {
            manager_status: ViewStatus::Present,
            manager_complete: true,
            selected_army_id: None,
            selected_army_present: None,
            execution_type: None,
            cohesion_type: None,
            execution_mode: None,
            cohesion_mode: None,
            leader_proximity: None,
        };
    };
    let row = manager.army(army_id);
    ArmyOrdersGroupControls {
        manager_status: ViewStatus::Present,
        manager_complete: true,
        selected_army_id: Some(army_id),
        selected_army_present: Some(row.is_some()),
        execution_type: row.map(|row| row.execution_type),
        cohesion_type: row.map(|row| row.cohesion_type),
        execution_mode: row.map(|row| row.execution_mode()),
        cohesion_mode: row.map(|row| row.cohesion_mode()),
        leader_proximity: row.map(|row| row.leader_proximity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{ArmyOrdersGroupScalarExact, ORDERS_GROUP_MANAGER_AUTHORITY};

    fn manager(status: ViewStatus) -> ArmyOrdersGroupManagerView {
        ArmyOrdersGroupManagerView {
            status,
            authority: (status == ViewStatus::Present)
                .then_some(ORDERS_GROUP_MANAGER_AUTHORITY.into()),
            complete: status == ViewStatus::Present,
            registered_entry_count: (status == ViewStatus::Present).then_some(1),
            army_group_count: (status == ViewStatus::Present).then_some(0),
            armies: if status == ViewStatus::Present {
                vec![ArmyOrdersGroupScalarExact {
                    army_id: 17,
                    execution_type: 2,
                    cohesion_type: 3,
                    leader_proximity: 4,
                }]
            } else {
                Vec::new()
            },
            diagnostic: None,
        }
    }

    #[test]
    fn binds_selected_zero_order_army_scalars() {
        let bound = bind_view(&manager(ViewStatus::Present), Some(17));
        assert_eq!(bound.selected_army_present, Some(true));
        assert_eq!(bound.execution_type, Some(2));
        assert_eq!(bound.cohesion_type, Some(3));
        assert_eq!(bound.execution_mode, Some(ArmyExecutionMode::Aggressive));
        assert_eq!(bound.cohesion_mode, Some(ArmyCohesionMode::Immobile));
        assert_eq!(bound.leader_proximity, Some(4));
    }

    #[test]
    fn unavailable_manager_never_fabricates_control_values() {
        let bound = bind_view(&manager(ViewStatus::DomainUnavailable), Some(17));
        assert_eq!(bound.selected_army_present, None);
        assert_eq!(bound.execution_type, None);
        assert_eq!(bound.cohesion_type, None);
        assert_eq!(bound.execution_mode, None);
        assert_eq!(bound.cohesion_mode, None);
        assert_eq!(bound.leader_proximity, None);
    }

    #[test]
    fn complete_manager_reports_selected_id_absent_without_default_values() {
        let bound = bind_view(&manager(ViewStatus::Present), Some(18));
        assert_eq!(bound.selected_army_present, Some(false));
        assert_eq!(bound.execution_type, None);
        assert_eq!(bound.cohesion_type, None);
        assert_eq!(bound.execution_mode, None);
        assert_eq!(bound.cohesion_mode, None);
        assert_eq!(bound.leader_proximity, None);
    }

    fn battleplans(status: ViewStatus) -> ArmyBattleplansView {
        ArmyBattleplansView {
            status,
            baseline_seen: status == ViewStatus::Present,
            omitted_non_exact_rows_this_hour: (status == ViewStatus::Present).then_some(0),
            path_geometry_omitted_rows_this_hour: (status == ViewStatus::Present).then_some(0),
            identity_or_active_omitted_rows_this_hour: (status == ViewStatus::Present).then_some(0),
            orders: if status == ViewStatus::Present {
                vec![ArmyBattleplanOrderExact {
                    key: ArmyBattleplanOrderKey {
                        army_id: 17,
                        engine_order_instance_id: 2025,
                        generation: 4,
                    },
                    slot_index: 1,
                    order_type: 2,
                    geometry: ArmyBattleplanGeometry::PathProvinces(vec![101, 102]),
                    assigned_division_ids: vec![301, 205],
                    active: false,
                }]
            } else {
                Vec::new()
            },
            diagnostic: None,
        }
    }

    #[test]
    fn panel_binds_exact_selected_order_without_inventing_labels() {
        let key = ArmyBattleplanOrderKey {
            army_id: 17,
            engine_order_instance_id: 2025,
            generation: 4,
        };
        let panel = bind_battleplan_views(
            &battleplans(ViewStatus::Present),
            &manager(ViewStatus::Present),
            Some(17),
            Some(&key),
        );
        assert_eq!(panel.status, ViewStatus::Present);
        assert_eq!(panel.selected_army_present, Some(true));
        assert_eq!(panel.orders.len(), 1);
        assert_eq!(panel.selected_order.unwrap().assigned_division_ids, vec![301, 205]);
    }

    #[test]
    fn panel_fails_closed_when_order_carrier_is_unavailable() {
        let panel = bind_battleplan_views(
            &battleplans(ViewStatus::DomainUnavailable),
            &manager(ViewStatus::Present),
            Some(17),
            None,
        );
        assert_eq!(panel.status, ViewStatus::DomainUnavailable);
        assert_eq!(panel.selected_army_present, None);
        assert!(panel.orders.is_empty());
    }
}
