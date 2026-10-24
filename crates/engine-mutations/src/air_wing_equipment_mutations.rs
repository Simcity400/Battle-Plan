//! Runtime writers of the air-wing equipment pool WingEquipment records: the
//! wing's embedded pool (`POOL`), with a vtable, item array and count
//! (`cequipment_variant_pool::ITEMS`, `COUNT`), and 16-byte entries holding a
//! variant pointer and amount. Observing writers selects wings to reread;
//! recording never calls them. Each recorded variant handle is read from the
//! variant fixed by the entry's pointer.
//!
//! Writer coverage includes the primary and secondary pool addresses, the
//! wing's inline equipment-bank data/count pair (`cair_wing::EQUIPMENT_BANK`,
//! `EQUIPMENT_BANK_COUNT`), and all hand-offs to pool primitives: variant and
//! pool addition, variant setting, pool and variant subtraction, scaling,
//! clearing, assignment, token reading and the two insertion helpers.
//! - Aircraft removal is reached from hourly air combat (through the air
//!   manager and mission helpers), accidents, land-combat support, raids,
//!   naval combat, airbase bombing and nuclear damage. Its entry belongs to
//!   `equipment_destruction::AIR`, whose detour notifies this watch.
//!   There is one patch per site, so it is absent from [`MUTATIONS`].
//! - Reinforcement and upgrades reach aircraft addition through reinforcement
//!   and variant upgrades, from daily deployment and rockets.
//! - Pool addition receives destinations for deployment into a wing,
//!   equipment movement and consolidation. Pool subtraction receives their
//!   sources and moves to reserves. Aircraft movement transfers a count from
//!   source to destination for wing splitting.
//! - Hourly over-capacity trimming and daily plane return call variant
//!   subtraction on the wing pool at their two sites below. Other callers of
//!   that primitive pass other pools.
//! - Every removal ends in wing destruction. Three constructors build new
//!   wings; an address reused after destruction retains its notification.
//!   A fourth constructor and the wing save reader load a wing from a save.
//! - The base tick, wing transfer to a base, manpower updates, six other wing
//!   helpers, and the copies made by lend-lease, capture and equipment-addition
//!   effects into stockpiles or their own pools only read the watched pool.

pub use crate::advisor_mutations::MutationSite;

/// Bytes from a wing to its equipment pool.
pub const POOL: usize = 0x1d8;

/// Inline fields use their own receiver keys, never the equipment-pool key.
pub const TRAINING_STOP: usize = 0x10c;
pub const ATTACHMENT_COVERAGE: usize = 0x9c0;
pub const MISSION: usize = 0x90;
pub const RAID_REFERENCE: usize = engine_native::cair_wing::transport_operands::RAID_REFERENCE;
/// A disjoint inline lifetime key within an observed raid object.
pub const RAID_LIFETIME_KEY: usize = 1;

/// Field writers and the pointed raid's lifetime. Construction/loading is
/// covered by lifetime admission. Coverage names the wing in R8; the mission
/// setter names its embedded mission in RCX; the other two name RCX directly.
pub const MAP_MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "wing_training_stop",
        rva: 0xf8_56b0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24,
            0x18, 0x89,
        ],
    },
    MutationSite {
        name: "wing_attachment_coverage",
        rva: 0xbf_2d00,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x57,
        ],
    },
    MutationSite {
        name: "wing_raid_reference",
        rva: 0xf6_7250,
        sentinel: [
            0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x05, 0xcb, 0x62, 0x3d, 0x02, 0x48,
            0x8b, 0xf9,
        ],
    },
    MutationSite {
        name: "raid_reference_lifetime",
        rva: 0xfe_9f90,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x8b,
        ],
    },
    MutationSite {
        name: "append_incoming_air_wing",
        rva: 0xc4_bd80,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x30, 0x80,
        ],
    },
    MutationSite {
        name: "erase_incoming_air_wing",
        rva: 0xc6_36f0,
        sentinel: [
            0x48, 0x83, 0xec, 0x08, 0x48, 0x63, 0x41, 0x24, 0x45, 0x33, 0xc9, 0x4c, 0x8b, 0xd9,
            0x85, 0xc0,
        ],
    },
];

