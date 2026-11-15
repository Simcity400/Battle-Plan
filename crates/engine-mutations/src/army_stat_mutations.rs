//! Runtime writer of the division stat block (`cunit::STAT_BLOCK`) whose attack
//! operands DivisionStatus records (`attack_stats`). Observing it selects
//! divisions to reread; recording never calls it.
//!
//! - Active-stat rebuilding writes the block through the template's equipment
//!   stat calculation (its sixth argument is the division's block), then the
//!   equipment modifier loop. Callers are the pre-hourly pass (one in 24
//!   divisions per hour, selected by `(id + 1) % 24`), its parallel worker,
//!   another refresh helper and the division ability refresh. The only other
//!   reference is unwind data.
//! - The ten other equipment-stat calculation callers write stack definitions.
//!   Definition assignment does not reach an army block outside active-stat
//!   rebuilding; its callers build ships, railway guns, deployments and templates.
//! - Construction allocates a new division's block; a division without a held
//!   read is read afresh. Loads and resets invalidate the hour and revoke every
//!   certificate.
//!
//! This domain also observes writers of a division's name member
//! (`carmy_combat_inputs::NAME_GROUP_MEMBER`, `cname_group_member`), whose span
//! and name cache (`cname_group_member::CACHED_NAME`) DivisionStatus records.
//! - Only construction stores the name-member pointer, at two sites; nothing
//!   reassigns it.
//! - Cached-name refresh ends every runtime rewrite of an existing member:
//!   template assignment, template edits/autonomy/annexation, fake-template
//!   assignment and selection (the latter from active-stat rebuilding), three
//!   rename paths behind the division-name command, the template binding and
//!   two order setters called first by those paths, and division deserialization.
//!   Ships, operatives and production lines also call it without naming a
//!   watched member.
//! - Country-wide name refresh rewrites matching name-group members inline
//!   without cached-name refresh. It runs from template-update commands and
//!   hourly country-update workers when the group tracker is dirty. It names
//!   no member, only its country (RDX), whose division roster holds every
//!   member it rewrites: the collector marks those once it returns.
//! - Other member writers reach only new divisions (two helpers, one from
//!   deployment) or loads. The collector reads those afresh or invalidates.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

const UPDATE_ACTIVE_STATS_RVA: Rva = Rva(0xce_a3c0);

/// The writers, in callback order: `update_active_stats` (RCX the primary
/// division), `refresh_cached_name` (RCX the member) and `refresh_country_names`
/// (RCX the group and tracker, RDX the country). A detour forwards RCX..R9
/// and returns RAX unchanged.
const AWARD_EXPERIENCE_RVA: Rva = Rva(0xccb810);
const ADD_MANPOWER_EXPERIENCE_RVA: Rva = Rva(0xccb940);
const REFRESH_EXPERIENCE_RVA: Rva = Rva(0xcecd10);
const REBIND_EXPERIENCE_RVA: Rva = Rva(0xcd6700);

const CONSTRUCT_DIVISION_HISTORY_RVA: Rva = Rva(0x14e23c0);
const DESTROY_DIVISION_HISTORY_RVA: Rva = Rva(0x14e2980);
const APPEND_DIVISION_HISTORY_RVA: Rva = Rva(0x14e2de0);
const APPLY_DIVISION_AWARD_RVA: Rva = Rva(0x14e33b0);
const CLEAR_MEDAL_INHERITANCE_RVA: Rva = Rva(0x14e76e0);
const CONSTRUCT_MEDAL_STORE_RVA: Rva = Rva(0x14e28d0);
const BIND_DIVISION_TEMPLATE_RVA: Rva = Rva(0xceb320);
const ADD_OFFICER_EXPERIENCE_RVA: Rva = Rva(0x148_4b80);
const SET_OFFICER_EXPERIENCE_RVA: Rva = Rva(0x148_69e0);
const PROMOTE_DIVISION_OFFICER_RVA: Rva = Rva(0x14ecd00);
const DESTROY_MEDAL_STORE_RVA: Rva = Rva(0x14e2d40);

pub const MUTATIONS: [MutationSite; 19] = [
    // The five-byte RBX store to R8's home slot relocates, followed by saves of
    // RBP/RSI/RDI/R12..R15.
    MutationSite {
        name: "update_active_stats",
        rva: UPDATE_ACTIVE_STATS_RVA,
    },
    // The five-byte RBX store to RCX's home slot relocates. It is followed by an
    // RSI store to RDX's home slot, an RDI save, 0x20 stack bytes and the prefix
    // of the address calculation placing the member's cached name in RSI.
    MutationSite {
        name: "refresh_cached_name",
        rva: Rva(0xa0_c820),
    },
    // The five-byte R8B store to its home slot relocates. It is followed by an
    // RDX store to its home slot and saves of RBX/RBP/RSI/RDI/R12.
    MutationSite {
        name: "refresh_country_names",
        rva: Rva(0xa0_5020),
    },
    MutationSite {
        name: "award_experience",
        rva: AWARD_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "add_manpower_experience",
        rva: ADD_MANPOWER_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "refresh_experience",
        rva: REFRESH_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "rebind_experience",
        rva: REBIND_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "construct_division",
        rva: engine_native::carmy::CONSTRUCTOR_RVA,
    },
    MutationSite {
        name: "construct_division_history",
        rva: CONSTRUCT_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "destroy_division_history",
        rva: DESTROY_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "append_division_history",
        rva: APPEND_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "apply_division_award",
        rva: APPLY_DIVISION_AWARD_RVA,
    },
    MutationSite {
        name: "clear_medal_inheritance",
        rva: CLEAR_MEDAL_INHERITANCE_RVA,
    },
    MutationSite {
        name: "construct_medal_store",
        rva: CONSTRUCT_MEDAL_STORE_RVA,
    },
    MutationSite {
        name: "destroy_medal_store",
        rva: DESTROY_MEDAL_STORE_RVA,
    },
    MutationSite {
        name: "promote_division_officer",
        rva: PROMOTE_DIVISION_OFFICER_RVA,
    },
    MutationSite {
        name: "bind_division_template",
        rva: BIND_DIVISION_TEMPLATE_RVA,
    },
    MutationSite {
        name: "add_officer_experience",
        rva: ADD_OFFICER_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "set_officer_experience",
        rva: SET_OFFICER_EXPERIENCE_RVA,
    },
];

/// Template rebinding can rescale stored XP before the later experience refresh.
pub const EXPERIENCE_TEMPLATE_SITE: usize = 16;

/// These writers invalidate the independent stored-experience certificate only.
pub const EXPERIENCE_FIRST: usize = 3;
/// History/store writers follow the independent experience writers.
pub const HISTORY_FIRST: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_site_is_the_function_the_carmy_layout_names() {
        assert_eq!(MUTATIONS[0].rva, crate::carmy::UPDATE_ACTIVE_STATS_RVA);
    }
}
