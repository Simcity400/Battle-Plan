//! The runtime writers of UnitLeaders fields (`cunit_leader::read`,
//! `read_text`, `read_codename` and `read_formation_reference`). Observing
//! them tells the collector which leaders to reread; recording never calls
//! them.
//!
//! Writer coverage:
//! - Formation reference (`carmy_leader::FORMATION_REFERENCE`) is written by
//!   formation assignment from orders-group leader selection, container
//!   repair and faction commander handling. Saves retain a separate field
//!   beside it; load rebuilds the formation reference through repair.
//! - Skill definition (`cunit_leader::SKILL`) changes through level
//!   selection. Status, valid byte, level, required experience and category
//!   belong to the shared skill definition, written only at construction and
//!   database load. Residual experience changes through ordinary and unchecked
//!   additions and the daily progress update. Trait progress changes through
//!   addition, daily update, removal, eligibility pruning and completion.
//!   These writers share the leader certificate; loads start a new epoch.
//! - Category (`cunit_leader::CATEGORY`) has its own setter.
//! - Army attack, defense, planning and logistics components
//!   (`carmy_leader::ATTACK`, `DEFENSE`, `PLANNING`, `LOGISTICS`) have four
//!   setters. Navy attack, defense, maneuvering and coordination have four
//!   setters. Level-ups, traits, skill-add effects and loads call them.
//! - Army captured flag (`carmy_leader::CAPTURED`) is written by capture and
//!   release.
//! - Owner (`cunit_leader::OWNER`) is written by the nine-byte ordinary
//!   setter and operative override, virtual slot 21.
//! - Exile country (`cunit_leader::EXILE_COUNTRY`) is written by daily
//!   diplomacy's exile leader generation, resolving the leader from its newly
//!   created character, and the remove-exile effect, resolving a leader from
//!   event scope. Neither names a leader at entry.
//! - Inherited texts (`cunit_leader::TOKEN`, `NAME`) change through name
//!   setting's string moves and derived-state rebuild, virtual slot 8, which
//!   copies the token text into the name. Operative rebuild calls that first
//!   before renaming.
//! - Character linking writes the backlink (`cunit_leader::CHARACTER_TYPE`,
//!   `CHARACTER_ID`) and template visibility
//!   (`cunit_leader::TEMPLATE_VISIBILITY`).
//! - Operative codename (`coperative_leader::CODENAME`), a naming-group
//!   member, changes through the rename wrapper (called only by the codename
//!   command), admission wrapper (from both agency operative-add paths) and
//!   operative rebuild.
//! - Construction, copy construction and destruction belong to
//!   [`crate::naval_tree_mutations`], which forwards them; reused addresses
//!   cannot keep freed leaders' certificates. All other stores to these
//!   fields are in construction, cloning or save loading. Character-template
//!   building writes the visibility choice
//!   (`cunit_leader::VISIBILITY_CHOICE`) only for character-definition
//!   building. Load changes the epoch.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

const SET_NAME_RVA: Rva = Rva(0xc7_cc10);

/// The writers, in callback order. Each takes its leader in RCX, except the
/// two exile sites, which name none. A detour forwards RCX..R9 and returns
/// RAX unchanged.
const EXPERIENCE_ADD_UNCHECKED_RVA: Rva = Rva(0xc64da0);
const DAILY_PROGRESS_RVA: Rva = Rva(0xc75a30);
const TRAIT_PROGRESS_ADD_RVA: Rva = Rva(0xc6a800);
const TRAIT_PROGRESS_REMOVE_RVA: Rva = Rva(0xc6aae0);
const TRAIT_PROGRESS_PRUNE_RVA: Rva = Rva(0xc786e0);
const TRAIT_FACTOR_SET_RVA: Rva = Rva(0xc647e0);
const TRAIT_FACTOR_REMOVE_RVA: Rva = Rva(0xc790e0);
const TRAIT_PROGRESS_COMPLETE_RVA: Rva = Rva(0xc743b0);

pub const MUTATIONS: [MutationSite; 32] = [
    // The five-byte RBX spill to RDX's home slot relocates.
    site("formation", Rva(0xc7_cea0)),
    site("skill", Rva(0xc7_d870)),
    site("category", Rva(0xc7_62e0)),
    site("army_attack", Rva(0xc7_ae90)),
    site("army_defense", Rva(0xc7_bcd0)),
    site("army_planning", Rva(0xc7_d310)),
    site("army_logistics", Rva(0xc7_c7d0)),
    site("navy_attack", Rva(0xc7_afc0)),
    site("navy_defense", Rva(0xc7_be00)),
    site("navy_maneuvering", Rva(0xc7_c900)),
    site("navy_coordination", Rva(0xc7_bb00)),
    site("capture", Rva(0xc6_7900)),
    site("release", Rva(0xc7_8f60)),
    // Reading the tag through RDX into EAX and storing it in the receiver's owner
    // (`cunit_leader::OWNER`) occupy eight relocated bytes before the return and
    // padding.
    site("owner", Rva(0xc7_bcc0)),
    site("operative_owner", Rva(0xc7_bc30)),
    site("exile_generation", Rva(0x72_dfa0)),
    // Reserving 0x28 stack bytes and copying RDX to RCX occupy seven relocated bytes.
    site("exile_removal", Rva(0x30_05e0)),
    site("set_name", SET_NAME_RVA),
    // Saving RBX and reserving 0x40 stack bytes occupy six relocated bytes.
    site("rebuild_derived", Rva(0xc7_1100)),
    // Saving RBX and reserving 0x20 stack bytes occupy six relocated bytes
    // before the derived-state rebuild call.
    site("operative_rebuild", Rva(0xc7_0fe0)),
    site("link_character", Rva(0xc7_b3c0)),
    site("operative_codename", Rva(0xc7_b4a0)),
    // Reserving 0x28 stack bytes and testing a receiver qword against zero
    // occupy twelve relocated bytes.
    site("operative_admission", Rva(0xc6_9030)),
    site(
        "experience_add",
        engine_native::cunit_leader::ADD_EXPERIENCE_RVA,
    ),
    site("experience_add_unchecked", EXPERIENCE_ADD_UNCHECKED_RVA),
    site("daily_progress", DAILY_PROGRESS_RVA),
    site("trait_progress_add", TRAIT_PROGRESS_ADD_RVA),
    site("trait_progress_remove", TRAIT_PROGRESS_REMOVE_RVA),
    site("trait_progress_prune", TRAIT_PROGRESS_PRUNE_RVA),
    site("trait_progress_complete", TRAIT_PROGRESS_COMPLETE_RVA),
    site("trait_factor_set", TRAIT_FACTOR_SET_RVA),
    site("trait_factor_remove", TRAIT_FACTOR_REMOVE_RVA),
];

/// Index of each site in [`MUTATIONS`] that names no leader.
pub const EXILE_GENERATION: usize = 15;
pub const EXILE_REMOVAL: usize = 16;

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
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
