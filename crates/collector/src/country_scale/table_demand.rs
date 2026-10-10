//! Live storage hints for complete first samples, independently of countries.
//! A failed census preserves previous storage; hints never supply recorded data.
use super::DemandFailures;
use crate::{extended, map_capture::MapCapacities};
use engine::{Memory, walk::Outcome};

const NAMES: &[&str] = &[
    "world",
    "air_wings",
    "naval",
    "province_supply",
    "province_unit_order",
    "governance",
    "characters",
    "projects",
    "lexical",
    "division_templates",
    "industrial_organisations",
    "division_equipment_and_upgrades",
    "available_equipment_variants",
    "task_force_map_motion",
    "relation_opinions",
    "country_stability",
    "division_names",
    "battle_plan_execution",
    "weather",
    "combat_progress",
    "province_names",
    "modifier_definitions",
    "naval_bases",
    "map_facilities",
    "raid_launch_inputs",
    "supply_trains",
    "active_abilities",
    "faction_theatres",
    "logistics",
    "appointed_advisors",
    "raid_target_statuses",
    "raid_state_inputs",
    "combat_log",
    "group_counters",
    "air",
    "naval_history",
    "visibility",
    "fog",
    "country_tactic_unlocks",
    "construction",
    "economy",
    "session",
    "naval_mission_inputs",
    "character_portraits",
    "army_icon_inputs",
    "leader_portraits",
    "leader_paths",
    "naval_result_details",
    "naval_combat_sources",
    "map_combats",
];
static FAILURES: [DemandFailures; NAMES.len()] = [const { DemandFailures::new() }; NAMES.len()];

