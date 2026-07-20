//! Read-only UI binding for exact army orders-group controls.
//!
//! The exact engine evidence proves ordinal domains, not localized labels.
//! Therefore this binding exposes measured raw ordinals only and never invents
//! names for execution type, cohesion type, or leader proximity.

use replay_core::{ArmyOrdersGroupManagerView, ViewStatus, WorldFrame};

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
    pub leader_proximity: Option<u32>,
}

pub fn bind_army_orders_group_controls(
    frame: &WorldFrame,
    selected_army_id: Option<u32>,
) -> ArmyOrdersGroupControls {
    bind_view(&frame.army_orders_group_manager(), selected_army_id)
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
        assert_eq!(bound.leader_proximity, Some(4));
    }

    #[test]
    fn unavailable_manager_never_fabricates_control_values() {
        let bound = bind_view(&manager(ViewStatus::DomainUnavailable), Some(17));
        assert_eq!(bound.selected_army_present, None);
        assert_eq!(bound.execution_type, None);
        assert_eq!(bound.cohesion_type, None);
        assert_eq!(bound.leader_proximity, None);
    }

    #[test]
    fn complete_manager_reports_selected_id_absent_without_default_values() {
        let bound = bind_view(&manager(ViewStatus::Present), Some(18));
        assert_eq!(bound.selected_army_present, Some(false));
        assert_eq!(bound.execution_type, None);
        assert_eq!(bound.cohesion_type, None);
        assert_eq!(bound.leader_proximity, None);
    }
}
