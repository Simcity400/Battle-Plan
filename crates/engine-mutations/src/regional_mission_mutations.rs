//! The runtime writers of country air registered missions (regional vectors,
//! all-active vector and active-region ids: `cstrategic_air::REGIONAL_MISSIONS`,
//! `ALL_ACTIVE_MISSIONS`, `ACTIVE_MISSION_REGIONS`) recorded by
//! AirRegionalMissions. Observing them tells the collector which country
//! air objects to reread; recording never calls them. The row's wing is
//! resolved from the registered mission pointer in the hourly wing census.
//!
//! Writer coverage:
//! - Registration takes air, mission and region. It appends the mission pointer
//!   with eight-byte stride to the vector indexed by the region's index and the
//!   all-active vector, then sorted-inserts the region id when absent.
//! - Unregistration removes every matching mission from both vectors,
//!   shifting later ordinals down, and erases the region id when the ordinary
//!   regional vector becomes empty.
//! - Only mission region changes call either writer; other references are
//!   unwind data. The air receiver is reached through the mission's wing
//!   (`air_mission::WING`), the wing's pool (`cair_wing::POOL`) and the
//!   pool's owner (`cair_wing_pool::OWNER`); R8 names the old or new region.
//!   An unchanged region id returns without either call. Sixteen call paths
//!   include wing hourly update in strategic-air parallel workers; hourly
//!   base transfer through mission-location refresh; relocation, directly and
//!   through a helper also used by a command; daily training in parallel
//!   workers; four command paths including wing splitting; raids; paradrops;
//!   naval operations; creation; the mission save reader; and two mission
//!   teardown paths. Wing destruction tears down its embedded mission by
//!   selecting no region. Observers must therefore be allocation-free and
//!   lock-free on workers.
//! - Load and reset populate and construct every country air object and size
//!   regional vectors through initialization, idler setup and header loading.
//!   The virtual deleting wrapper reaches destruction, which clears them.
//!   The air object's save reader writes none of these three structures; its
//!   only vector append writes the pool vector (`cstrategic_air::POOLS`).
//! - Carrier combat refresh writes another air-object field and the
//!   active-region ids, never ordinary mission vectors. It appends a carrier
//!   region and rotates it into place, or pops it again when already listed.
//!   Existing ids keep their values; a new id raises the count, and growth
//!   replaces storage. An unchanged id-vector header therefore names the same
//!   ids. Carrier regions have empty ordinary vectors and produce no rows.

pub use crate::advisor_mutations::MutationSite;

/// Both sites, whose RCX is the written country air object. Neither returns a
/// value a caller reads; a detour forwards RCX..R9 and returns RAX unchanged.
pub const MUTATIONS: [MutationSite; 2] = [
    // The five-byte RBX spill to R8's home slot relocates.
    MutationSite {
        name: "register_mission",
        rva: 0xcb_a810,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x57,
        ],
    },
    // The five-byte RBX spill to R8's home slot relocates.
    MutationSite {
        name: "unregister_mission",
        rva: 0xcb_aa20,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x56,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sentinel opens with the five-byte store the hook relocates and
    /// the spill of the mission argument (RDX), which both sites share.
    #[test]
    fn sentinels_open_with_the_relocated_store_and_the_mission_spill() {
        for site in MUTATIONS {
            assert_eq!(
                site.sentinel[..5],
                [0x48, 0x89, 0x5c, 0x24, 0x18],
                "{}",
                site.name
            );
            assert_eq!(
                site.sentinel[10..15],
                [0x48, 0x89, 0x54, 0x24, 0x10],
                "{}",
                site.name
            );
        }
        assert_ne!(MUTATIONS[0].rva, MUTATIONS[1].rva);
    }
}
