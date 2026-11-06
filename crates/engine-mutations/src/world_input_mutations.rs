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

use engine_sites::Rva;
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

pub const MUTATIONS: [(MutationSite, Domain); 27] = [
    // Province rename effect (effect): save RDI and reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "rename_province",
            rva: Rva(0x37_7d50),
        },
        Domain::ProvinceNames,
    ),
    // Province name reset effect (effect, scope): spill RBX to RDX's home slot.
    (
        MutationSite {
            name: "reset_province_name",
            rva: Rva(0x37_7fa0),
        },
        Domain::ProvinceNames,
    ),
    // Naval access-mode assignment (navy, changes): save RBX and reserve 0x20
    // stack bytes.
    (
        MutationSite {
            name: "set_region_access_modes",
            rva: Rva(0xf2_3440),
        },
        Domain::RegionAccess,
    ),
    // Navy reset (navy): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "reset_navy",
            rva: Rva(0xf1_ee30),
        },
        Domain::RegionAccess,
    ),
    // Threat-total update (manager): save RDI and R15 and reserve 0x28 stack bytes.
    (
        MutationSite {
            name: "update_threat_total",
            rva: crate::cworld_threat::UPDATE_TOTAL_RVA,
        },
        Domain::Tension,
    ),
    // Dynamic modifier addition (container, definition, scope, days).
    (
        MutationSite {
            name: "add_dynamic_modifier",
            rva: crate::cdynamic_modifier_container::ADD_RVA,
        },
        Domain::DynamicModifiers,
    ),
    // Dynamic modifier renewal (container, definition, scope, days) sets
    // existing entry days.
    (
        MutationSite {
            name: "renew_dynamic_modifier",
            rva: Rva(0x65_93d0),
        },
        Domain::DynamicModifiers,
    ),
    // Dynamic modifier removal (container, definition, scope) serves the remove effect.
    (
        MutationSite {
            name: "remove_dynamic_modifier",
            rva: Rva(0x65_8650),
        },
        Domain::DynamicModifiers,
    ),
    // Daily dynamic modifier update (container).
    (
        MutationSite {
            name: "dynamic_modifier_daily_update",
            rva: crate::cdynamic_modifier_container::DAILY_UPDATE_RVA,
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "mio_queue_append",
            rva: Rva(0xe22680),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_state_copy",
            rva: Rva(0xe22a60),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_allowed_policies",
            rva: Rva(0xe24fd0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_move",
            rva: Rva(0xe28ba0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_read_state",
            rva: Rva(0xe28ce0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_remove",
            rva: Rva(0xe29230),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_replace",
            rva: Rva(0xe293b0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_unlock_trait",
            rva: Rva(0xe2a1f0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "design_mio_bonuses",
            rva: Rva(0xc29860),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_link",
            rva: Rva(0xc37c60),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_move",
            rva: Rva(0x673bc0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_clone",
            rva: Rva(0xc2d4e0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_read",
            rva: Rva(0xc341e0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "mio_policy_score_refresh",
            rva: Rva(0xa9a610),
        },
        Domain::IndustrialOrganisationPolicies,
    ),
    (
        MutationSite {
            name: "fog_enter_game",
            rva: Rva(0x00e300e0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_enter_frontend",
            rva: Rva(0x00b8d1f0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_leave_nudge",
            rva: Rva(0x013647d0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_toggle",
            rva: Rva(0x0026ff10),
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
    }
}
