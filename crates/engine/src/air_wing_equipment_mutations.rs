//! Runtime writers of the air-wing equipment pool WingEquipment records:
//! wing `+0x1D8`, with a vtable, item array `+0x20`, count `+0x2C`, and
//! 16-byte entries holding a variant pointer and amount. Observing writers
//! selects wings to reread; recording never calls them. Each recorded variant
//! handle is read from the variant fixed by the entry's pointer.
//!
//! Writer coverage includes the primary `+0x1D8` and secondary `+0x1C8` pool
//! addresses, inline data/count pairs at `+0x1F8`/`+0x204`, and all hand-offs
//! to pool primitives: variant and pool addition, variant setting, pool and
//! variant subtraction, scaling, clearing, assignment, token reading and the
//! two insertion helpers.
//! - Aircraft removal is reached from hourly air combat (through the air
//!   manager and mission helpers), accidents, land-combat support, raids,
//!   naval combat, airbase bombing and nuclear damage. Its entry belongs to
//!   [`crate::equipment_destruction::AIR`], whose detour notifies this watch.
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

/// The pool primitive the call sites in [`MUTATIONS`] invoke.
pub const SUBTRACT_VARIANT_RVA: usize = 0xff_d5a0;

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
    // Aircraft addition (wing, variant, count, ...): stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "add_airplanes",
            rva: 0xf4_5c50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Wing,
    ),
    // Equipment-pool addition (wing, pool, ...): stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "add_equipment_pool",
            rva: 0xf4_5ad0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Wing,
    ),
    // Aircraft movement (source, destination, count): copies the stack pointer
    // to R11, then stores RDX at R11+0x10; seven bytes relocate together.
    (
        MutationSite {
            name: "move_airplanes",
            rva: 0xf4_e970,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b, 0x08, 0x53, 0x55, 0x56,
                0x57, 0x41,
            ],
        },
        Receiver::Wings,
    ),
    // Equipment-pool subtraction (wing, pool): stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "subtract_equipment_pool",
            rva: 0xf5_1710,
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
            rva: 0xf5_319a,
            sentinel: [
                0xe8, 0x01, 0xa4, 0x0a, 0x00, 0x48, 0xb8, 0x09, 0xe1, 0xd1, 0xc6, 0x11, 0x6b, 0xf1,
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
            rva: 0xe5_b2d0,
            sentinel: [
                0xe8, 0xcb, 0x22, 0x1a, 0x00, 0x48, 0xb8, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00,
                0x00, 0x49,
            ],
        },
        Receiver::Pool,
    ),
    // Wing destruction (wing): stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "destroy_wing",
            rva: 0xf4_5430,
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
