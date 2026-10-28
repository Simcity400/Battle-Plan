//! The runtime writers of tables carried between hours by
//! `world_input_changes`. Observing them tells the collector which whole
//! table or country's rows may differ from the held read; recording never
//! calls them. Whole-table sites name no receiver. Dynamic modifier sites
//! name the container they write, whose
//! `cdynamic_modifier_container::COUNTRY` points to its owning country.
//!
//! Writer coverage:
//! - ProvinceNames: the rename effect assigns the province override string
//!   (`cprovince::NAME_OVERRIDE`) and the reset effect clears it. Both are
//!   interpreter-only effects at virtual slot 13. Province loading and both
//!   constructors run only during save load or map building, invalidating
//!   the hour.
//! - NavalRegionAccess: the strategic-navy mode setter writes each access
//!   byte in `cstrategic_navy::ACCESS_MODES`. Only the naval region access
//!   command calls it, posted by player UI or the AI interior minister's
//!   daily step. Navy reset zeroes modes during dynamic country reuse and
//!   another reset path. Construction, sizing and save reading occur only
//!   at setup or load.
//! - WorldTension, ThreatSources and ThreatCountryAggregates: total update
//!   drops exhausted sources, rebuilds aggregate cache through its sole
//!   helper and stores the clamped total (`cworld_threat`). Only
//!   source creation, source update and daily decay call it, each ending
//!   there after writes. Daily decay ramps every source. Both save readers
//!   are load-only.
//! - DynamicModifiers: a country's container
//!   (`ccountry::DYNAMIC_MODIFIERS`, `cdynamic_modifier_container`) inserts
//!   entries only through add from the add effect and its insertion helper,
//!   or save loading. Renewal sets days on the add effect's existing-entry
//!   branch. Remove effects and daily update erase through a shared helper;
//!   daily update also decrements every timed entry's days and runs for
//!   every country. Each site takes the container in RCX. Only dynamic
//!   value recalculation rewrites enabled bytes and values, reached for
//!   countries only by country modifier recalculation. Only appearance
//!   reset clears entries through the clear helper. The naval tree family
//!   forwards both country sites; the collector joins their certificate to
//!   this domain.
//!
//! Every site takes at most four integer or pointer arguments and returns
//! RAX unchanged from its original.

// MIO configuration, trait ownership and queue writers certify rare state.
// Construction and load are observed through the full census and world epoch.
// Policy reload is forwarded by country inputs; the policy score callback also
// invalidates that domain. Both observations are required before carrying it.
// Design author/bonus setters, copy, consuming move and load certify frozen
// payloads; receiver pointers and full references guard their lifetimes.
pub use crate::advisor_mutations::MutationSite;

/// Which held table a site's call may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    ProvinceNames,
    RegionAccess,
    Tension,
    /// Names its receiver: the container's owning country.
    DynamicModifiers,
    /// Rare MIO trait, queue and allowed-policy rosters, keyed by full object lifetime.
    IndustrialOrganisationRosters,
    /// The country input family forwards the policy database reload writer.
    IndustrialOrganisationPolicies,
    /// Frozen MIO author, source marker and bonuses of a design.
    DesignIndustrialOrganisation,
    /// The process-wide game fog switch, with nudge entry forwarded by buildings.
    GameFog,
}

pub const DOMAINS: usize = 8;

/// Offset of a dynamic modifier container's owning country, null for a state's.
pub const CONTAINER_COUNTRY: usize = crate::cdynamic_modifier_container::COUNTRY;

const CONTAINER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

