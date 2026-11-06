//! The runtime writers of fields recorded by States and Provinces.
//! Observing them tells the collector which states and provinces to reread;
//! recording never calls them. Every site asserts that its thread's game-state
//! access forbid count is zero.
//!
//! Provinces records id (`cprovince::ID`), state link (`cprovince::STATE`) and
//! the state's id and owner (`cstate::ID`, `cstate::OWNER`), controller
//! (`cprovince::CONTROLLER`), victory points (`VICTORY_POINTS`), the fort sum
//! from the building status (`cprovince::BUILDING_STATUS`), border-conflict set
//! size (`BORDER_CONFLICT_NONZERO_QWORD`, set `BORDER_CONFLICT_TREE`), owner
//! area (`OWNER_AREA`) and definition terrain flags and port from static map
//! content. States records id, owner, controller and template (`cstate::ID`,
//! `OWNER`, `CONTROLLER`, `TEMPLATE`) and its continent
//! (`cstate_template::CONTINENT`), custom name (`cstate::PRIMARY_DISPLAY_NAME`),
//! owner name (`cstate::SECONDARY_DISPLAY_NAME`), manpower, and movement and
//! railway infrastructure from the building status (`cstate::BUILDING_STATUS`).
//!
//! Writer coverage:
//! - Province controller: only controller-change notification writes it. Its
//!   20 callers include hourly army occupation through its helper, the state
//!   controller setter's province loop, two effects, actions, console,
//!   daily state update and province loading.
//! - Victory points: the setter from two effects and history application,
//!   and an inline store in the province token reader.
//! - Border conflicts: insertion from two combat paths and erasure from
//!   combat finalization and state reset or the active-conflict setter's
//!   tail call. Each visits provinces listed with that state by the combat
//!   side descriptor.
//! - Owner area: only the receiverless rebuild refills all provinces, from
//!   game header loading, in-game idler construction and peace finalization.
//! - Province state link: state province initialization links every province
//!   its template lists, regardless of its previous state, from four load
//!   paths and three map-editor paths. State reset unlinks its own provinces.
//! - State owner: only the owner setter, with 18 callers spanning effects,
//!   peace, border wars, releases and state loading. Only its owner-name
//!   helper writes the owner name. State reset clears owner and controller.
//! - State controller: only the controller setter, which also notifies every
//!   province through the observed province controller writer.
//! - Custom name: rename and reset effects share an interpreter entry,
//!   selecting the state from the scope; state token loading reads the
//!   string, and reset clears it.
//! - Continent is written only by template database loading; only state
//!   construction writes the template pointer. It is static per state object.
//! - Infrastructure calculations read only the infrastructure building
//!   ([`crate::building_mutations`]) and loader-written operands: a
//!   building-database field, five infrastructure-template fields (among them
//!   `cbuilding_template::TYPE_ID`, `LOOKUP_ENABLED_CANDIDATE`, `SCOPE` and
//!   `DATABASE_ORDINAL`), and two defines. A capture compares these operands
//!   hourly instead.
//! - Manpower is read hourly instead of observed: withdrawal's reinforcement
//!   path rewrites every controlled state hourly. StateFlags is also read
//!   hourly because the active-border-conflict setter has no clean hook
//!   entry. BorderConflicts rows are read hourly because the orders group's
//!   border war (`corders_group::BORDER_WAR`) and combat lifetimes name no
//!   province.
//! - The developer map editor rewrites states, names and province links
//!   inline. The building family owns and forwards its entry patch.

pub use crate::advisor_mutations::MutationSite;

/// Which rows a state site's writes reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRows {
    /// The state's own States row.
    pub state: bool,
    /// The Provinces rows of the provinces linked to the state.
    pub provinces: bool,
}

/// How a site's first argument (RCX) names what it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the province; its id is `cprovince::ID`.
    Province,
    /// RCX is the state; its id is `cstate::ID`.
    State(StateRows),
    /// The site may write any province's row.
    EveryProvince,
    /// The site may write any state's row.
    EveryState,
}

const BOTH: StateRows = StateRows {
    state: true,
    provinces: true,
};

