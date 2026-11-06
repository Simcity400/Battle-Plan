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
use engine_sites::Rva;

const SET_COUNTRY_RVA: Rva = Rva(0x102_a700);

/// The writers, in callback order. Each takes at most three integer or
/// pointer arguments; a detour forwards RCX..R9 and returns RAX unchanged.
pub const MUTATIONS: [MutationSite; 8] = [
    site("set_country", SET_COUNTRY_RVA),
    site("create_army_role", Rva(0x102_18f0)),
    site("create_navy_role", Rva(0x102_1d10)),
    // The RBX save and 0x20-byte stack reservation relocate in six bytes.
    site("clear_unit_role", Rva(0x102_a670)),
    site("add_country_leader_role", Rva(0x36_18c0)),
    site("erase_leader_roles", Rva(0x3d_2830)),
    site("add_scientist_role", Rva(0x36_4610)),
    // Loads the reference held by RDX's object into RAX and stores it in the
    // character's operative reference; eleven bytes relocate before the return
    // and padding.
    site("set_operative_reference", Rva(0x102_a8b0)),
];

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}
