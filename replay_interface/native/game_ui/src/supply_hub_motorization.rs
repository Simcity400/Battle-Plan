//! Production binding for D62 persisted supply-hub motorization settings.

use replay_core::{HubMotorizationRow, HubMotorizationView, Leaf, ViewStatus, WorldFrame};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyHubMotorization {
    pub owner_tag_db_id: u32,
    pub province_id: u32,
    pub actor_tag_db_id: u32,
    pub level: u32,
    pub truck_fraction_raw: u32,
}

pub fn bind_supply_hub_motorization(
    frame: &WorldFrame,
    owner_tag_db_id: u32,
    province_id: u32,
    actor_tag_db_id: u32,
) -> Leaf<SupplyHubMotorization> {
    bind_supply_hub_motorization_view(
        frame.hub_motorization(),
        owner_tag_db_id,
        province_id,
        actor_tag_db_id,
    )
}

/// Persisted selected-country settings used by the supply-map and hub tooltip
/// composition layers. A measured empty vector is authoritative only when D62
/// itself is present.
pub fn bind_supply_hub_motorization_rows(
    frame: &WorldFrame,
    country: &str,
) -> Leaf<Vec<HubMotorizationRow>> {
    match frame.hub_motorization().status {
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
        ViewStatus::Present => Leaf::Measured(
            frame
                .hub_motorization()
                .rows
                .iter()
                .filter(|row| row.country == country || row.motorization_country == country)
                .cloned()
                .collect(),
        ),
    }
}

fn bind_supply_hub_motorization_view(
    view: &HubMotorizationView,
    owner_tag_db_id: u32,
    province_id: u32,
    actor_tag_db_id: u32,
) -> Leaf<SupplyHubMotorization> {
    match view.status {
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
        ViewStatus::Present => {
            let level = view
                .level_for(owner_tag_db_id, province_id, actor_tag_db_id)
                .expect("Present D62 view has an exact measured level");
            Leaf::Measured(SupplyHubMotorization {
                owner_tag_db_id,
                province_id,
                actor_tag_db_id,
                level,
                truck_fraction_raw: level * 50_000,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_level_and_complete_absence_bind_as_two_and_zero() {
        let view = HubMotorizationView {
            status: ViewStatus::Present,
            rows: vec![HubMotorizationRow {
                country: "GER".into(),
                owner_tag_db_id: 1,
                motorization_country: "GER".into(),
                actor_tag_db_id: 1,
                province_id: 42,
                state_id: 7,
                record_index: 0,
                map_entry_index: 0,
                motorization_level: 2,
                truck_fraction_raw: 100_000,
            }],
        };
        assert_eq!(
            bind_supply_hub_motorization_view(&view, 1, 42, 1)
                .measured()
                .unwrap()
                .level,
            2
        );
        assert_eq!(
            bind_supply_hub_motorization_view(&view, 1, 43, 1)
                .measured()
                .unwrap()
                .level,
            0
        );
    }
}
