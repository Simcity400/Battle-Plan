//! The runtime writers of ship fields served from a held ShipModels read
//! ([`crate::cship::HeldShip`]): fixed fields, equipment-pool entries, heap
//! fallback sprite, experience accumulator +0x750, medal-store accumulator
//! +0x1B0 (store pointer at ship +0x8F8) and carrier air-base pointer +0x730.
//! Observing them tells the collector which ships to reread; recording never
//! calls them.
//!
//! Writer coverage:
//! - Equipment-stat recalculation rebuilds embedded anti-air +0x218, weather
//!   factor +0x270, base organisation +0x300 and maximum strength +0x308,
//!   fallback sprite +0xE8 and model number +0x810. Its six callers are
//!   creation/load setup, refit, random-part and named-part critical hits,
//!   critical-hit clearing and ability refresh. Every critical array +0x918
//!   writer recalculates afterward.
//! - Refit clears and refills pool +0x40, copy-assigns embedded definition
//!   +0x80 (class +0x88, valid byte +0x90, sub-unit index +0x60C, formation
//!   flags +0x628) and erases the carrier base. Refit completion and the
//!   instant-refit command call it. Creation constructs, deploys and sets
//!   the pool before setup recalculates the new ship. Load invalidates the hour.
//! - Experience refresh clears and refills +0x750 at construction, load
//!   repair and in the hourly helper, only when combat or daily training
//!   awards have set dirty byte +0x800.
//! - Carrier-base addition stores ship +0x730. Air-base destruction clears
//!   its host's pointer, so the base itself is an observed receiver.
//! - Medal refresh clears and refills the store accumulator from award,
//!   save loading and promotion copy. The store pointer changes only at
//!   construction and load.
//! - Ship vtable and handle never change; the hourly census compares its
//!   task force +0x728.
//! - `cship::OrganisationOperands` keeps the same certified fields: handle,
//!   class +0x88, sub-unit index +0x60C, weather factor +0x270, base
//!   organisation +0x300, and the raw values of +0x750 and the medal store's
//!   +0x1B0. Its critical header +0x918 is covered by the three critical-hit
//!   writers above, which all recalculate equipment stats; each entry's count
//!   and modifier pointee stays a fresh read on both passes of the ship
//!   modifier calculation. Terrain-class and leader-subunit terms remain
//!   hourly observations. Only the DLC-set command writes the active modifier
//!   feature mask, and it then always resets the game world, whose observed
//!   reset revokes ship and shared-term epochs. Recompute applies current
//!   effective metadata to the raw values; whole-row carry also compares its
//!   compact metadata guard.
//! - Only task-force doctrine rebuilding clears and rebuilds tree +0x640
//!   (selector +0x20, demand +0x28, support capacity +0x2C, source +0x30),
//!   from initialization/registration and ship addition and removal.
//! - Only weather-region modifier refresh clears and rebuilds accumulator
//!   +0x58 from weather flags and scripted condition values. Callers are
//!   region update when values change, manager loading and console.
//!   Day/night update rewrites province night accumulator +0x280 only when
//!   its factor differs from applied factor +0x268, then stores that factor.
//!   Equality preserves the accumulator; ShipModels compares the factor
//!   instead of hooking the hourly pass.

pub use crate::advisor_mutations::MutationSite;

/// The writers, each forwarded RCX..R9 by its detour (none takes a stack
/// argument). [`RECEIVER_ARGUMENT`] names the argument that is the receiver.
pub const MUTATIONS: [MutationSite; 8] = [
    // The four-byte DL spill at stack +0x10 and saving RBP relocate.
    MutationSite {
        name: "recalculate_equipment_stats",
        rva: 0xc2_d950,
        sentinel: [
            0x88, 0x54, 0x24, 0x10, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // The five-byte R9 spill at stack +0x20 relocates.
    MutationSite {
        name: "refit",
        rva: 0xc2_4460,
        sentinel: [
            0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x55,
        ],
    },
    // The five-byte RBX spill at stack +0x10 relocates.
    MutationSite {
        name: "rebuild_experience_modifiers",
        rva: 0xc2_d710,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54,
            0x41, 0x55,
        ],
    },
    // The five-byte RBX spill at stack +0x20 relocates.
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
        rva: 0xc3_9000,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    MutationSite {
        name: "rebuild_medal_modifiers",
        rva: 0x143_9f00,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    MutationSite {
        name: "rebuild_doctrine_tree",
        rva: 0xd5_1b60,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // Saving RBP, RDI and R12 occupies five relocated bytes.
    MutationSite {
        name: "rebuild_region_weather_modifiers",
        rva: 0xf0_cf40,
        sentinel: [
            0x40, 0x55, 0x57, 0x41, 0x54, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0xf9, 0x48, 0x83,
            0xc1, 0x58,
        ],
    },
];

/// Receiver argument (0 = RCX, 1 = RDX): the ship, except carrier-base
/// addition's manager-first call, base destruction, medal refresh, doctrine
/// refresh and weather refresh name the ship, base, store, task force and
/// weather region respectively.
pub const RECEIVER_ARGUMENT: [usize; 8] = [0, 0, 0, 1, 0, 0, 0, 0];

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_CARRIER_BASE_RVA: usize = 0xc3_be60;

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
