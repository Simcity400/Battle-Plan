//! The runtime writers of a province's ordered occupant lists (mixed data
//! +0xE0/count +0xEC, naval data +0xF8/count +0x104), which ProvinceUnitOrder
//! records through `cprovince::walk_unit_order`. Observing them tells
//! the collector which provinces to reread; recording never calls them.
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
//!   location setter writes unit location +0x1F0, bracketed by erase and
//!   append. An untouched occupant list therefore yields the same handles,
//!   classes and backlinks.

pub use crate::advisor_mutations::MutationSite;

/// The writers, in callback order: `add_unit` and `remove_unit`. Each takes
/// the province in RCX; a detour forwards RCX..R9 and returns RAX
/// unchanged.
pub const MUTATIONS: [MutationSite; 2] = [
    // The three-byte stack-pointer copy to RAX and RBX spill at +0x10
    // relocate; RBP and RSI spills at +0x18/+0x20 and saving RDI follow.
    MutationSite {
        name: "add_unit",
        rva: 0xe6_5210,
        sentinel: [
            0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x48, 0x89, 0x68, 0x18, 0x48, 0x89, 0x70,
            0x20, 0x57,
        ],
    },
    // The five-byte RBX spill at stack +8 relocates. Saving RDI and
    // reserving 0x20 stack bytes follow, then zeroing EBX, retaining the
    // receiver in RDI and the start of loading the count at +0xEC into ECX.
    MutationSite {
        name: "remove_unit",
        rva: 0xe6_b110,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xdb, 0x48, 0x8b,
            0xf9, 0x8b,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_ADD_RVA: usize = 0xe6_5210;
    const EXPECTED_REMOVE_RVA: usize = 0xe6_b110;

    /// Each patch covers whole leading instructions: append's stack-pointer
    /// copy with its first spill, and erase's five-byte spill.
    #[test]
    fn the_sentinels_open_with_whole_relocated_instructions() {
        let [add, remove] = MUTATIONS;
        assert_eq!(
            add.sentinel[..7],
            [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10]
        );
        assert_eq!(remove.sentinel[..5], [0x48, 0x89, 0x5c, 0x24, 0x08]);
        assert_eq!(
            (add.rva, remove.rva),
            (EXPECTED_ADD_RVA, EXPECTED_REMOVE_RVA)
        );
    }
}
