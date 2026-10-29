//! The runtime writers of ship fields served from a held ShipModels read
//! (`cship::HeldShip`): fixed fields, equipment-pool entries, heap fallback
//! sprite, experience accumulator, medal-store accumulator (through the ship's
//! medal-store pointer) and carrier air-base pointer (`cship::CARRIER_AIR_BASE`).
//! Observing them tells the collector which ships to reread; recording never
//! calls them.
//!
//! Writer coverage:
//! - Equipment-stat recalculation rebuilds embedded anti-air, weather factor,
//!   base organisation and maximum strength, fallback sprite and model number
//!   (`cship::ANTI_AIR_BASE`, `BASE_ORGANISATION`, `MAXIMUM_STRENGTH`,
//!   `FALLBACK_SPRITE`, `MODEL_NUMBER`). Its six callers are creation/load
//!   setup, refit, random-part and named-part critical hits, critical-hit
//!   clearing and ability refresh. Every critical array writer recalculates
//!   afterward.
//! - Refit clears and refills the pool (`cship::EQUIPMENT_POOL`), copy-assigns
//!   the embedded definition (`cship::SUBUNIT_DEFINITION`: class
//!   `CLASS_DATABASE_ID`, valid byte `DEFINITION_VALID`, sub-unit index
//!   `SUBUNIT_INDEX`, formation flags `FORMATION_FLAGS`) and erases the carrier
//!   base. Refit completion and the instant-refit command call it. Creation
//!   constructs, deploys and sets the pool before setup recalculates the new
//!   ship. Load invalidates the hour.
//! - Experience refresh clears and refills the experience accumulator at
//!   construction, load repair and in the hourly helper, only when combat or
//!   daily training awards have set its dirty byte.
//! - Carrier-base addition stores `cship::CARRIER_AIR_BASE`. Air-base
//!   destruction clears its host's pointer, so the base itself is an observed
//!   receiver.
//! - Medal refresh clears and refills the store accumulator from award,
//!   save loading and promotion copy. The store pointer changes only at
//!   construction and load.
//! - Ship vtable and handle never change; the hourly census compares its
//!   task force (`cship::TASK_FORCE`).
//! - `cship::OrganisationOperands` keeps the same certified fields: handle,
//!   class, sub-unit index, weather factor, base organisation, and the raw
//!   values of the experience accumulator and the medal store's accumulator.
//!   Its critical header is covered by the three critical-hit writers above,
//!   which all recalculate equipment stats; each entry's count and modifier
//!   pointee stays a fresh read on both passes of the ship modifier
//!   calculation. Terrain-class and leader-subunit terms remain hourly
//!   observations. Only the DLC-set command writes the active modifier
//!   feature mask, and it then always resets the game world, whose observed
//!   reset revokes ship and shared-term epochs. Recompute applies current
//!   effective metadata to the raw values; whole-row carry also compares its
//!   compact metadata guard.
//! - Only task-force doctrine rebuilding clears and rebuilds the task force's
//!   doctrine tree (selector, demand, support capacity and source), from
//!   initialization/registration and ship addition and removal.
//! - Only weather-region modifier refresh clears and rebuilds its modifier
//!   accumulator from weather flags and scripted condition values. Callers
//!   are region update when values change, manager loading and console.
//!   Day/night update rewrites a province's night accumulator only when its
//!   factor differs from the applied factor, then stores that factor.
//!   Equality preserves the accumulator; ShipModels compares the factor
//!   instead of hooking the hourly pass.
//! - Only fleet leader binding stores a fleet's leader reference
//!   (`cfleet::LEADER`): the set-leader and HQ-admiral commands, leader
//!   removal and fleet addition call it; fleet loading invalidates the hour.

pub use crate::advisor_mutations::MutationSite;

/// The writers, each forwarded RCX..R9 by its detour (none takes a stack
/// argument). [`RECEIVER_ARGUMENT`] names the argument that is the receiver.
pub const MUTATIONS: [MutationSite; 10] = [
    // The four-byte DL spill to its home slot and saving RBP relocate.
    MutationSite {
        name: "recalculate_equipment_stats",
        rva: 0xc9_3760,
        sentinel: [
            0x88, 0x54, 0x24, 0x10, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // The five-byte R9 spill to its home slot relocates.
    MutationSite {
        name: "refit",
        rva: 0xc8_a230,
        sentinel: [
            0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x55,
        ],
    },
    // The five-byte RBX spill to RDX's home slot relocates.
    MutationSite {
        name: "rebuild_experience_modifiers",
        rva: 0xc9_3520,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54,
            0x41, 0x55,
        ],
    },
    // The five-byte RBX spill to R9's home slot relocates.
    MutationSite {
        name: "add_carrier_air_base",
        rva: crate::cstrategic_air_manager::ADD_CARRIER_AIR_BASE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    MutationSite {
        name: "destroy_air_base",
        rva: 0xca_4eb0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    MutationSite {
        name: "rebuild_medal_modifiers",
        rva: 0x14e_8780,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    MutationSite {
        name: "rebuild_doctrine_tree",
        rva: 0xdc_c6e0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // Saving RBP, RDI and R12 occupies five relocated bytes.
    MutationSite {
        name: "rebuild_region_weather_modifiers",
        rva: 0xfa_4f50,
        sentinel: [
            0x40, 0x55, 0x57, 0x41, 0x54, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0xf9, 0x48, 0x83,
            0xc1, 0x58,
        ],
    },
    // The five-byte RBX spill to R8's home slot relocates.
    MutationSite {
        name: "bind_fleet_leader",
        rva: 0xdc_04e0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec,
            0x30, 0x83,
        ],
    },
    // Reserve conversion clears the fleet's leader without the binder; the RBX spill relocates.
    MutationSite {
        name: "reset_reserve_fleet",
        rva: 0xdc_07a0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
            0x48, 0x83,
        ],
    },
];

/// Receiver argument (0 = RCX, 1 = RDX): the ship, except carrier-base
/// addition's manager-first call, base destruction, medal refresh, doctrine
/// refresh, weather refresh, leader binding and reserve conversion name the
/// ship, base, store, task force, weather region and fleet respectively.
pub const RECEIVER_ARGUMENT: [usize; 10] = [0, 0, 0, 1, 0, 0, 0, 0, 0, 0];

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_CARRIER_BASE_RVA: usize = 0xca_87e0;

    /// Each sentinel opens with a relocatable stack store; the sites are
    /// distinct and the carrier site is the one the air manager names.
    #[test]
    fn the_sentinels_open_with_relocatable_stack_stores() {
        for site in MUTATIONS {
            assert!(
                matches!(
                    site.sentinel[..2],
                    [0x88, 0x54] | [0x4c, 0x89] | [0x48, 0x89] | [0x40, 0x55]
                ),
                "{}",
                site.name
            );
        }
        for (index, site) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[index + 1..]
                    .iter()
                    .all(|other| other.rva != site.rva)
            );
        }
        assert_eq!(MUTATIONS[3].rva, EXPECTED_CARRIER_BASE_RVA);
    }
}
