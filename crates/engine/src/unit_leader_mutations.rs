//! The runtime writers of UnitLeaders fields ([`crate::cunit_leader::read`],
//! `read_text`, `read_codename` and `read_formation_reference`). Observing
//! them tells the collector which leaders to reread; recording never calls
//! them.
//!
//! Writer coverage:
//! - Formation reference +0x1048 is written by formation assignment from
//!   orders-group leader selection, container repair and faction commander
//!   handling. Saves retain +0x1050; load rebuilds +0x1048 through repair.
//! - Skill definition +0xE60 changes through level selection. Status, valid
//!   byte, level, required experience and category belong to the shared skill
//!   definition, written only at construction and database load. Experience
//!   addition changes unrecorded +0xE68 and reaches level-up only past its
//!   threshold.
//! - Category +0xE7C has its own setter.
//! - Army attack, defense, planning and logistics components at +0xF58,
//!   +0xF68, +0xF78 and +0xF88 have four setters. Navy attack, defense,
//!   maneuvering and coordination at +0xF68, +0xF78, +0xF88 and +0xF98 have
//!   four setters. Level-ups, traits, skill-add effects and loads call them.
//! - Army captured flag +0x1059 is written by capture and release.
//! - Owner +0x120 is written by the nine-byte ordinary setter and operative
//!   override, virtual slot 21.
//! - Exile country +0xED4 is written by daily diplomacy's exile leader
//!   generation, resolving the leader from its newly created character, and
//!   the remove-exile effect, resolving a leader from event scope. Neither
//!   names a leader at entry.
//! - Inherited texts +0x20 and +0x40 change through name setting's string
//!   moves and derived-state rebuild, virtual slot 8, which copies +0x20 to
//!   +0x40. Operative rebuild calls that first before renaming.
//! - Character linking writes backlink +0xF48 and template visibility +0xF50.
//! - Operative codename +0xFD0, a naming-group member, changes through the
//!   rename wrapper (called only by the codename command), admission wrapper
//!   (from both agency operative-add paths) and operative rebuild.
//! - Construction, copy construction and destruction belong to
//!   [`crate::naval_tree_mutations`], which forwards them; reused addresses
//!   cannot keep freed leaders' certificates. All other stores to these
//!   fields are in construction, cloning or save loading. Character-template
//!   building writes the visibility choice +0x18 only for character-definition
//!   building. Load changes the epoch.

pub use crate::advisor_mutations::MutationSite;

const SET_NAME_RVA: usize = 0xc1_6fb0;

/// The writers, in callback order. Each takes its leader in RCX, except the
/// two exile sites, which name none. A detour forwards RCX..R9 and returns
/// RAX unchanged.
pub const MUTATIONS: [MutationSite; 23] = [
    // The five-byte R8B spill at stack +0x18 relocates.
    site(
        "formation",
        0xc1_7240,
        [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x53, 0x55, 0x57, 0x48, 0x83, 0xec, 0x50, 0x4c, 0x89,
            0x64, 0x24,
        ],
    ),
    site(
        "skill",
        0xc1_7c50,
        [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    ),
    site(
        "category",
        0xc1_0d60,
        [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x81, 0xec, 0x90, 0x01, 0x00, 0x00, 0x41,
            0x8b, 0xf8,
        ],
    ),
    site("army_attack", 0xc1_51e0, SETTER),
    site("army_defense", 0xc1_6020, SETTER),
    site("army_planning", 0xc1_76f0, SETTER),
    site("army_logistics", 0xc1_6b70, SETTER),
    site("navy_attack", 0xc1_5310, SETTER),
    site("navy_defense", 0xc1_6150, SETTER),
    site("navy_maneuvering", 0xc1_6ca0, SETTER),
    site("navy_coordination", 0xc1_5e50, SETTER),
    site(
        "capture",
        0xc0_2810,
        [
            0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x53, 0x57, 0x41,
            0x57, 0x48,
        ],
    ),
    site(
        "release",
        0xc1_3a20,
        [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec,
            0x30, 0x8b,
        ],
    ),
    // Reading the tag through RDX into EAX and storing it at receiver
    // +0x120 occupy eight relocated bytes before the return and padding.
    site(
        "owner",
        0xc1_6010,
        [
            0x8b, 0x02, 0x89, 0x81, 0x20, 0x01, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
            0xcc, 0xcc,
        ],
    ),
    site(
        "operative_owner",
        0xc1_5f80,
        [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x02, 0x48, 0x8b,
            0xd9, 0x89,
        ],
    ),
    site(
        "exile_generation",
        0x6d_9870,
        [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x56, 0x57, 0x41, 0x54,
            0x41, 0x56,
        ],
    ),
    // Reserving 0x28 stack bytes and copying RDX to RCX occupy seven relocated bytes.
    site(
        "exile_removal",
        0x2e_7ae0,
        [
            0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0xca, 0xe8, 0x04, 0x1a, 0x24, 0x00, 0x48, 0x85,
            0xc0, 0x74,
        ],
    ),
    site(
        "set_name",
        SET_NAME_RVA,
        [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x57,
        ],
    ),
    // Saving RBX and reserving 0x40 stack bytes occupy six relocated bytes.
    site(
        "rebuild_derived",
        0xc0_bc70,
        [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x40, 0x48, 0x83, 0x79, 0x38, 0x0f, 0x48, 0x8d, 0x51,
            0x20, 0x48,
        ],
    ),
    // Saving RBX and reserving 0x20 stack bytes occupy six relocated bytes
    // before the derived-state rebuild call.
    site(
        "operative_rebuild",
        0xc0_bb50,
        [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x12, 0x01, 0x00, 0x00,
            0x48, 0x83,
        ],
    ),
    site(
        "link_character",
        0xc1_5710,
        [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x05, 0x77,
            0xee, 0x70,
        ],
    ),
    site(
        "operative_codename",
        0xc1_57f0,
        [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    ),
    // Reserving 0x28 stack bytes and testing the receiver's qword at +0x1040
    // against zero occupy twelve relocated bytes.
    site(
        "operative_admission",
        0xc0_3eb0,
        [
            0x48, 0x83, 0xec, 0x28, 0x48, 0x83, 0xb9, 0x40, 0x10, 0x00, 0x00, 0x00, 0x0f, 0x85,
            0xed, 0x00,
        ],
    ),
];

/// Index of each site in [`MUTATIONS`] that names no leader.
pub const EXILE_GENERATION: usize = 15;
pub const EXILE_REMOVAL: usize = 16;

/// The eight component setters share a five-byte relocated RBX spill at stack +8.
const SETTER: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];

const fn site(name: &'static str, rva: usize, sentinel: [u8; 16]) -> MutationSite {
    MutationSite {
        name,
        rva,
        sentinel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_the_unnamed_indices_name_the_exile_sites() {
        for (index, site) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|other| other.rva != site.rva && other.name != site.name),
                "{}",
                site.name
            );
        }
        assert_eq!(MUTATIONS[EXILE_GENERATION].name, "exile_generation");
        assert_eq!(MUTATIONS[EXILE_REMOVAL].name, "exile_removal");
    }
}