/// Every site with its receiver. Each takes at most four integer or pointer
/// arguments and returns RAX unchanged from its original, so a detour
/// forwards RCX..R9 and returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 14] = [
    // Province controller change (province, tag, bool, bool): the R9B
    // spill to its home slot occupies five relocated bytes.
    (
        MutationSite {
            name: "province_controller",
            rva: 0xef_1860,
            sentinel: [
                0x44, 0x88, 0x4c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
                0x10, 0x55,
            ],
        },
        Receiver::Province,
    ),
    // Victory-point assignment (province, value): save RBX and reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "victory_points",
            rva: 0xef_27b0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x89, 0x51, 0x38, 0x48, 0x8b, 0xd9, 0x80, 0x3d,
                0x4e, 0xce,
            ],
        },
        Receiver::Province,
    ),
    // Province token loading (province, reader, ...): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "read_province",
            rva: 0xef_0de0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x40, 0x48,
            ],
        },
        Receiver::Province,
    ),
    // Border-conflict insertion (state, combat): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "insert_border_conflict",
            rva: 0xa1_c020,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::State(StateRows {
            state: false,
            provinces: true,
        }),
    ),
    // Border-conflict erasure (state, combat): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "erase_border_conflict",
            rva: 0xa2_7e70,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::State(StateRows {
            state: false,
            provinces: true,
        }),
    ),
    // Owner-area rebuild (no receiver): save RBX, RBP, RDI and R14.
    (
        MutationSite {
            name: "rebuild_owner_areas",
            rva: 0xd5_74c0,
            sentinel: [
                0x40, 0x53, 0x55, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x58, 0x4c, 0x89, 0x6c, 0x24,
                0x40, 0xe8,
            ],
        },
        Receiver::EveryProvince,
    ),
    // State province initialization (state): spill RBX to R8's home slot.
    // It links provinces that may have been linked to another state.
    (
        MutationSite {
            name: "initialize_provinces",
            rva: 0xa2_9aa0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::EveryProvince,
    ),
    // State owner change (state, tag, bool, bool): spill RBX to R8's home slot.
    (
        MutationSite {
            name: "state_owner",
            rva: 0xa2_9290,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10,
                0x55, 0x57,
            ],
        },
        Receiver::State(BOTH),
    ),
    // State controller change (state, tag, bool, bool): the stack-pointer
    // copy to RAX and RBX spill through it to R8's home slot occupy seven
    // relocated bytes.
    // Province controllers change through the hooked notification writer.
    (
        MutationSite {
            name: "state_controller",
            rva: 0xa2_8770,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41,
                0x54, 0x41,
            ],
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    // State reset (state): spill RBX to RDX's home slot. It clears both tags
    // and the custom name and unlinks the state's provinces.
    (
        MutationSite {
            name: "reset_state",
            rva: 0xa2_1920,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::State(BOTH),
    ),
    // State token loading (state, reader, token): the stack-pointer copy
    // to RAX and RBX spill through it to RCX's home slot occupy seven relocated
    // bytes.
    (
        MutationSite {
            name: "read_state",
            rva: 0xa2_6b30,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78,
                0x20, 0x55,
            ],
        },
        Receiver::State(BOTH),
    ),
    // State rename/reset effect interpreter entry (effect, scope): the
    // stack-pointer copy to R11 and RBX spill through it to RCX's home slot
    // occupy seven relocated bytes.
    (
        MutationSite {
            name: "state_name_effect",
            rva: 0x32_96d0,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x08, 0x49, 0x89, 0x6b, 0x18, 0x49, 0x89, 0x73,
                0x20, 0x57,
            ],
        },
        Receiver::EveryState,
    ),
    (
        MutationSite {
            name: "recount_controller_groups",
            rva: 0x00a2_a440,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    (
        MutationSite {
            name: "refresh_cached_victory_point",
            rva: 0x00a2_a1e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x7c, 0x24, 0x10, 0x8b, 0x79, 0x24, 0x33,
                0xd2, 0xbb,
            ],
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// One patch owner per native address; every site names its receiver
    /// through RCX or names none.
    #[test]
    fn sites_are_distinct_and_named_once() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            for (other, _) in &MUTATIONS[index + 1..] {
                assert_ne!(site.rva, other.rva, "{}", site.name);
                assert_ne!(site.name, other.name);
            }
        }
        // Every state site reaches at least one table.
        for (site, receiver) in MUTATIONS {
            if let Receiver::State(rows) = receiver {
                assert!(rows.state || rows.provinces, "{}", site.name);
            }
        }
    }
}
