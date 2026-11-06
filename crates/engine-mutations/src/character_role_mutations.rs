//! Writers of country-leader and advisor role rows read by a character-scope
//! trait check, and of CountryLeaderRoles rows: character country-leader map
//! (`ccharacter::LEADER_MAP_HEAD`; ideology key and role pointer), role traits
//! (`ccountry_leader::TRAITS`) and current ideology
//! (`ccountry_leader::CURRENT_IDEOLOGY`), and advisor traits
//! (`cadvisor::TRAITS`). The advisor map (`ccharacter::ADVISOR_MAP_HEAD`), its
//! roles, character parsing, merging and destruction belong to
//! [`crate::advisor_mutations`]. These observe calls; recording never invokes
//! game functions to obtain data.
//!
//! - Leader traits are appended by one helper, called by generic and
//!   country-leader trait addition and both ruler/leader trait swaps. Generic
//!   and leader trait removal and both swaps compact inline. Every path then
//!   rebuilds modifiers on the same role in the same call, as do both role
//!   creators (one after storing the role's character) and the virtual name
//!   setter. The script reader runs inside the character parser, an
//!   advisor-family site.
//! - Map nodes are inserted by the shared helper (parsing, legacy leader
//!   creation, and creation with registration), and inline by the add-role
//!   effect, which builds the role from an output, character and definition in
//!   the same call. Replaced and removed roles run their destructor from role
//!   removal through range erasure, both creators, parsing and tree
//!   destruction; the role's character pointer still names the character on
//!   entry. Node keys are written only on insertion.
//! - Advisor traits are appended and compacted by trait addition and removal.
//!   Both, two factories, trait upgrading and the virtual name setter then
//!   rebuild the same role's modifiers.
//! - The advisor's cached accumulator (`cadvisor::MODIFIER_ENTRIES`) and
//!   tooltip key set (`cadvisor::TOOLTIP_KEYS`, the accumulator's string set)
//!   are cleared and refilled only by that rebuild, through accumulator
//!   clear/add and key insertion, after constructor initialization. Callers
//!   include combat-experience trait upgrading. The factory producing a new
//!   object and the name setter then assign the role's own modifier block and
//!   add it into the cached accumulator in the same call. Of callers of
//!   accumulator clear, add and both copy helpers, only those three and an
//!   unrelated unit-medal refill write an accumulator at the advisor's
//!   accumulator position (`cadvisor::MODIFIER_ENTRIES`).
//! - Current ideology is written by both creators before modifier rebuilding,
//!   the script reader inside parsing, and the ideology-setting effect. That
//!   effect stores directly on the scope country's ruling party's first leader
//!   without rebuilding. It resolves the receiver inside the effect, so the
//!   hook names no character and revokes every certificate. The ideology token
//!   is a database value.
//!
//! - Carried Characters fields are name token, template, display name, saved
//!   name and gender (`ccharacter::NAME_TOKEN_ID`, `TEMPLATE`, `NAME`, `TOKEN`,
//!   `GENDER`). Writers are identity/name setting (character-name and
//!   leader-name effects, three registration paths and four other creation/name
//!   paths), identity repair (rebuilds the display name), parsing and merging
//!   (advisor family), and definition building and constructors on new objects
//!   only. Character destruction first runs the advisor family's role-tree
//!   destructor.
//!
//! Scientist rows and operative links are read every hour and need no site.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// Index of each site in [`MUTATIONS`].
pub const COUNTRY_LEADER_MODIFIERS: usize = 0;
pub const COUNTRY_LEADER_DESTROY: usize = 1;
pub const COUNTRY_LEADER_MAP_INSERT: usize = 2;
pub const COUNTRY_LEADER_CREATE: usize = 3;
pub const ADVISOR_MODIFIERS: usize = 4;
pub const COUNTRY_LEADER_IDEOLOGY: usize = 5;
pub const CHARACTER_NAME: usize = 6;
pub const CHARACTER_REPAIR: usize = 7;

pub const MUTATIONS: [MutationSite; 8] = [
    // Leader modifier rebuilding (leader); character at `ccountry_leader::CHARACTER`.
    MutationSite {
        name: "country_leader_modifiers",
        rva: Rva(0x1050ba0),
    },
    // Leader destruction (leader); character at `ccountry_leader::CHARACTER`.
    MutationSite {
        name: "country_leader_destroy",
        rva: Rva(0x104e6e0),
    },
    // Leader-map try_emplace(map = the character's leader map, result, key, value).
    MutationSite {
        name: "country_leader_map_insert",
        rva: Rva(0x343ef0),
    },
    // Role builder (out, character, definition) of add_country_leader_role.
    MutationSite {
        name: "country_leader_create",
        rva: Rva(0x104f8d0),
    },
    // Advisor modifier rebuild (advisor); character at `cadvisor::CHARACTER`.
    MutationSite {
        name: "advisor_modifiers",
        rva: Rva(0x1334c90),
    },
    // Leader-ideology effect (effect, scope); names no character.
    MutationSite {
        name: "country_leader_ideology",
        rva: Rva(0x379e30),
    },
    // Name setter (character, name string).
    MutationSite {
        name: "character_name",
        rva: Rva(0x102a770),
    },
    // Character identity repair (character).
    MutationSite {
        name: "character_repair",
        rva: Rva(0x1028950),
    },
];
