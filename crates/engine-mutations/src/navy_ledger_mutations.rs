//! The runtime writers of a strategic navy's mission ledgers
//! (`cstrategic_navy::REGION_MISSIONS`) and registered-mission set
//! (`cstrategic_navy::REGISTERED_MISSIONS`), which NavalMissionLedgers records.
//! Observing them tells the collector which navies' ledgers to reread;
//! recording never calls them.
//!
//! Writer coverage:
//! - Registration takes the navy in RCX, the mission in RDX and the region in
//!   R8. It appends the mission to the ledger at the region's id, then to the
//!   set when absent. Mission refresh and the mission's registration virtual
//!   call it.
//! - Removal takes the navy in RCX and the mission in RDX. It removes the
//!   mission from every region ledger, then from the set. Mission refresh,
//!   mission destruction and teardown call it.
//! - Reset zeroes every ledger count and the set count together, from dynamic
//!   reuse and game reset. The world-input family patches it and forwards it.
//! - Construction builds both empty; regional growth resizes the ledger array
//!   at setup; persistent load rebuilds the navy. The naval map family patches
//!   all three. Loads also change the epoch.
//! - Convoy-escort presence rings are not ledger state: the naval mission
//!   update rewrites them, so they stay hourly reads.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// The writers, in callback order. Each takes its navy in RCX; a detour
/// forwards RCX..R9 and returns RAX unchanged.
pub const MUTATIONS: [MutationSite; 2] = [
    // The five-byte RBX spill to RCX's home slot relocates; spilling RDX,
    // saving RDI and reserving 0x20 stack bytes follow.
    MutationSite {
        name: "register_mission",
        rva: Rva(0xf1_e560),
    },
    // The five-byte RSI spill to R8's home slot relocates; spilling RDX,
    // saving RDI and reserving 0x20 stack bytes follow.
    MutationSite {
        name: "unregister_mission",
        rva: Rva(0xf2_24a0),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_are_distinct_and_the_reset_belongs_to_world_input() {
        assert_ne!(MUTATIONS[0].rva, MUTATIONS[1].rva);
        let (reset, _) = crate::world_input_mutations::MUTATIONS
            .iter()
            .find(|(site, _)| site.name == "reset_navy")
            .unwrap();
        for site in MUTATIONS {
            assert_ne!(site.rva, reset.rva, "{}", site.name);
        }
    }
}
