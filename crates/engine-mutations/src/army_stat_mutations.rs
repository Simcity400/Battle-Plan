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
//!   no member, so it revokes every certificate.
//! - Other member writers reach only new divisions (two helpers, one from
//!   deployment) or loads. The collector reads those afresh or invalidates.

pub use crate::advisor_mutations::MutationSite;

const UPDATE_ACTIVE_STATS_RVA: usize = 0xc7_deb0;

/// The writers, in callback order: `update_active_stats` (RCX the primary
/// division), `refresh_cached_name` (RCX the member) and `refresh_country_names`
/// (RCX the group and tracker, RDX the country). A detour forwards RCX..R9
/// and returns RAX unchanged.
const AWARD_EXPERIENCE_RVA: usize = 0xc5f3a0;
const ADD_MANPOWER_EXPERIENCE_RVA: usize = 0xc5f4d0;
const REFRESH_EXPERIENCE_RVA: usize = 0xc80830;
const REBIND_EXPERIENCE_RVA: usize = 0xc6a2c0;

const CONSTRUCT_DIVISION_HISTORY_RVA: usize = 0x1433ad0;
const DESTROY_DIVISION_HISTORY_RVA: usize = 0x1434090;
const APPEND_DIVISION_HISTORY_RVA: usize = 0x14344f0;
const APPLY_DIVISION_AWARD_RVA: usize = 0x1434ac0;
const CLEAR_MEDAL_INHERITANCE_RVA: usize = 0x1438df0;
const CONSTRUCT_MEDAL_STORE_RVA: usize = 0x1433fe0;
const BIND_DIVISION_TEMPLATE_RVA: usize = 0xc7ee00;
const ADD_OFFICER_EXPERIENCE_RVA: usize = 0x13d_3b00;
const SET_OFFICER_EXPERIENCE_RVA: usize = 0x13d_5960;
const PROMOTE_DIVISION_OFFICER_RVA: usize = 0x143e500;
const DESTROY_MEDAL_STORE_RVA: usize = 0x1434450;

pub const MUTATIONS: [MutationSite; 19] = [
    // The five-byte RBX store to R8's home slot relocates, followed by saves of
    // RBP/RSI/RDI/R12..R15.
    MutationSite {
        name: "update_active_stats",
        rva: UPDATE_ACTIVE_STATS_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // The five-byte RBX store to RCX's home slot relocates. It is followed by an
    // RSI store to RDX's home slot, an RDI save, 0x20 stack bytes and the prefix
    // of the address calculation placing the member's cached name in RSI.
    MutationSite {
        name: "refresh_cached_name",
        rva: 0x9b_4c40,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    // The five-byte R8B store to its home slot relocates. It is followed by an
    // RDX store to its home slot and saves of RBX/RBP/RSI/RDI/R12.
    MutationSite {
        name: "refresh_country_names",
        rva: 0x9a_d440,
        sentinel: [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x57,
            0x41, 0x54,
        ],
    },
    MutationSite {
        name: "award_experience",
        rva: AWARD_EXPERIENCE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x30, 0x48,
        ],
    },
    MutationSite {
        name: "add_manpower_experience",
        rva: ADD_MANPOWER_EXPERIENCE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    MutationSite {
        name: "refresh_experience",
        rva: REFRESH_EXPERIENCE_RVA,
        sentinel: [
            0x40, 0x53, 0x55, 0x56, 0x57, 0x48, 0x81, 0xec, 0x68, 0x01, 0x00, 0x00, 0x48, 0x8b,
            0xf1, 0x45,
        ],
    },
    MutationSite {
        name: "rebind_experience",
        rva: REBIND_EXPERIENCE_RVA,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x8b, 0x89, 0xb8, 0x03, 0x00, 0x00, 0x48,
            0x8b, 0xd9,
        ],
    },
    MutationSite {
        name: "construct_division",
        rva: engine_native::carmy::CONSTRUCTOR_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x48,
            0x83, 0xec,
        ],
    },
    MutationSite {
        name: "construct_division_history",
        rva: CONSTRUCT_DIVISION_HISTORY_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    MutationSite {
        name: "destroy_division_history",
        rva: DESTROY_DIVISION_HISTORY_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xd7,
            0x6b, 0x57,
        ],
    },
    MutationSite {
        name: "append_division_history",
        rva: APPEND_DIVISION_HISTORY_RVA,
        sentinel: [
            0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41,
            0x57, 0x48,
        ],
    },
    MutationSite {
        name: "apply_division_award",
        rva: APPLY_DIVISION_AWARD_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x57,
        ],
    },
    MutationSite {
        name: "clear_medal_inheritance",
        rva: CLEAR_MEDAL_INHERITANCE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x59, 0x08,
            0x48, 0x63,
        ],
    },
    MutationSite {
        name: "construct_medal_store",
        rva: CONSTRUCT_MEDAL_STORE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x57,
        ],
    },
    MutationSite {
        name: "destroy_medal_store",
        rva: DESTROY_MEDAL_STORE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    MutationSite {
        name: "promote_division_officer",
        rva: PROMOTE_DIVISION_OFFICER_RVA,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0x5a, 0x28, 0x48, 0x63, 0x42, 0x34,
            0x48, 0x89,
        ],
    },
    MutationSite {
        name: "bind_division_template",
        rva: BIND_DIVISION_TEMPLATE_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
    },
    MutationSite {
        name: "add_officer_experience",
        rva: ADD_OFFICER_EXPERIENCE_RVA,
        sentinel: [
            0x4c, 0x8b, 0x81, 0x88, 0, 0, 0, 0x33, 0xc0, 0x44, 0x8b, 0x49, 0x10, 0x4c, 0x03, 0xc2,
        ],
    },
    MutationSite {
        name: "set_officer_experience",
        rva: SET_OFFICER_EXPERIENCE_RVA,
        sentinel: [
            0x44, 0x8b, 0x49, 0x10, 0x33, 0xc0, 0x45, 0x85, 0xc9, 0x74, 0x1c, 0x41, 0x83, 0xf9, 1,
            0x75,
        ],
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

    const EXPECTED_CACHED_NAME_RVA: usize = 0x9b_4c40;
    const EXPECTED_COUNTRY_NAMES_RVA: usize = 0x9a_d440;

    /// The sentinel opens with the five-byte store the hook relocates and
    /// the seven pushes of the prologue; the site is the function the carmy
    /// layout names.
    #[test]
    fn the_sentinel_is_the_relocated_store_and_the_prologue_pushes() {
        let [site, refresh, country, ..] = MUTATIONS;
        // Each patch replaces exactly one five-byte store.
        for site in [&refresh, &country] {
            assert!(matches!(
                site.sentinel[..5],
                [0x48 | 0x44, 0x89 | 0x88, _, 0x24, _]
            ));
        }
        assert_eq!(refresh.rva, EXPECTED_CACHED_NAME_RVA);
        assert_eq!(country.rva, EXPECTED_COUNTRY_NAMES_RVA);
        assert_eq!(site.rva, crate::carmy::UPDATE_ACTIVE_STATS_RVA);
        assert_eq!(site.sentinel[..5], [0x48, 0x89, 0x5c, 0x24, 0x18]);
        assert_eq!(
            site.sentinel[5..],
            [
                0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57
            ]
        );
    }
}
