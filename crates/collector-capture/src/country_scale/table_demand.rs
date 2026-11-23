use crate::extended;
pub(crate) use collector_extended_support::demand::rows;
use collector_map_capture::map_capture::MapCapacities;
use engine_native_memory::Memory;

/// # Safety
/// Run only in the active session's serial sizing pass or before callback publication at attach.
/// Use that session's image, keep its observer storage live throughout, and exclude detach and
/// competing presizers.
pub unsafe fn presize(
    memory: &impl Memory,
    base: usize,
    map: &mut MapCapacities,
    extended: &mut extended::Capacities,
) -> Result<(), &'static str> {
    // SAFETY: The serialized sizing pass owns the observer storage and excludes teardown.
    unsafe { presize_periodic(memory, base, map, extended, true) }
}

/// # Safety
/// Run only in the active session's serial sizing pass or before callback publication at attach.
/// Use that session's image, keep its observer storage live throughout, and exclude detach and
/// competing presizers.
pub unsafe fn presize_periodic(
    memory: &impl Memory,
    base: usize,
    map: &mut MapCapacities,
    extended: &mut extended::Capacities,
    capacity_census: bool,
) -> Result<(), &'static str> {
    let dictionary = if capacity_census {
        extended.lexical.presize(memory, base, 0)
    } else {
        Ok(())
    };
    let state = super::demand_game_state(memory, base);
    let Some(state) = state? else {
        return dictionary;
    };
    let mut failure = dictionary.err();
    let mut accept = |result: Result<(), &'static str>| {
        if let Err(reason) = result {
            failure.get_or_insert(reason);
        }
    };
    if capacity_census {
        accept(
            rows(engine_native_air::cair_wing::count(memory, base, state))
                .map(|count| map.raise_air_wings(count)),
        );
        let naval = engine_native::cfleet::count(memory, base, state);
        for (index, outcome) in [naval.fleets, naval.task_forces, naval.ships]
            .into_iter()
            .enumerate()
        {
            accept(rows(outcome).map(|count| map.raise_naval(index, count)));
        }
    }
    macro_rules! size {
        (prompt, $result:expr $(,)?) => {
            accept($result)
        };
        (census, $result:expr $(,)?) => {
            if capacity_census {
                accept($result);
            }
        };
    }
    size!(
        census,
        rows(engine_railways::sprovince_supply_data::count(
            memory, base, state
        ))
        .map(|count| super::raise(&mut extended.economy.rows[2], super::rows(count, 1))),
    );
    size!(
        census,
        extended.province_unit_order.presize(memory, base, state)
    );
    size!(census, extended.governance.presize(memory, base, state));
    size!(census, extended.characters.presize(memory, base, state));
    size!(census, extended.projects.presize(memory, base, state));
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe { extended.division_templates.presize(memory, base, state) },
    );
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe {
            extended
                .industrial_organisations
                .presize(memory, base, state)
        },
    );
    size!(census, extended.production.presize(memory, base, state));
    size!(census, extended.model_inputs.presize(memory, base, state));
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe { extended.naval_counters.presize(memory, base, state) },
    );
    size!(census, extended.intel.presize(memory, base, state));
    size!(
        census,
        extended.country_stability.presize(memory, base, state)
    );
    size!(
        prompt,
        extended
            .division_presentation
            .presize_names(memory, base, state),
    );
    size!(
        census,
        extended.plans.presize_execution(memory, base, state)
    );
    size!(census, extended.weather.presize(memory, base, state));
    size!(census, extended.combat_progress.presize(memory, base));
    extended
        .new_divisions
        .raise_for_equipment(extended.production.rows[2]);
    size!(census, extended.map_names.presize(memory, base, state));
    size!(
        census,
        extended.modifier_definitions.presize(memory, base, state),
    );
    size!(census, extended.naval_bases.presize(memory, base, state));
    size!(census, extended.map_facilities.presize(memory, base, state));
    size!(
        census,
        extended.raid_launch_inputs.presize(memory, base, state)
    );
    size!(census, extended.supply_trains.presize(memory, base, state));
    size!(
        census,
        extended.active_abilities.presize(memory, base, state)
    );
    size!(
        census,
        extended.faction_theatres.presize(memory, base, state)
    );
    size!(census, extended.logistics.presize(memory, base, state));
    size!(census, extended.hired_advisors.presize(memory, base, state));
    size!(
        census,
        extended.raid_target_statuses.presize(memory, base, state),
    );
    size!(census, extended.raid_state_inputs.presize(memory, base, state));
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe { extended.combat_log.presize(memory, base, state) },
    );
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe { extended.group_counters.presize(memory, base, state) },
    );
    size!(census, extended.air.presize(memory, base, state));
    size!(census, extended.naval_history.presize(memory, base, state));
    size!(census, extended.visibility.presize(memory, base, state));
    size!(census, extended.fog.presize(memory, base, state));
    size!(
        census,
        extended.country_tactic_unlocks.presize(memory, base, state),
    );
    size!(census, extended.construction.presize(memory, base, state));
    size!(
        prompt,
        // SAFETY: Runs in attach or the session's sole serial sizing pass against its live image.
        unsafe { extended.economy.presize(memory, base, state) },
    );
    size!(census, extended.session.presize(memory, base, state));
    size!(
        census,
        extended.naval_mission_inputs.presize(memory, base, state),
    );
    size!(
        census,
        extended.character_portraits.presize(memory, base, state),
    );
    size!(
        census,
        extended.army_icon_inputs.presize(memory, base, state)
    );
    size!(census, extended.leader_portraits.presize(memory, base));
    size!(census, extended.leader_paths.presize(memory, base));
    size!(
        census,
        extended.naval_result_details.presize(memory, base, state),
    );
    size!(
        census,
        extended.naval_combat_sources.presize(memory, base, state),
    );
    size!(census, map.presize_combats(memory, base));
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{
        cgame_state as gs,
        cprovince,
        ccountry_supply_system::world as csupply_system,
    };
    use engine_native_memory::walk::Outcome;

    struct LargeMap {
        slots: u32,
        provinces: u32,
    }
    impl Memory for LargeMap {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            const STATE: usize = 0x1000;
            const SUPPLY: usize = 0x3000;
            const POINTERS: usize = 0x100_000;
            const OBJECTS: usize = 0x1000_0000;
            for (offset, byte) in output.iter_mut().enumerate() {
                let at = address + offset;
                let header = |data: usize, count: u32| {
                    let mut bytes = [0u8; 16];
                    bytes[..8].copy_from_slice(&(data as u64).to_le_bytes());
                    bytes[8..12].copy_from_slice(&count.to_le_bytes());
                    bytes[12..].copy_from_slice(&count.to_le_bytes());
                    bytes
                };
                let fields = [
                    (gs::SINGLETON_RVA.0, header(STATE, 0)),
                    (STATE + gs::COUNTRIES, header(0x800_0000, self.slots)),
                    (STATE + gs::COUNTRY_LINKS, header(0x900_0000, self.slots)),
                    (STATE + gs::COUNTRY_TAGS, header(0xa00_0000, self.slots)),
                    (STATE + gs::PROVINCES, header(POINTERS, self.provinces)),
                    (STATE + gs::SUPPLY_SYSTEM, header(SUPPLY, 0)),
                    (
                        SUPPLY + csupply_system::PROVINCE_DATA,
                        header(0x600_000, self.provinces),
                    ),
                ];
                let found = fields.iter().find_map(|(root, bytes)| {
                    (*root..*root + 16).contains(&at).then(|| bytes[at - *root])
                });
                *byte = if let Some(byte) = found {
                    byte
                } else if (SUPPLY..SUPPLY + 16).contains(&at) {
                    let vtables = [
                        csupply_system::VTABLE_RVA.0 as u64,
                        csupply_system::SECONDARY_VTABLE_RVA.0 as u64,
                    ];
                    vtables[(at - SUPPLY) / 8].to_le_bytes()[(at - SUPPLY) % 8]
                } else if (POINTERS..POINTERS + self.provinces as usize * 8).contains(&at) {
                    let slot = (at - POINTERS) / 8;
                    ((OBJECTS + slot * 0x200) as u64).to_le_bytes()[(at - POINTERS) % 8]
                } else if (OBJECTS..OBJECTS + self.provinces as usize * 0x200).contains(&at) {
                    let slot = (at - OBJECTS) / 0x200;
                    match (at - OBJECTS) % 0x200 {
                        field @ 0..8 => (cprovince::VTABLE_RVA.0 as u64).to_le_bytes()[field],
                        field if (cprovince::ID..cprovince::ID + 4).contains(&field) => {
                            (slot as u32).to_le_bytes()[field - cprovince::ID]
                        }
                        _ => 0,
                    }
                } else {
                    0
                };
            }
            true
        }
    }

    #[test]
    fn attach_and_unchanged_country_count_follow_live_mod_map_population() {
        let mut memory = LargeMap {
            slots: 5001,
            provinces: 20001,
        };
        assert_eq!(super::super::observe(&memory, 0), Some(5001));
        let mut sizing = super::super::Sizing::for_slots(5001);
        assert!(sizing.extended.economy.rows[2] < 20000);
        assert!(
            // SAFETY: The serialized sizing pass or isolated fixture owns the observer storage
            // throughout.
            unsafe { presize(&memory, 0, &mut sizing.map, &mut sizing.extended) }.is_err()
        );
        assert!(sizing.extended.economy.rows[2] >= 20000);
        let mut order =
            collector_divisions::extended::province_unit_order::Captured::with_capacities(
                &sizing.extended.province_unit_order,
            );
        order.capture(&memory, 0, 0x1000);
        assert_eq!(order.outcomes(), [Outcome::Rows { len: 20000 }]);
        memory.provinces = 70001;
        assert!(
            // SAFETY: The serialized sizing pass or isolated fixture owns the observer storage
            // throughout.
            unsafe { presize(&memory, 0, &mut sizing.map, &mut sizing.extended) }.is_err()
        );
        assert!(sizing.extended.economy.rows[2] >= 70000);
        let retained = sizing.extended.economy.rows[2];
        memory.provinces = 2;
        assert!(
            // SAFETY: The serialized sizing pass or isolated fixture owns the observer storage
            // throughout.
            unsafe { presize(&memory, 0, &mut sizing.map, &mut sizing.extended) }.is_err()
        );
        assert_eq!(sizing.extended.economy.rows[2], retained);
    }
}