pub const MUTATIONS: [(MutationSite, Domain); 27] = [
    // Province rename effect (effect): save RDI and reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "rename_province",
            rva: 0x37_7ce0,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x24, 0x79, 0x31, 0x03, 0x00, 0x48,
                0x8b, 0xf9,
            ],
        },
        Domain::ProvinceNames,
    ),
    // Province name reset effect (effect, scope): spill RBX to RDX's home slot.
    (
        MutationSite {
            name: "reset_province_name",
            rva: 0x37_7f30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Domain::ProvinceNames,
    ),
    // Naval access-mode assignment (navy, changes): save RBX and reserve 0x20
    // stack bytes.
    (
        MutationSite {
            name: "set_region_access_modes",
            rva: 0xf2_33d0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x45, 0x32, 0xdb, 0x45, 0x33, 0xc0, 0x4c, 0x8b,
                0xd2, 0x48,
            ],
        },
        Domain::RegionAccess,
    ),
    // Navy reset (navy): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "reset_navy",
            rva: 0xf1_edc0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Domain::RegionAccess,
    ),
    // Threat-total update (manager): save RDI and R15 and reserve 0x28 stack bytes.
    (
        MutationSite {
            name: "update_threat_total",
            rva: crate::cworld_threat::UPDATE_TOTAL_RVA,
            sentinel: [
                0x40, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x28, 0x8b, 0x51, 0x1c, 0x45, 0x33, 0xff,
                0x48, 0x89,
            ],
        },
        Domain::Tension,
    ),
    // Dynamic modifier addition (container, definition, scope, days).
    (
        MutationSite {
            name: "add_dynamic_modifier",
            rva: crate::cdynamic_modifier_container::ADD_RVA,
            sentinel: CONTAINER_SENTINEL,
        },
        Domain::DynamicModifiers,
    ),
    // Dynamic modifier renewal (container, definition, scope, days) sets
    // existing entry days.
    (
        MutationSite {
            name: "renew_dynamic_modifier",
            rva: 0x65_9360,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Domain::DynamicModifiers,
    ),
    // Dynamic modifier removal (container, definition, scope) serves the remove effect.
    (
        MutationSite {
            name: "remove_dynamic_modifier",
            rva: 0x65_85e0,
            sentinel: CONTAINER_SENTINEL,
        },
        Domain::DynamicModifiers,
    ),
    // Daily dynamic modifier update (container).
    (
        MutationSite {
            name: "dynamic_modifier_daily_update",
            rva: crate::cdynamic_modifier_container::DAILY_UPDATE_RVA,
            sentinel: CONTAINER_SENTINEL,
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "mio_queue_append",
            rva: 0xe22610,
            sentinel: [
                0x40, 0x53, 0x55, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x38, 0x48, 0x8d, 0xb9, 0x50,
                0x01, 0x00,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_state_copy",
            rva: 0xe229f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
                0x18, 0x4c,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_allowed_policies",
            rva: 0xe24f60,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_move",
            rva: 0xe28b30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_read_state",
            rva: 0xe28c70,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_remove",
            rva: 0xe291c0,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x40, 0x48, 0x89, 0x5c, 0x24, 0x50, 0x48, 0x8b, 0xf9,
                0x48, 0x89,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_replace",
            rva: 0xe29340,
            sentinel: [
                0x40, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x38, 0x48, 0x8d, 0xb9, 0x50, 0x01, 0x00,
                0x00, 0x4c,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_unlock_trait",
            rva: 0xe2a180,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x50, 0x48, 0x8d, 0x05, 0x0f,
                0x90, 0x99,
            ],
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "design_mio_bonuses",
            rva: 0xc297f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_link",
            rva: 0xc37bf0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_move",
            rva: 0x673b50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xfa, 0x48,
                0x8b, 0xd9,
            ],
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_clone",
            rva: 0xc2d470,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x48,
            ],
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_read",
            rva: 0xc34170,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "mio_policy_score_refresh",
            rva: 0xa9a5a0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x19, 0x48, 0x8d, 0x54, 0x24, 0x30,
                0x48, 0x8b,
            ],
        },
        Domain::IndustrialOrganisationPolicies,
    ),
    (
        MutationSite {
            name: "fog_enter_game",
            rva: 0x00e30070,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48, 0x89, 0x78,
                0x20, 0x48,
            ],
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_enter_frontend",
            rva: 0x00b8d180,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x55,
            ],
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_leave_nudge",
            rva: 0x01364760,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_toggle",
            rva: 0x0026fea0,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xd9, 0x33,
                0xc0, 0x89,
            ],
        },
        Domain::GameFog,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_cover_every_domain() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|(other, _)| other.rva != site.rva && other.name != site.name)
            );
        }
        for domain in [
            Domain::ProvinceNames,
            Domain::RegionAccess,
            Domain::Tension,
            Domain::DynamicModifiers,
            Domain::IndustrialOrganisationRosters,
            Domain::IndustrialOrganisationPolicies,
            Domain::DesignIndustrialOrganisation,
            Domain::GameFog,
        ] {
            assert!(MUTATIONS.iter().any(|(_, of)| *of == domain));
            assert!((domain as usize) < DOMAINS);
        }
        // The daily update's sentinel matches its supported entry sequence.
        assert_eq!(
            crate::cdynamic_modifier_container::DAILY_UPDATE_SENTINEL,
            CONTAINER_SENTINEL
        );
    }
}
