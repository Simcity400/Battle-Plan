//! The runtime writers of a province's combat vector (`cprovince::COMBATS`),
//! which NavalProvinceCombats records through `cprovince::walk_direct_combats`.
//! Observing them tells the collector which provinces to reread; recording
//! never calls them.
//!
//! Writer coverage:
//! - Only the unique append and the swap-remove write the vector at runtime;
//!   both take the province in RCX and the combat in RDX.
//! - The append serves land and naval combat creation, joining an existing
//!   naval combat and the combat link rebuild after a load. The swap-remove
//!   is called only from combat finalization, which every removal path
//!   (including the combat manager's removal) runs before deleting the
//!   combat.
//! - Province loading and teardown write the vector too; loads, resets and
//!   teardown invalidate the hour and revoke every mark's epoch.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// The writers, in callback order: `add_combat` and `remove_combat`. Each
/// takes the province in RCX; a detour forwards RCX..R9 and returns RAX
/// unchanged.
pub const MUTATIONS: [MutationSite; 2] = [
    // The RBP spill to RCX's home slot relocates; pushing R14 and the frame
    // follow.
    MutationSite {
        name: "add_combat",
        rva: Rva(0xee_aeb0),
    },
    // Pushing RBX, the frame and the vector's data load through RCX.
    MutationSite {
        name: "remove_combat",
        rva: Rva(0xef_1040),
    },
];
