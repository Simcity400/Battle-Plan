//! Runtime writers of every field StateBuildings and ProvinceBuildings
//! record from a building status (`cstate::BUILDING_STATUS`,
//! `cprovince::BUILDING_STATUS`) and its buildings. Observing them selects
//! statuses to reread; recording never calls them.
//!
//! Rows record status flags, location id and country
//! (`cbuilding_status::FLAGS`, `ID`, `COUNTRY`); building position in the
//! building vector (`cbuilding_status::BUILDINGS`) and membership in the active
//! vector (`ACTIVE_BUILDINGS`); type, level, healthy level and partial health
//! (`cbuilding::TYPE_ID`, `LEVEL`, `HEALTHY_LEVEL`, `PARTIAL_HEALTH`). The walk
//! also authenticates the status backlink (`cbuilding::BUILDING_STATUS`),
//! template (`cbuilding::TEMPLATE`) and index vector
//! (`cbuilding_status::BUILDING_INDICES`). Recorded values are not derived from
//! another object. Outside load or reset, location tables, ids and province
//! state links change only in nudge mode, whose entry is also a site. An hour
//! without any site leaves every walked location, status and building
//! unchanged.
//!
//! - Only level setting writes the level at runtime: construction completion,
//!   daily production, instant builds, owner changes, railways, effects,
//!   commands and peace. It also writes healthy level and partial health and
//!   rebuilds the active vector through two helpers on a zero crossing. Other
//!   writers of health are the damage step (only from damage application:
//!   bombing, naval air, land fort damage and scripted damage), repair
//!   advancement (construction and console), and healthy-level destruction
//!   (scorching and two nuclear paths). Each receives the building in RCX; its
//!   status is its backlink.
//! - Only building lookup/creation writes the building and index vectors at
//!   runtime. With the status in RCX, it alone calls building construction,
//!   setting type, backlink and template and zeroing levels. Its callers use
//!   the result only through the hooked setters.
//! - The status save reader receives the status in RCX, creates buildings and
//!   tail-calls their readers and clamp. Status clearing, also with status in
//!   RCX, empties it on country-appearance reset and three other reset paths.
//!   Status construction and destruction occur only with the state or province,
//!   which loading or nudge mode replaces.
//! - Identity is written only on construction: the status constructor stores
//!   location id and country zero; the owner's constructor sets scope flags
//!   in place (state 2, both province constructors 5). State owner and controller
//!   setters write none of these fields.
//! - Index initialization fills the index vector and count only when count is
//!   zero, after construction or hooked clearing. It runs during
//!   country-appearance reset for the country's own status (never walked),
//!   state building-status reset after hooked clearing on game reset, and state
//!   index initialization from game reset and nudge mode.
//! - Only hooked sites and destruction write building and active-vector headers.
//!   Active-vector rebuilding runs inside hooked level setting through its
//!   helper, or at the end of the hooked reader's block on load, through a
//!   slot-8 persistence-reader thunk.
//! - Only load, reset and nudge mode write location structure. The game-state
//!   state table (`cgame_state::STATES`) is written by construction,
//!   destruction, reset and the nudger's state rebuild; its province table
//!   (`cgame_state::PROVINCES`) by construction, destruction and load-only
//!   province initialization. Constructors write ids. The province state link
//!   (`cprovince::STATE`) is written by both province constructors, state
//!   province initialization (load, two reset paths and nudge mode), and state
//!   teardown (game reset, runtime reset and nudge mode).
//! - No building writer runs on a parallel worker thread.
//! - Nudge mode rewrites statuses and locations without a hooked writer. Its
//!   handlers rebuild a state in place (destruction, construction at the same
//!   address, index initialization), create states with province initialization,
//!   and move provinces through teardown and two input-handler initialization
//!   paths. They run only in nudge mode. The game idler enters it by constructing
//!   the nudge idler, the constructor's only caller, with five arguments. This
//!   eighth site ([`Receiver::Structure`]) names no status. Subsequent unobserved
//!   writes can happen at any moment, so entry ends reuse for the attachment.

pub use crate::advisor_mutations::MutationSite;

/// How a site's first argument (RCX) names the status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is a building; its status is the backlink `cbuilding::BUILDING_STATUS`.
    Building,
    /// RCX is the building status.
    Status,
    /// The site names no status: it opens a path that may rewrite any status
    /// and what locates it, later and unobserved (nudge mode).
    Structure,
}

const SET_LEVEL_RVA: usize = 0x1163eb0;

/// Every site with its receiver; each returns
/// RAX unchanged from its original, so a detour returns the original's
/// value.
pub const MUTATIONS: [(MutationSite, Receiver); 8] = [
    // Level setting (building, level, notify) returns a boolean.
    (
        MutationSite {
            name: "set_level",
            rva: SET_LEVEL_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41,
                0x54, 0x41,
            ],
        },
        Receiver::Building,
    ),
    // Damage step (building, out, amount, flag, cause, flag): the five-byte
    // saves of RBX/RBP/RSI/RDI relocate.
    (
        MutationSite {
            name: "apply_damage_step",
            rva: 0x1163d00,
            sentinel: [
                0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x4c,
            ],
        },
        Receiver::Building,
    ),
    // Repair advancement (building, out, progress, flag).
    (
        MutationSite {
            name: "advance_repair",
            rva: 0x1162990,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40,
                0x45, 0x0f,
            ],
        },
        Receiver::Building,
    ),
    // Healthy-level destruction (building, out, levels, flag, source).
    (
        MutationSite {
            name: "destroy_healthy_levels",
            rva: 0x1163a20,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x48, 0x83,
                0xec, 0x50,
            ],
        },
        Receiver::Building,
    ),
    // Building lookup/creation (status, template) returns a building pointer.
    (
        MutationSite {
            name: "get_or_create_building",
            rva: 0x11ffa20,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Status,
    ),
    // Building-status save reading (status, reader).
    (
        MutationSite {
            name: "read_building_status",
            rva: 0x1205610,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x40, 0x48,
            ],
        },
        Receiver::Status,
    ),
    // Building-status clearing (status).
    (
        MutationSite {
            name: "clear_building_status",
            rva: 0x1201640,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Status,
    ),
    // Nudge-mode entry constructs an idler with five arguments and returns it.
    // Four argument-register stores to shadow space open the function; the
    // fifth argument is read from its stack slot through RBP.
    (
        MutationSite {
            name: "enter_nudge_mode",
            rva: 0x1363e90,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x48,
            ],
        },
        Receiver::Structure,
    ),
];
