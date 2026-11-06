//! The runtime writers of a province's ordered occupant lists (mixed and
//! naval data with their counts, `cprovince::OCCUPANTS` and
//! `NAVAL_OCCUPANTS`), which ProvinceUnitOrder records through
//! `cprovince::walk_unit_order`. Observing them tells the collector which
//! provinces to reread; recording never calls them.
//!
//! Writer coverage:
//! - Only append and stable compacting erase write list contents; both take
//!   the province in RCX and unit in RDX. Only unit location changes append,
//!   guarded by a nonzero new province. Erase follows location changes with a
//!   nonzero old province, game-state unit deletion, two fleet removal paths,
//!   two country transfer paths and exile and volunteer transfers and readers.
//! - Changing location removes and re-appends even when the province stays
//!   the same. A fleet merge therefore moves the surviving task force to the
//!   end of both lists without changing counts. Nothing sorts or inserts in
//!   the middle.
//! - Other writers only reserve (construction and post-read), clear (only
//!   game runtime reset from game reset or current-state destruction) or free
//!   (province destruction). Loads, resets and teardown invalidate the hour
//!   and revoke every mark's epoch.
//! - Occupant handles and classes are set at construction or load. Only the
//!   location setter writes unit location (`cunit::LOCATION`), bracketed by
//!   erase and append. An untouched occupant list therefore yields the same
//!   handles, classes and backlinks.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// The writers, in callback order: `add_unit` and `remove_unit`. Each takes
/// the province in RCX; a detour forwards RCX..R9 and returns RAX
/// unchanged.
pub const MUTATIONS: [MutationSite; 2] = [
    // The three-byte stack-pointer copy to RAX and RBX spill through it to RDX's
    // home slot relocate; RBP and RSI spills to the R8 and R9 home slots and
    // saving RDI follow.
    MutationSite {
        name: "add_unit",
        rva: Rva(0xee_b1d0),
    },
    // The five-byte RBX spill to RCX's home slot relocates. Saving RDI and
    // reserving 0x20 stack bytes follow, then zeroing EBX, retaining the
    // receiver in RDI and the start of loading the mixed list's count into ECX.
    MutationSite {
        name: "remove_unit",
        rva: Rva(0xef_10d0),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_are_distinct() {
        let [add, remove] = MUTATIONS;
        assert_ne!(add.rva, remove.rva);
    }
}
