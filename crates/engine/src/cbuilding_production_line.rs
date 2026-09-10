//! CBuildingProductionLine; exact-build A14 construction report identifies its CBuilding pointer.

use crate::{
    Memory, cbuilding, cbuilding_status, ccountry, cgeneral_production_line, cpdx_array::Array,
    cproduction_line, cproduction_status, crailway_production_line, read, walk::Outcome,
};

pub const VTABLE_RVA: usize = 0x0296_4d28;

/// CBuildingProductionLine CReferenceObject type 59; compact build_profile.rs LINE_TYPE_CONSTRUCTION.
pub const REFERENCE_TYPE_ID: u32 = 59;
/// CBuildingProductionLine CBuilding pointer; A14_RE_EVIDENCE.json and old construction queue walker agree.
pub const BUILDING: usize = 0x70;
/// CBuildingProductionLine repair levels remaining i32; A14 report: level minus healthy_level, zero for new construction.
pub const REPAIR_LEVELS_REMAINING: usize = 0x78;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub building_type: u32,
    pub target_flags: u32,
    pub target_location: u32,
    pub target_state: u32,
    pub target_country: u32,
    pub repair_levels: i32,
}

pub struct Buffers<'a> {
    pub buildings: &'a mut [Row],
    pub railways: &'a mut [crailway_production_line::Row],
    pub route_provinces: &'a mut [u32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub buildings: Outcome,
    pub railways: Outcome,
    pub route_len: usize,
    pub required_route_len: usize,
}

#[derive(Default)]
struct Census {
    len: usize,
    omitted: u32,
    failure: Option<&'static str>,
}

impl Census {
    fn outcome(&self) -> Outcome {
        if let Some(reason) = self.failure {
            Outcome::Unavailable { reason }
        } else if self.omitted != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.omitted,
            }
        } else {
            Outcome::Rows { len: self.len }
        }
    }
    fn omit(&mut self) -> Result<(), &'static str> {
        self.omitted = self
            .omitted
            .checked_add(1)
            .ok_or("construction omitted row count overflow")?;
        Ok(())
    }
}

/// Visits the mixed construction vector once, preserving its shared queue
/// ordinals across building and railway lines. Each family has its own outcome;
/// a corrupt shared country/manager/census invalidates both families.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, buffers: Buffers<'_>) -> Walk {
    let mut buildings = Census::default();
    let mut railways = Census::default();
    let mut route_len = 0_usize;
    let mut required_route_len = 0_usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        // Map roots are required by building targets only. A broken map root
        // does not invalidate the independently readable railway queue.
        let locations = cbuilding_status::Locations::new(memory, game_state);
        let building_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let railway_vtable = base
            .checked_add(crailway_production_line::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let manager = country
                .pointer
                .checked_add(ccountry::PRODUCTION_STATUS)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("construction production status unreadable or null")?;
            if manager
                .checked_add(cproduction_status::COUNTRY)
                .and_then(|p| read::<u64>(memory, p))
                != Some(country.pointer as u64)
            {
                return Err("construction production status country backlink mismatch");
            }
            let lines = manager
                .checked_add(cproduction_status::CONSTRUCTION_LINES)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("construction queue unreadable or invalid")?;
            for ordinal in 0..lines.count {
                let pointer = lines
                    .element(ordinal, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("construction line pointer unreadable or null")?;
                let vtable =
                    read::<u64>(memory, pointer).ok_or("construction line vtable unreadable")?;
                if vtable == building_vtable {
                    if buildings.failure.is_some() {
                        continue;
                    }
                    if buildings.len == buffers.buildings.len() {
                        buildings.omit()?;
                        continue;
                    }
                    let parsed =
                        locations
                            .as_ref()
                            .map_err(|&reason| reason)
                            .and_then(|locations| {
                                building_row(
                                    memory, base, pointer, manager, country.id, ordinal, locations,
                                )
                            });
                    match parsed {
                        Ok(row) => {
                            buffers.buildings[buildings.len] = row;
                            buildings.len += 1;
                        }
                        Err(reason) => buildings.failure = Some(reason),
                    }
                } else if vtable == railway_vtable {
                    if railways.failure.is_some() {
                        continue;
                    }
                    let route = match crailway_production_line::route(memory, pointer) {
                        Ok(route) => route,
                        Err(reason) => {
                            railways.failure = Some(reason);
                            continue;
                        }
                    };
                    required_route_len = required_route_len
                        .checked_add(route.count as usize)
                        .ok_or("railway route capacity count overflow")?;
                    if railways.len == buffers.railways.len()
                        || route.count as usize > buffers.route_provinces.len() - route_len
                    {
                        railways.omit()?;
                        continue;
                    }
                    let parsed = (|| {
                        let common = common(memory, pointer, manager)?;
                        let mut row = crailway_production_line::Row {
                            id: common.id,
                            country: country.id,
                            queue_ordinal: ordinal,
                            active_factories: common.active_factories,
                            cost: common.cost,
                            speed: common.speed,
                            produced: common.produced,
                            priority: common.priority,
                            amount: common.amount,
                            created_hour: common.created_hour,
                            route_start: route_len,
                            route_len: route.count as usize,
                            ..crailway_production_line::Row::default()
                        };
                        crailway_production_line::fields(memory, pointer, &mut row)?;
                        for index in 0..route.count {
                            buffers.route_provinces[route_len + index as usize] = route
                                .element(index, 4)
                                .and_then(|p| read::<u32>(memory, p))
                                .ok_or("railway route province unreadable")?;
                        }
                        Ok(row)
                    })();
                    match parsed {
                        Ok(row) => {
                            route_len += row.route_len;
                            buffers.railways[railways.len] = row;
                            railways.len += 1;
                        }
                        Err(reason) => railways.failure = Some(reason),
                    }
                } else {
                    return Err("unsupported construction line vtable");
                }
            }
        }
        Ok(())
    })();
    if let Err(reason) = result {
        buildings.failure = Some(reason);
        railways.failure = Some(reason);
    }
    Walk {
        buildings: buildings.outcome(),
        railways: railways.outcome(),
        route_len: if railways.failure.is_some() {
            0
        } else {
            route_len
        },
        required_route_len,
    }
}

