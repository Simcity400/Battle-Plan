//! The runtime writers of the political-party fields PoliticalParties
//! carries between hours: id +0x8, ideology group +0x18 (its token is content),
//! tag +0x20, name key +0x28, long-name key +0x48 and default-name flag +0x68.
//! Observing them tells the collector which parties to reread; recording
//! never calls them.
//!
//! A carry still reads each country's party-array header (political status
//! +0x20, count +0x2C), every party pointer, ruling party +0xD0 and each
//! party's popularity +0x88 hourly. Daily politics rewrites every popularity
//! through the clamped setter, whose prologue cannot be detoured; effects can
//! also rewrite popularity at any hour. Party build, status clear and status
//! destruction write the array. Comparing every pointer hourly avoids hooks.
//!
//! The writer inventory covers politics and party stores, loaders of country
//! +0xF90 and its accessor, their direct callees and pointers to these fields:
//! - Only party construction writes id, ideology group and tag. It takes the
//!   party, country link and ideology, and is called only by party build at
//!   setup or dynamic reuse. It starts both names empty and the default flag
//!   at one. A party reusing a destroyed address carries that notification.
//! - Name assignment takes the party, long name and name; it clears the
//!   default flag and move-assigns both keys. Callers are the party-name and
//!   politics effects, and the civil-war, dynamic-country and peace copy
//!   path. Otherwise only the save reader and destructor touch these fields.
//!
//! ActiveIdeas records each slot's definition token from the political-status
//! idea array (+0x50, count +0x5C). The idea database fixes tokens for the
//! session. Runtime replacement has one writer reached through two thunks,
//! besides status clear, construction and destruction. A carry compares every
//! slot pointer hourly instead of hooking them.

pub use crate::advisor_mutations::MutationSite;

/// Every site; RCX is the written political party and each returns RAX
/// unchanged from its original, so a detour returns the original's value.
pub const MUTATIONS: [MutationSite; 2] = [
    // Party-name assignment (party, long name, name): RBX spill at stack +8
    // occupies the five patched bytes.
    MutationSite {
        name: "party_names",
        rva: 0x119_0ea0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    // Party construction (party, country link, ideology): RBX spill at
    // stack +0x10 occupies the five patched bytes.
    MutationSite {
        name: "construct_party",
        rva: 0x118_e330,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x57,
        ],
    },
];