pub(crate) fn diagnostics() -> impl serde::Serialize {
    crate::status::map(
        NAMES
            .iter()
            .copied()
            .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

pub(crate) fn rows(outcome: Outcome) -> Result<usize, &'static str> {
    match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("table storage demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

pub(crate) fn presize(
    memory: &impl Memory,
    base: usize,
    map: &mut MapCapacities,
    extended: &mut extended::Capacities,
) -> Result<(), &'static str> {
    // The dictionary is independent of the campaign root, including a failed
    // or racing world hint. Count both failures without withholding its floor.
    let dictionary = FAILURES[8].observe(extended.lexical.presize(memory, base, 0));
    let Some(state) = FAILURES[0].observe(super::demand_game_state(memory, base))? else {
        return dictionary;
    };
    let mut failure = dictionary.err();
    let mut accept = |index: usize, result: Result<(), &'static str>| {
        if let Err(reason) = FAILURES[index].observe(result) {
            failure.get_or_insert(reason);
        }
    };
    // These empty-output walks count every original occurrence while skipping
    // body fields. CFleet::serialize@140D4A390 retains +B8/+C4 native order;
    // air pool census uses CStrategicAirManager::country_entry@1406ECA90.
    accept(
        1,
        rows(engine::cair_wing::walk(memory, base, state, &mut []))
            .map(|count| map.raise_air_wings(count)),
    );
    let naval = engine::cfleet::walk(memory, base, state, &mut [], &mut [], &mut []);
    for (index, outcome) in [naval.fleets, naval.task_forces, naval.ships]
        .into_iter()
        .enumerate()
    {
        accept(2, rows(outcome).map(|count| map.raise_naval(index, count)));
    }
    // Checksum@140EB08F0 uses all non-sentinel +10/+1C supply slots.
    accept(
        3,
        rows(engine::sprovince_supply_data::walk(
            memory,
            base,
            state,
            &mut [],
        ))
        .map(|count| super::raise(&mut extended.economy.rows[2], super::rows(count, 1))),
    );
    accept(4, extended.province_unit_order.presize(memory, base, state));
    accept(5, extended.governance.presize(memory, base, state));
    accept(6, extended.characters.presize(memory, base, state));
    accept(7, extended.projects.presize(memory, base, state));
    accept(9, extended.division_templates.presize(memory, base, state));
    accept(
        10,
        extended
            .industrial_organisations
            .presize(memory, base, state),
    );
    accept(11, extended.production.presize(memory, base, state));
    accept(12, extended.model_inputs.presize(memory, base, state));
    accept(13, extended.naval_counters.presize(memory, base, state));
    accept(14, extended.intel.presize(memory, base, state));
    accept(15, extended.country_stability.presize(memory, base, state));
    accept(
        16,
        extended
            .division_presentation
            .presize_names(memory, base, state),
    );
    accept(17, extended.plans.presize_execution(memory, base, state));
    accept(18, extended.weather.presize(memory, base, state));
    accept(19, extended.combat_progress.presize(memory, base));
    extended
        .new_divisions
        .raise_for_equipment(extended.production.rows[2]);
    accept(20, extended.map_names.presize(memory, base, state));
    accept(
        21,
        extended.modifier_definitions.presize(memory, base, state),
    );
    accept(22, extended.naval_bases.presize(memory, base, state));
    accept(23, extended.map_facilities.presize(memory, base, state));
    accept(24, extended.raid_launch_inputs.presize(memory, base, state));
    accept(25, extended.supply_trains.presize(memory, base, state));
    accept(26, extended.active_abilities.presize(memory, base, state));
    accept(27, extended.faction_theatres.presize(memory, base, state));
    accept(28, extended.logistics.presize(memory, base, state));
    accept(29, extended.hired_advisors.presize(memory, base, state));
    accept(
        30,
        extended.raid_target_statuses.presize(memory, base, state),
    );
    accept(31, extended.raid_state_inputs.presize(memory, base, state));
    accept(32, extended.combat_log.presize(memory, base, state));
    accept(33, extended.group_counters.presize(memory, base, state));
    accept(34, extended.air.presize(memory, base, state));
    accept(35, extended.naval_history.presize(memory, base, state));
    accept(36, extended.visibility.presize(memory, base, state));
    accept(37, extended.fog.presize(memory, base, state));
    accept(
        38,
        extended.country_tactic_unlocks.presize(memory, base, state),
    );
    accept(39, extended.construction.presize(memory, base, state));
    accept(40, extended.economy.presize(memory, base, state));
    accept(41, extended.session.presize(memory, base, state));
    accept(
        42,
        extended.naval_mission_inputs.presize(memory, base, state),
    );
    accept(
        43,
        extended.character_portraits.presize(memory, base, state),
    );
    accept(44, extended.army_icon_inputs.presize(memory, base, state));
    accept(45, extended.leader_portraits.presize(memory, base));
    accept(46, extended.leader_paths.presize(memory, base));
    accept(
        47,
        extended.naval_result_details.presize(memory, base, state),
    );
    accept(
        48,
        extended.naval_combat_sources.presize(memory, base, state),
    );
    accept(49, map.presize_combats(memory, base));
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{cgame_state as gs, cprovince, csupply_system};

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
                    (gs::SINGLETON_RVA, header(STATE, 0)),
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
                        csupply_system::VTABLE_RVA as u64,
                        csupply_system::SECONDARY_VTABLE_RVA as u64,
                    ];
                    vtables[(at - SUPPLY) / 8].to_le_bytes()[(at - SUPPLY) % 8]
                } else if (POINTERS..POINTERS + self.provinces as usize * 8).contains(&at) {
                    let slot = (at - POINTERS) / 8;
                    ((OBJECTS + slot * 0x200) as u64).to_le_bytes()[(at - POINTERS) % 8]
                } else if (OBJECTS..OBJECTS + self.provinces as usize * 0x200).contains(&at) {
                    let slot = (at - OBJECTS) / 0x200;
                    match (at - OBJECTS) % 0x200 {
                        field @ 0..8 => (cprovince::VTABLE_RVA as u64).to_le_bytes()[field],
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
        // Other deliberately absent native roots have named failures; the
        // independently valid map populations must still prepare full rows.
        assert!(presize(&memory, 0, &mut sizing.map, &mut sizing.extended).is_err());
        assert!(sizing.extended.economy.rows[2] >= 20000);
        let mut order = extended::province_unit_order::Captured::with_capacities(
            &sizing.extended.province_unit_order,
        );
        order.capture(&memory, 0, 0x1000);
        assert_eq!(order.outcomes(), [Outcome::Rows { len: 20000 }]);
        memory.provinces = 70001;
        assert!(presize(&memory, 0, &mut sizing.map, &mut sizing.extended).is_err());
        assert!(sizing.extended.economy.rows[2] >= 70000);
        let retained = sizing.extended.economy.rows[2];
        memory.provinces = 2;
        assert!(presize(&memory, 0, &mut sizing.map, &mut sizing.extended).is_err());
        assert_eq!(sizing.extended.economy.rows[2], retained);
    }
}