#[derive(Default)]
struct Common {
    id: u64,
    active_factories: i32,
    cost: i64,
    speed: i64,
    produced: i64,
    priority: i32,
    amount: i32,
    created_hour: i32,
}

fn common(memory: &impl Memory, pointer: usize, manager: usize) -> Result<Common, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("construction line address overflow")
    };
    let mut id = [0; 8];
    let mut factories = [0; 4];
    let mut fixed = [0; 24];
    let mut counts = [0; 8];
    let mut date = [0; 4];
    let mut owner = [0; 8];
    if memory.copy_scattered(
        [
            at(cproduction_line::REFERENCE_TYPE)?,
            at(cproduction_line::ACTIVE_FACTORIES)?,
            at(cproduction_line::COST)?,
            at(cproduction_line::PRIORITY)?,
            at(cgeneral_production_line::CREATED_DATE)?,
            at(cproduction_line::PRODUCTION_STATUS)?,
        ],
        &mut [
            &mut id,
            &mut factories,
            &mut fixed,
            &mut counts,
            &mut date,
            &mut owner,
        ],
    ) != 6
    {
        return Err("construction line common fields unreadable");
    }
    if u64::from_le_bytes(owner) != manager as u64 {
        return Err("construction line production status backlink mismatch");
    }
    let id = u64::from_le_bytes(id);
    if id == 0 {
        return Err("construction line handle is zero");
    }
    Ok(Common {
        id,
        active_factories: i32::from_le_bytes(factories),
        cost: i64::from_le_bytes(fixed[..8].try_into().unwrap()),
        speed: i64::from_le_bytes(fixed[8..16].try_into().unwrap()),
        produced: i64::from_le_bytes(fixed[16..].try_into().unwrap()),
        priority: i32::from_le_bytes(counts[..4].try_into().unwrap()),
        amount: i32::from_le_bytes(counts[4..].try_into().unwrap()),
        created_hour: i32::from_le_bytes(date),
    })
}

