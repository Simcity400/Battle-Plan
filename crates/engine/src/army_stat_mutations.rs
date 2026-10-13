//! Runtime writer of the division stat block (`+0x138`) whose attack operands
//! DivisionStatus records (`attack_stats`). Observing it selects divisions to
//! reread; recording never calls it.
//!
//! - Active-stat rebuilding writes the block through the template's equipment
//!   stat calculation (its sixth argument is the division's block), then the
//!   equipment modifier loop (`+0xA0..+0x310`). Callers are the pre-hourly pass
//!   (one in 24 divisions per hour, selected by `(id + 1) % 24`), its parallel
//!   worker, another refresh helper and the division ability refresh. The only
//!   other reference is unwind data.
//! - The ten other equipment-stat calculation callers write stack definitions.
//!   Definition assignment does not reach an army block outside active-stat
//!   rebuilding; its callers build ships, railway guns, deployments and templates.
//! - Construction allocates a new division's block; a division without a held
//!   read is read afresh. Loads and resets invalidate the hour and revoke every
//!   certificate.
//!
//! This domain also observes writers of a division's name member (`+0x340`,
//! [`crate::cname_group_member`]), whose span and `+0x60` name cache
//! DivisionStatus records.
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
pub const MUTATIONS: [MutationSite; 3] = [
    // The five-byte RBX store at RSP+0x18 relocates, followed by saves of
    // RBP/RSI/RDI/R12..R15.
    MutationSite {
        name: "update_active_stats",
        rva: UPDATE_ACTIVE_STATS_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    // The five-byte RBX store at RSP+0x8 relocates. It is followed by an RSI
    // store at RSP+0x10, an RDI save, 0x20 stack bytes and the prefix of the
    // address calculation placing RCX+0x60 in RSI.
    MutationSite {
        name: "refresh_cached_name",
        rva: 0x9b_4c40,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    // The five-byte R8B store at RSP+0x18 relocates. It is followed by an RDX
    // store at RSP+0x10 and saves of RBX/RBP/RSI/RDI/R12.
    MutationSite {
        name: "refresh_country_names",
        rva: 0x9a_d440,
        sentinel: [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x57,
            0x41, 0x54,
        ],
    },
];

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
        let [site, refresh, country] = MUTATIONS;
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
