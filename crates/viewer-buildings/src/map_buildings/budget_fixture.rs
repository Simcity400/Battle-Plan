use super::*;
use replay_data::Hour;

pub(super) fn prepared(hour: Hour, health: i128, landmark: bool) -> PreparedBuildings {
    PreparedBuildings {
        branch: 1,
        hour,
        shrink_distance: 180.0,
        popup_speed: 5.0,
        slots: Chunked::from(vec![PreparedSlot {
            source: None,
            id: 0,
            drawn_at_distance: Some(landmark),
            entity: Some(
                PreparedEntity {
                    healthy: Selection {
                        admitted: true,
                        mesh: 1,
                        definition: 1,
                        variant: 0,
                    },
                    ruined: Selection {
                        admitted: true,
                        mesh: 2,
                        definition: 2,
                        variant: 0,
                    },
                    destroyed: health <= 6_000_000,
                    uncertain: health > 6_000_000 && health <= 8_000_000,
                    damaged: health < 8_000_000,
                    disable_grow_animation: false,
                    instance: ModelInstance {
                        scale: 1.0,
                        ..Default::default()
                    },
                }
                .retain(),
            ),
        }]),
        groups: Arc::from([group([0.0, 0.0, 2.0, 2.0], vec![0])]),
        city_damage: Default::default(),
    }
}

pub(super) fn group(bounds: [f32; 4], slots: Vec<u32>) -> ProvinceSlots {
    ProvinceSlots {
        province: 0,
        bounds,
        slots,
    }
}

pub fn budget_buildings(provinces: &[(u32, usize)]) -> BuildingHistory {
    let mut input = prepared(10, 10_000_000, false);
    let template = input.slots[0].clone();
    let mut slots = Vec::new();
    let mut groups = Vec::new();
    for &(province, count) in provinces {
        let mut ids = Vec::new();
        for _ in 0..count {
            let mut slot = template.clone();
            slot.id = slots.len() as u32;
            let mesh = 100 + slot.id as usize;
            let healthy = &mut slot.entity_mut().healthy;
            healthy.mesh = mesh;
            healthy.definition = healthy.mesh;
            ids.push(slot.id);
            slots.push(slot);
        }
        groups.push(ProvinceSlots {
            province,
            bounds: [0.0, 0.0, 2.0, 2.0],
            slots: ids,
        });
    }
    input.slots = slots.into();
    input.groups = groups.into();
    let mut history = BuildingHistory::default();
    history.set(input, false);
    history
}

pub fn next_hour(buildings: &BuildingHistory) -> PreparedBuildings {
    let mut next = buildings.prepared.clone().unwrap();
    next.hour += 1;
    next
}

pub fn hour_after(prepared: PreparedBuildings) -> PreparedBuildings {
    prepared.for_hour(prepared.branch, prepared.hour + 1)
}

pub fn destruction(prepared: &mut PreparedBuildings, destroyed: bool, uncertain: bool) {
    prepared.slots.edit(0, |slot| {
        let entity = slot.entity_mut();
        entity.destroyed |= destroyed;
        entity.uncertain |= uncertain;
    });
}

pub fn exempt_growth(buildings: &mut BuildingHistory) {
    buildings.prepared.as_mut().unwrap().slots.edit_all(|slot| {
        slot.entity_mut().disable_grow_animation = true;
    });
}

pub fn generation(buildings: &BuildingHistory) -> u32 {
    buildings.slots[&0].generation
}

pub fn damage(prepared: &mut PreparedBuildings, mesh: usize) {
    prepared.slots.edit(0, |slot| {
        let entity = slot.entity_mut();
        entity.healthy.mesh = mesh;
        entity.damaged = true;
    });
}

pub fn variant(prepared: &mut PreparedBuildings, variant: usize) {
    prepared.slots.edit(0, |slot| slot.entity_mut().healthy.variant = variant);
}

pub fn replace(prepared: &mut PreparedBuildings, mesh: usize, definition: usize) {
    prepared.slots.edit(0, |slot| {
        let entity = slot.entity_mut();
        entity.healthy.mesh = mesh;
        entity.healthy.definition = definition;
    });
}

pub fn file_order(buildings: &mut BuildingHistory, order: Vec<u32>) {
    Arc::make_mut(&mut buildings.prepared.as_mut().unwrap().groups)[0].slots = order;
}
