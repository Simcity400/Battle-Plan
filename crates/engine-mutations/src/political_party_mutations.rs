//! The runtime writers of the political-party fields PoliticalParties
//! carries between hours: id, ideology group (its token is content), tag, name
//! key, long-name key and default-name flag (`cpolitical_party::ID`,
//! `IDEOLOGY_GROUP`, `TAG`, `NAME`, `LONG_NAME`, `NAME_DEFAULT`).
//! Observing them tells the collector which parties to reread; recording
//! never calls them.
//!
//! A carry still reads each country's party-array header
//! (`cpolitical_status::PARTIES`, `PARTY_COUNT`), every party pointer,
//! ruling party (`cpolitical_status::RULING_PARTY`) and each party's
//! popularity (`cpolitical_party::POPULARITY`) hourly. Daily politics
//! rewrites every popularity through the clamped setter, whose prologue
//! cannot be detoured; effects can also rewrite popularity at any hour.
//! Party build, status clear and status destruction write the array.
//! Comparing every pointer hourly avoids hooks.
//!
//! The writer inventory covers politics and party stores, loaders of the
//! country's political status (`ccountry::POLITICAL_STATUS`) and its
//! accessor, their direct callees and pointers to these fields:
//! - Only party construction writes id, ideology group and tag. It takes the
//!   party, country link and ideology, and is called only by party build at
//!   setup or dynamic reuse. It starts both names empty and the default flag
//!   at one. A party reusing a destroyed address carries that notification.
//! - Name assignment takes the party, long name and name; it clears the
//!   default flag and move-assigns both keys. Callers are the party-name and
//!   politics effects, and the civil-war, dynamic-country and peace copy
//!   path. Otherwise only the save reader and destructor touch these fields.
//!
//! ActiveIdeas records each slot's definition token from the
//! political-status idea array (`cpolitical_status::IDEAS`, count
//! `IDEA_COUNT`). The idea database fixes tokens for the session. Runtime
//! replacement has one writer reached through two thunks, besides status
//! clear, construction and destruction. A carry compares every slot pointer
//! hourly instead of hooking them.

pub use crate::advisor_mutations::MutationSite;

/// Every site; RCX is the written political party and each returns RAX
/// unchanged from its original, so a detour returns the original's value.
pub const MUTATIONS: [MutationSite; 2] = [
    // Party-name assignment (party, long name, name): RBX spill to RCX's
    // home slot occupies the five patched bytes.
    MutationSite {
        name: "party_names",
        rva: 0x123_2e00,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    // Party construction (party, country link, ideology): RBX spill to
    // RDX's home slot occupies the five patched bytes.
    MutationSite {
        name: "construct_party",
        rva: 0x123_0290,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x57,
        ],
    },
];
