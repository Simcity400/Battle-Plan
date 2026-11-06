//! Runtime writers of the character manager's roster arrays and scientist
//! role traits. Observing them marks the roster or scientist traits as possibly
//! changed; recording never calls them.
//!
//! - The manager (`cgame_state::CHARACTER_MANAGER`, authenticated by its
//!   vtable) holds the historical and dynamic rosters
//!   (`ccharacter_manager::HISTORICAL_CHARACTERS`, `DYNAMIC_CHARACTERS`),
//!   arrays of character pointers. Only the append helper writes elements and
//!   counts; only it calls the reallocation helper. All callers insert: dynamic
//!   creation with registration or portrait (every effect, command, console and
//!   daily path creating leaders, scientists, operatives or country leaders),
//!   four historical inserts (civil war, peace conferences, unit transfer and
//!   character generation), start-character building and load reading. No
//!   erasure, swap, sort or cross-array movement occurs, so existing
//!   occurrences keep their roster and ordinal.
//! - Array clearing frees every element through role-tree destruction and
//!   zeroes the count. It runs from the manager's virtual reader, token reader
//!   and destructor. Characters are freed only with their whole array or before
//!   append.
//! - Manager construction rebuilds it on every load and game reset, from two
//!   reset paths. The advisor-pool family ([`crate::advisor_pool_mutations`])
//!   owns that site and forwards calls.
//! - Character ids (`ccharacter::ID`) are written before append, then only by
//!   identity repair/registration, forwarded by
//!   [`crate::character_role_mutations::CHARACTER_REPAIR`].
//! - Scientist traits (data, capacity and count: `cscientist::TRAIT_DATA`,
//!   `TRAIT_CAPACITY`, `TRAIT_COUNT`; and the owning character) are appended by
//!   trait addition (effect, console, pool refill), compacted by trait removal
//!   (console), and reset/refilled by token assignment (role generation,
//!   building, trait database reload). Other writers build new objects: the
//!   builder receives output, character, definition and a fourth argument
//!   (add-role effect and character definition building); attachment swaps a
//!   new scientist into `ccharacter::SCIENTIST` (creation with registration,
//!   pool refill and console); clearing removes that pointer through the
//!   remove-role path; and character parsing, merging and destruction are
//!   advisor-family sites. The role census gathers the scientist pointer afresh
//!   every hour.
//!
//! Every site takes only integer or pointer arguments and returns RAX unchanged.

pub use crate::advisor_mutations::MutationSite;

/// Which observation a site's call moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// The roster arrays.
    Roster,
    /// Scientist roles and their traits.
    Scientists,
}

pub const PUSH: usize = 0;
pub const CLEAR: usize = 1;
pub const SCIENTIST_BUILD: usize = 2;
pub const SCIENTIST_ATTACH: usize = 3;
pub const SCIENTIST_CLEAR: usize = 4;
pub const SCIENTIST_TRAIT_ADD: usize = 5;
pub const SCIENTIST_TRAIT_REMOVE: usize = 6;
pub const SCIENTIST_TRAIT_RESET: usize = 7;

pub const MUTATIONS: [(MutationSite, Domain); 8] = [
    // Roster append (array, unique pointer): the five-byte RBP store to R8's
    // home slot is patched.
    (
        MutationSite {
            name: "roster_push",
            rva: 0x6f_a550,
            sentinel: [
                0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83,
                0xec, 0x20,
            ],
        },
        Domain::Roster,
    ),
    // Roster clearing (array): stores RBP in R8's home slot.
    (
        MutationSite {
            name: "roster_clear",
            rva: 0x70_2520,
            sentinel: [
                0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x63, 0x69, 0x0c,
                0x48, 0x8b,
            ],
        },
        Domain::Roster,
    ),
    // Scientist building (out, character, definition, _): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "scientist_build",
            rva: 0x14a_7530,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x57,
            ],
        },
        Domain::Scientists,
    ),
    // Scientist attachment (character): stores RBX in RDX's home slot.
    (
        MutationSite {
            name: "scientist_attach",
            rva: 0x102_7600,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0x48,
                0x8b, 0xd1,
            ],
        },
        Domain::Scientists,
    ),
    // Scientist clearing (character): saves RBX and reserves 0x20 stack bytes.
    (
        MutationSite {
            name: "scientist_clear",
            rva: 0x102_a640,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x89, 0xc0, 0x00,
                0x00, 0x00,
            ],
        },
        Domain::Scientists,
    ),
    // Scientist trait addition (scientist, trait): stores RBX in R8's home slot.
    (
        MutationSite {
            name: "scientist_trait_add",
            rva: 0x14a_6890,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x8b, 0x51, 0x30,
                0x48, 0x8b,
            ],
        },
        Domain::Scientists,
    ),
    // Scientist trait removal (scientist, trait): reserves 0x28 stack bytes
    // and loads the trait count (`cscientist::TRAIT_COUNT`) into R10D.
    (
        MutationSite {
            name: "scientist_trait_remove",
            rva: 0x14a_8c90,
            sentinel: [
                0x48, 0x83, 0xec, 0x28, 0x44, 0x8b, 0x51, 0x3c, 0x33, 0xc0, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x48,
            ],
        },
        Domain::Scientists,
    ),
    // Scientist trait reset (scientist, tokens): copies the stack pointer to
    // RAX and stores RBX in RDX's home slot through it.
    (
        MutationSite {
            name: "scientist_trait_reset",
            rva: 0x14a_8a20,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x48, 0x89, 0x68, 0x18, 0x48, 0x89, 0x70,
                0x20, 0x57,
            ],
        },
        Domain::Scientists,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_their_indices_name_them() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|(other, _)| other.rva != site.rva && other.name != site.name)
            );
        }
        for (index, name) in [
            (PUSH, "roster_push"),
            (CLEAR, "roster_clear"),
            (SCIENTIST_BUILD, "scientist_build"),
            (SCIENTIST_ATTACH, "scientist_attach"),
            (SCIENTIST_CLEAR, "scientist_clear"),
            (SCIENTIST_TRAIT_ADD, "scientist_trait_add"),
            (SCIENTIST_TRAIT_REMOVE, "scientist_trait_remove"),
            (SCIENTIST_TRAIT_RESET, "scientist_trait_reset"),
        ] {
            assert_eq!(MUTATIONS[index].0.name, name);
        }
        // No other family patches these addresses (the shared ones are
        // forwarded instead).
        for (site, _) in &MUTATIONS {
            for other in crate::advisor_mutations::MUTATIONS
                .iter()
                .chain(&crate::character_role_mutations::MUTATIONS)
            {
                assert_ne!(site.rva, other.rva);
            }
        }
    }
}