fn building_row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    manager: usize,
    country: u32,
    queue_ordinal: u32,
    locations: &cbuilding_status::Locations,
) -> Result<Row, &'static str> {
    let common = common(memory, pointer, manager)?;
    let target = pointer
        .checked_add(BUILDING)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("construction building target unreadable or null")?;
    let expected = base
        .checked_add(cbuilding::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if read::<u64>(memory, target) != Some(expected) {
        return Err("construction building target vtable mismatch");
    }
    let status = target
        .checked_add(cbuilding::BUILDING_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("construction building status unreadable or null")?;
    let (identity, state) = locations.resolve(memory, base, status)?;
    let building_type = target
        .checked_add(cbuilding::TYPE_ID)
        .and_then(|p| read::<u32>(memory, p))
        .ok_or("construction building type unreadable")?;
    let repair_levels = pointer
        .checked_add(REPAIR_LEVELS_REMAINING)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("construction repair level count unreadable")?;
    Ok(Row {
        id: common.id,
        country,
        queue_ordinal,
        active_factories: common.active_factories,
        cost: common.cost,
        speed: common.speed,
        produced: common.produced,
        priority: common.priority,
        amount: common.amount,
        created_hour: common.created_hour,
        building_type,
        target_flags: identity.flags,
        target_location: identity.location,
        target_state: state,
        target_country: identity.country,
        repair_levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, cstate, memory::SliceMemory};

    fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
        bytes[at..at + value.as_ref().len()].copy_from_slice(value.as_ref());
    }
    fn vector(bytes: &mut [u8], at: usize, pointer: u64, count: i32) {
        put(bytes, at, pointer.to_le_bytes());
        put(bytes, at + 8, count.to_le_bytes());
        put(bytes, at + 12, count.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x4500];
        vector(&mut bytes, cgame_state::COUNTRIES, 0x1700, 2);
        vector(&mut bytes, cgame_state::COUNTRY_LINKS, 0x1720, 2);
        vector(&mut bytes, cgame_state::STATES, 0x1740, 2);
        vector(&mut bytes, cgame_state::PROVINCES, 0x1760, 2);
        put(&mut bytes, 0x708, 0x2000_u64.to_le_bytes());
        put(&mut bytes, 0x724, 1_u32.to_le_bytes());
        put(&mut bytes, 0x748, 0x4400_u64.to_le_bytes());
        put(
            &mut bytes,
            0x1000,
            (0x1000 + ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x1000 + ccountry::ID, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x1000 + ccountry::PRODUCTION_STATUS,
            0x3400_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2400 + cproduction_status::COUNTRY,
            0x2000_u64.to_le_bytes(),
        );
        vector(
            &mut bytes,
            0x2400 + cproduction_status::CONSTRUCTION_LINES,
            0x3780,
            2,
        );
        put(&mut bytes, 0x2780, 0x3800_u64.to_le_bytes());
        put(&mut bytes, 0x2788, 0x3900_u64.to_le_bytes());
        for (pointer, vtable, id) in [
            (0x2800, VTABLE_RVA, 0x0000_0001_ffff_ffff_u64),
            (
                0x2900,
                crailway_production_line::VTABLE_RVA,
                0xffff_ffff_0000_0001,
            ),
        ] {
            put(&mut bytes, pointer, (0x1000 + vtable as u64).to_le_bytes());
            put(
                &mut bytes,
                pointer + cproduction_line::REFERENCE_TYPE,
                id.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRODUCTION_STATUS,
                0x3400_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::ACTIVE_FACTORIES,
                27_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::COST,
                9_876_543_210_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::SPEED,
                (-4_321_098_765_i64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRODUCED,
                7_654_321_098_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRIORITY,
                (-9_i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::AMOUNT,
                50_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cgeneral_production_line::CREATED_DATE,
                60_788_575_i32.to_le_bytes(),
            );
        }
        put(&mut bytes, 0x2800 + BUILDING, 0x4900_u64.to_le_bytes());
        put(
            &mut bytes,
            0x2800 + REPAIR_LEVELS_REMAINING,
            2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2900 + crailway_production_line::BASE_LEVEL,
            (-5_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2900 + crailway_production_line::TARGET_LEVEL,
            7_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2900 + crailway_production_line::COMPLETION_FLAG,
            [1],
        );
        put(
            &mut bytes,
            0x2900 + crailway_production_line::CURRENT_SEGMENT,
            (-1_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2900 + crailway_production_line::REMAINING_COST,
            5_678_901_234_i64.to_le_bytes(),
        );
        vector(
            &mut bytes,
            0x2900 + crailway_production_line::ROUTE,
            0x3b00,
            3,
        );
        for (index, province) in [7_u32, 4, 7].into_iter().enumerate() {
            put(&mut bytes, 0x2b00 + index * 4, province.to_le_bytes());
        }
        put(
            &mut bytes,
            0x3400,
            (0x1000 + cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x3400 + cstate::ID, 1_u32.to_le_bytes());
        let status = 0x3400 + cstate::BUILDING_STATUS;
        put(
            &mut bytes,
            status,
            (0x1000 + cbuilding_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            status + cbuilding_status::FLAGS,
            0x102_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            status + cbuilding_status::ID,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            status + cbuilding_status::COUNTRY,
            0x8000_0001_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3900,
            (0x1000 + cbuilding::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3900 + cbuilding::BUILDING_STATUS,
            (0x1000 + status as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3900 + cbuilding::TYPE_ID,
            123_u32.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn mixed_queue_preserves_full_handles_signed_fixed_fields_and_route_order() {
        let bytes = fixture();
        let mut buildings = [Row::default(); 2];
        let mut railways = [crailway_production_line::Row::default(); 2];
        let mut routes = [0; 8];
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            0x1000,
            Buffers {
                buildings: &mut buildings,
                railways: &mut railways,
                route_provinces: &mut routes,
            },
        );
        assert_eq!(
            result,
            Walk {
                buildings: Outcome::Rows { len: 1 },
                railways: Outcome::Rows { len: 1 },
                route_len: 3,
                required_route_len: 3
            }
        );
        assert_eq!(buildings[0].id, 0x0000_0001_ffff_ffff);
        assert_eq!(buildings[0].cost, 9_876_543_210);
        assert_eq!(buildings[0].speed, -4_321_098_765);
        assert_eq!(buildings[0].produced, 7_654_321_098);
        assert_eq!(buildings[0].priority, -9);
        assert_eq!(
            (
                buildings[0].target_flags,
                buildings[0].target_state,
                buildings[0].target_country
            ),
            (0x102, 1, 0x8000_0001)
        );
        assert_eq!(
            (buildings[0].queue_ordinal, railways[0].queue_ordinal),
            (0, 1)
        );
        assert_eq!(railways[0].id, 0xffff_ffff_0000_0001);
        assert_eq!(railways[0].base_level, -5);
        assert_eq!(railways[0].current_segment, -1);
        assert_eq!(railways[0].remaining_cost, 5_678_901_234);
        assert_eq!(&routes[..3], &[7, 4, 7]);
    }

    #[test]
    fn route_capacity_omits_whole_lines_and_target_failure_is_family_local() {
        let mut bytes = fixture();
        let mut buildings = [Row::default(); 2];
        let mut railways = [crailway_production_line::Row::default(); 2];
        let mut routes = [0; 8];
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            0x1000,
            Buffers {
                buildings: &mut buildings,
                railways: &mut railways,
                route_provinces: &mut routes[..2],
            },
        );
        assert_eq!(
            result,
            Walk {
                buildings: Outcome::Rows { len: 1 },
                railways: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                route_len: 0,
                required_route_len: 3
            }
        );
        put(&mut bytes, 0x2800 + BUILDING, 0_u64.to_le_bytes());
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            0x1000,
            Buffers {
                buildings: &mut buildings,
                railways: &mut railways,
                route_provinces: &mut routes,
            },
        );
        assert!(matches!(result.buildings, Outcome::Unavailable { .. }));
        assert_eq!(result.railways, Outcome::Rows { len: 1 });
        put(&mut bytes, 0x2800, 0_u64.to_le_bytes());
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            0x1000,
            Buffers {
                buildings: &mut buildings,
                railways: &mut railways,
                route_provinces: &mut routes,
            },
        );
        assert_eq!(
            result.buildings,
            Outcome::Unavailable {
                reason: "unsupported construction line vtable"
            }
        );
        assert_eq!(result.railways, result.buildings);

        let mut bytes = fixture();
        put(&mut bytes, cgame_state::STATES + 12, 3_i32.to_le_bytes());
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            0x1000,
            Buffers {
                buildings: &mut buildings,
                railways: &mut railways,
                route_provinces: &mut routes,
            },
        );
        assert_eq!(
            result.buildings,
            Outcome::Unavailable {
                reason: "construction target state array unreadable or invalid"
            }
        );
        assert_eq!(result.railways, Outcome::Rows { len: 1 });
    }
}
