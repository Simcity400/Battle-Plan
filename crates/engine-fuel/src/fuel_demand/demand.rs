use super::air::{array, pointer, slot};
use super::*;
use engine_native_air::{cair_base, cair_wing, cstrategic_air, cstrategic_air_manager};
use engine_native_parts::cair_wing_pool;
use engine_native::{ccountry, cfleet, corders_group};
use engine_native_memory::{msvc_string, walk::Outcome};

fn add(rows: &mut [usize; 12], index: usize, count: usize) -> Result<(), &'static str> {
    rows[index] = rows[index]
        .checked_add(count)
        .ok_or("fuel demand sizing overflow")?;
    Ok(())
}
fn name(memory: &impl Memory, address: usize, rows: &mut [usize; 12]) -> Result<(), &'static str> {
    if let Ok(name) = msvc_string::Descriptor::read(memory, address) {
        add(rows, 8, name.len)?;
    }
    Ok(())
}
pub fn demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<[usize; 12], &'static str> {
    crate::allocation::advisory(|| demand_inner(memory, base, state))
}

fn demand_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<[usize; 12], &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut rows = [0; 12];
    let manager = field::<u64>(memory, state, engine_native::cgame_state::STRATEGIC_AIR_MANAGER)? as usize;
    let owners = array(memory, manager, cstrategic_air_manager::COUNTRY_AIR, 8)?;
    let mut regions = Vec::new();
    for index in 1..countries.count() {
        let Some(country) = countries.at(index)? else {
            continue;
        };
        add(&mut rows, 0, 1)?;
        let divisions = array(memory, country.pointer, ccountry::DIVISIONS, 8)?;
        add(&mut rows, 1, divisions.count as usize)?;
        for index in 0..divisions.count {
            let division = pointer(memory, divisions, index)?;
            if let Ok(member) = field::<u64>(memory, division, 0x340)
                && member != 0
            {
                name(memory, at(member as usize, 0x60)?, &mut rows)?;
            }
        }
        let air = pointer(memory, owners, country.canonical_slot)?;
        if air == 0 {
            continue;
        }
        for offset in [
            cstrategic_air::FIRST_AIR_SITES,
            cstrategic_air::SECOND_AIR_SITES,
        ] {
            if field::<i32>(memory, air, offset + 12)? <= 0 {
                continue;
            }
            let sites = array(memory, air, offset, 8)?;
            add(&mut rows, 4, sites.count as usize)?;
            for index in 0..sites.count {
                let site = pointer(memory, sites, index)?;
                let containers = array(memory, site, cair_base::COUNTRY_CONTAINERS, 8)?;
                let entry = pointer(memory, containers, slot(memory, &countries, country.id)?)?;
                if entry == 0
                    || field::<i32>(memory, entry, engine_native_air::ccountry_air_container::POOLS_COUNT)? <= 0
                {
                    continue;
                }
                let pools = array(memory, entry, engine_native_air::ccountry_air_container::POOLS, 8)?;
                for index in 0..pools.count {
                    let pool = pointer(memory, pools, index)?;
                    let wings = array(memory, pool, cair_wing_pool::WINGS, 8)?;
                    add(&mut rows, 5, wings.count as usize)?;
                    for index in 0..wings.count {
                        let wing = pointer(memory, wings, index)?;
                        name(memory, at(wing, cair_wing::NAME)?, &mut rows)?;
                        let region = field::<u64>(
                            memory,
                            wing,
                            cair_wing::MISSION + engine_native_air::air_mission::REGION,
                        )? as usize;
                        if region != 0 {
                            crate::allocation::reserve(&mut regions, 1)
                                .map_err(|_| "fuel demand sizing allocation failed")?;
                            regions.push(region);
                        }
                    }
                }
            }
        }
    }
    cfleet::visit_fuel_task_forces(memory, base, &countries, |visit| match visit {
        cfleet::Visit::Fleet { fleet, tasks, .. } => {
            add(&mut rows, 2, 1)?;
            add(&mut rows, 3, tasks.count as usize)?;
            name(memory, at(fleet, cfleet::NAME)?, &mut rows)
        }
        cfleet::Visit::TaskForce(task, _) => name(memory, at(task.task, 0x258)?, &mut rows),
    })?;
    regions.sort_unstable();
    regions.dedup();
    rows[7] = regions.len();
    for region in regions {
        let count = field::<i32>(memory, region, engine_native_parts::cstrategic_region::PROVINCES_COUNT)?;
        if count > 0 {
            add(
                &mut rows,
                6,
                (count as usize)
                    .checked_mul(24)
                    .ok_or("fuel demand weather sizing overflow")?,
            )?;
        }
    }
    let group_count = match corders_group::count_roots(memory, base) {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("fuel demand command sizing overflow")?,
        Outcome::Unavailable { reason } => return Err(reason),
    };
    let mut roots = crate::allocation::default_vec(group_count)
        .map_err(|_| "fuel demand sizing allocation failed")?;
    let Outcome::Rows { len } = corders_group::walk_roots(memory, base, &mut roots) else {
        return Err("fuel demand command sizing census changed");
    };
    rows[9] = len;
    for root in &roots[..len] {
        name(memory, at(root.pointer(), corders_group::NAME)?, &mut rows)?;
        add(
            &mut rows,
            10,
            array(memory, root.pointer(), corders_group::MEMBER_DATA, 8)?.count as usize,
        )?;
        if field::<u8>(memory, root.pointer(), corders_group::IS_ARMY_GROUP)? != 0 {
            add(
                &mut rows,
                11,
                array(memory, root.pointer(), engine_native_parts::carmy_group::MEMBER_DATA, 8)?.count as usize,
            )?;
        }
    }
    Ok(rows)
}