/// The pool primitive the call sites in [`MUTATIONS`] invoke.
pub const SUBTRACT_VARIANT_RVA: usize = 0x101_2850;

/// How a site's arguments name the written pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the air wing.
    Wing,
    /// RCX and RDX are both written wings.
    Wings,
    /// RCX is the pool, at a replaced CALL of [`SUBTRACT_VARIANT_RVA`].
    Pool,
}

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 7] = [
    // Aircraft addition (wing, variant, count, ...): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "add_airplanes",
            rva: 0xf5_add0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Wing,
    ),
    // Equipment-pool addition (wing, pool, ...): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "add_equipment_pool",
            rva: 0xf5_ac50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Wing,
    ),
    // Aircraft movement (source, destination, count): copies the stack pointer
    // to R11, then stores RDX in its home slot; seven bytes relocate together.
    (
        MutationSite {
            name: "move_airplanes",
            rva: 0xf6_3af0,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b, 0x08, 0x53, 0x55, 0x56,
                0x57, 0x41,
            ],
        },
        Receiver::Wings,
    ),
    // Equipment-pool subtraction (wing, pool): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "subtract_equipment_pool",
            rva: 0xf6_6890,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Receiver::Wing,
    ),
    // Over-capacity trim calls variant subtraction (pool, variant, amount);
    // execution resumes immediately after that call.
    (
        MutationSite {
            name: "trim_airplanes",
            rva: 0xf6_831a,
            sentinel: [
                0xe8, 0x31, 0xa5, 0x0a, 0x00, 0x48, 0xb8, 0x09, 0xe1, 0xd1, 0xc6, 0x11, 0x6b, 0xf1,
                0x29, 0x49,
            ],
        },
        Receiver::Pool,
    ),
    // Daily plane return calls variant subtraction (pool, variant, amount);
    // execution resumes immediately after that call.
    (
        MutationSite {
            name: "return_airplanes",
            rva: 0xe6_fbd0,
            sentinel: [
                0xe8, 0x7b, 0x2c, 0x1a, 0x00, 0x48, 0xb8, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00,
                0x00, 0x49,
            ],
        },
        Receiver::Pool,
    ),
    // Wing destruction (wing): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "destroy_wing",
            rva: 0xf5_a5b0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Wing,
    ),
];

/// The pools a site's first two arguments name; the second is zero unless
/// the site writes two wings.
pub fn pools(receiver: Receiver, first: usize, second: usize) -> [usize; 2] {
    match receiver {
        Receiver::Wing => [first.wrapping_add(POOL), 0],
        Receiver::Wings => [first.wrapping_add(POOL), second.wrapping_add(POOL)],
        Receiver::Pool => [first, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each call site's sentinel starts with its rel32 CALL of the primitive.
    #[test]
    fn call_sites_call_the_subtraction() {
        for (site, receiver) in MUTATIONS {
            if receiver != Receiver::Pool {
                continue;
            }
            assert_eq!(site.sentinel[0], 0xe8, "{}", site.name);
            let displacement = i32::from_le_bytes(site.sentinel[1..5].try_into().unwrap());
            let target = (site.rva + 5).wrapping_add_signed(displacement as isize);
            assert_eq!(target, SUBTRACT_VARIANT_RVA, "{}", site.name);
        }
    }

    #[test]
    fn receivers_name_the_pools() {
        assert_eq!(pools(Receiver::Wing, 0x5000, 7), [0x51d8, 0]);
        assert_eq!(pools(Receiver::Wings, 0x5000, 0x6000), [0x51d8, 0x61d8]);
        assert_eq!(pools(Receiver::Pool, 0x51d8, 0), [0x51d8, 0]);
    }
}
