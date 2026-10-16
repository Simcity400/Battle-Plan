//! Runtime writers of role-census fields no other family patches: a rostered
//! character's country (`ccharacter::COUNTRY`) and role span, from
//! `ccharacter::LEADER_MAP_HEAD` through `ccharacter::OPERATIVE` (country-leader
//! map head and size, unit leader pointer, advisor map head and size, scientist
//! pointer and operative reference). These and the forwarded sites select when
//! the census can be carried; recording never calls them.
//!
//! - Country assignment updates the unit leader too, reached from recruitment,
//!   registration, both generated-advisor branches, country-character loading
//!   and retirement. The character parser stores the country through a helper.
//! - Unit-leader pointers (`ccharacter::UNIT_LEADER`) are written by army- and
//!   navy-role creation, and cleared by the remove-unit-leader and
//!   remove-military-role effects. Role copying runs inside character merge or
//!   on a new character.
//! - Country-leader map (head and size) insertion is inline in the add-role
//!   effect; range erasure is reached from role removal. The try-emplace site
//!   belongs to [`crate::character_role_mutations`].
//! - Scientist pointers (`ccharacter::SCIENTIST`) are written by the add-role
//!   effect; attach and clear belong to [`crate::character_roster_mutations`].
//! - The operative reference (`ccharacter::OPERATIVE`) has one setter and one
//!   caller, which publishes the character first.
//! - Advisor map (head and size) writers, parsing, merging and destruction belong
//!   to [`crate::advisor_mutations`]. Constructors and definition building write
//!   new characters before roster publication, which the roster domain observes.
//!   Shared tree insertion and extraction helpers run for unrelated maps every
//!   hour; the boundaries above enclose their calls on character maps. Writes
//!   through other aliases are not proved closed; sampled slices compare the
//!   gathered rows.

pub use crate::advisor_mutations::MutationSite;

const SET_COUNTRY_RVA: usize = 0xf9_0ff0;

/// The writers, in callback order. Each takes at most three integer or
/// pointer arguments; a detour forwards RCX..R9 and returns RAX unchanged.
pub const MUTATIONS: [MutationSite; 8] = [
    site(
        "set_country",
        SET_COUNTRY_RVA,
        [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x02, 0x48, 0x8d,
            0x99, 0xa8,
        ],
    ),
    site("create_army_role", 0xf8_81e0, CREATE_ROLE),
    site("create_navy_role", 0xf8_8600, CREATE_ROLE),
    // The RBX save and 0x20-byte stack reservation relocate in six bytes.
    site(
        "clear_unit_role",
        0xf9_0f60,
        [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x89, 0xa8, 0x00,
            0x00, 0x00,
        ],
    ),
    site(
        "add_country_leader_role",
        0x34_8c50,
        [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    ),
    site(
        "erase_leader_roles",
        0x3b_9bc0,
        [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x41, 0x56, 0x41,
            0x57, 0x48,
        ],
    ),
    site(
        "add_scientist_role",
        0x34_b9b0,
        [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec,
            0x50, 0x01,
        ],
    ),
    // Loads the reference held by RDX's object into RAX and stores it in the
    // character's operative reference; eleven bytes relocate before the return
    // and padding.
    site(
        "set_operative_reference",
        0xf9_11a0,
        [
            0x48, 0x8b, 0x42, 0x08, 0x48, 0x89, 0x81, 0xc8, 0x00, 0x00, 0x00, 0xc3, 0xcc, 0xcc,
            0xcc, 0xcc,
        ],
    ),
];

/// Both unit-role creators share a prologue whose five-byte RBX store to RDX's
/// home slot relocates.
const CREATE_ROLE: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x8b,
];

const fn site(name: &'static str, rva: usize, sentinel: [u8; 16]) -> MutationSite {
    MutationSite {
        name,
        rva,
        sentinel,
    }
}
