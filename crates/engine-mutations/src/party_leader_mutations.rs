//! The runtime writers of each political party's country-leader vector
//! (`cpolitical_party::COUNTRY_LEADERS`, `COUNTRY_LEADER_CAPACITY`,
//! `COUNTRY_LEADER_COUNT`) recorded by PartyLeaders. Observing them tells the
//! collector which parties to reread; recording never calls them.
//!
//! A row also records the listed leader's character id
//! (`ccountry_leader::CHARACTER` to `ccharacter::ID`), its key in that
//! character's leader map (`ccharacter::LEADER_MAP_HEAD`) and its leader id
//! (`ccountry_leader::LEADER_ID`). The character link and leader id are written only on
//! new objects by three new-object paths and the reader inside the character
//! parser; only a hooked vector insert lists those objects. The map is written
//! only by [`crate::character_role_mutations`] and [`crate::advisor_mutations`],
//! which notify the character. A carried row also needs that certificate.
//!
//! Writer coverage includes every 32-bit count store and callers of all ten
//! generic array helpers:
//! - Insert at the front follows adding a country-leader role with promotion.
//!   Appending follows role addition, leader creation and registration,
//!   registering character leader roles with parties and the daily exile
//!   path through country leader handling and role registration.
//! - Remove by value follows role removal and retirement; only it calls the
//!   index removal. First-leader retirement and death each have two
//!   ruling-party call paths. Civil war clears the vector, and post-load
//!   validation can remove entries.
//! - Promotion moves a character to the front.
//! - Civil-war transfer writes both source and destination parties; equal
//!   parties write nothing.
//! - The vector save reader takes the vector in its third argument from the
//!   party save reader. Party destruction follows political-status clear,
//!   status destruction and appearance reset. Party-array build at setup and
//!   dynamic reuse follows the status clear; it and party construction only
//!   fill new objects. A party reusing a destroyed address therefore carries
//!   that destruction notification. The collector compares each country's
//!   party-array header hourly. Array build is not hooked here because the
//!   politics family owns an interior site.
//! - Elections change popularity and the ruling party, never leader vectors.

pub use crate::advisor_mutations::MutationSite;

/// How a site's arguments name the written parties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the political party.
    Party,
    /// RCX and RDX are both written parties.
    Parties,
    /// R8 is the party's leader vector, at `VECTOR`.
    Vector,
}

/// Bytes from a party to its leader vector, the save reader's third argument.
pub const VECTOR: usize = 0x70;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 11] = [
    // Insert at the front (party, leader).
    (
        MutationSite {
            name: "insert_leader_front",
            rva: 0x1230590,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Party,
    ),
    // Append (party, leader): copy the stack pointer to R11, save RBP and
    // reserve 0x40 bytes.
    (
        MutationSite {
            name: "append_leader",
            rva: 0x1230620,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x55, 0x48, 0x83, 0xec, 0x40, 0x49, 0x89, 0x5b, 0x08, 0x48, 0x8b,
                0xe9, 0x49,
            ],
        },
        Receiver::Party,
    ),
    // Remove by value (party, leader, flag) -> bool.
    (
        MutationSite {
            name: "remove_leader",
            rva: 0x1232b80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Party,
    ),
    // Move to front (party, leader).
    (
        MutationSite {
            name: "promote_leader",
            rva: 0x12324a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Party,
    ),
    // Retire the first leader (party): save RDI and reserve 0x20 stack bytes.
    (
        MutationSite {
            name: "retire_first_leader",
            rva: 0x1232c40,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x41, 0x7c, 0x48, 0x8b, 0xf9, 0x85, 0xc0,
                0x75, 0x16,
            ],
        },
        Receiver::Party,
    ),
    // Kill the first leader (party): save RDI and reserve 0x20 stack bytes.
    (
        MutationSite {
            name: "kill_first_leader",
            rva: 0x1232410,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x20, 0x83, 0x79, 0x7c, 0x01, 0x48, 0x8b, 0xf9, 0x7c,
                0x6d, 0x48,
            ],
        },
        Receiver::Party,
    ),
    // Clear every leader (party).
    (
        MutationSite {
            name: "clear_leaders",
            rva: 0x1232230,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
                0x18, 0x4c,
            ],
        },
        Receiver::Party,
    ),
    // Civil-war transfer (source, destination, list, out, flag, flag).
    // The receiver equality test and conditional relative branch occupy nine
    // relocated bytes.
    (
        MutationSite {
            name: "transfer_leaders",
            rva: 0x1233620,
            sentinel: [
                0x48, 0x3b, 0xca, 0x0f, 0x84, 0xea, 0x08, 0x00, 0x00, 0x4c, 0x89, 0x4c, 0x24, 0x20,
                0x4c, 0x89,
            ],
        },
        Receiver::Parties,
    ),
    // Post-load party validation (party).
    (
        MutationSite {
            name: "validate_leaders",
            rva: 0x1232140,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Party,
    ),
    // Leader vector save reader (reader, token, party leader vector): copy the
    // stack pointer to RAX and spill RBX through it to RCX's home slot; seven
    // bytes relocate.
    (
        MutationSite {
            name: "read_leaders",
            rva: 0x1232590,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x18, 0x48, 0x89, 0x78,
                0x20, 0x55,
            ],
        },
        Receiver::Vector,
    ),
    // Deleting party destructor (party, flags).
    (
        MutationSite {
            name: "destroy_party",
            rva: 0x1230450,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Party,
    ),
];
