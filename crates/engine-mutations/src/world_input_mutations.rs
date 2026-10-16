//! The runtime writers of tables carried between hours by
//! `world_input_changes`. Observing them tells the collector which whole table
//! or country's rows may differ from the held read; recording never calls
//! them. Whole-table sites name no receiver. Dynamic modifier sites name the
//! container they write, whose +0x08 points to its owning country.
//!
//! Writer coverage:
//! - ProvinceNames: the rename effect assigns province override string +0x40
//!   and the reset effect clears it (`cprovince`). Both are
//!   interpreter-only effects at virtual slot 13. Province loading and both
//!   constructors run only during save load or map building, invalidating
//!   the hour.
//! - NavalRegionAccess: the strategic-navy mode setter writes each access
//!   byte in +0xB0 (`cstrategic_navy::ACCESS_MODES`). Only the naval
//!   region access command calls it, posted by player UI or the AI interior
//!   minister's daily step. Navy reset zeroes modes during dynamic country
//!   reuse and another reset path. Construction, sizing and save reading
//!   occur only at setup or load.
//! - WorldTension, ThreatSources and ThreatCountryAggregates: total update
//!   drops exhausted sources, rebuilds aggregate cache through its sole
//!   helper and stores the clamped total (`cworld_threat`). Only
//!   source creation, source update and daily decay call it, each ending
//!   there after writes. Daily decay ramps every source. Both save readers
//!   are load-only.
//! - DynamicModifiers: a country's container +0xE58
//!   (`cdynamic_modifier_container`) inserts entries only through add
//!   from the add effect and its insertion helper, or save loading. Renewal
//!   sets days on the add effect's existing-entry branch. Remove effects and
//!   daily update erase through a shared helper; daily update also decrements
//!   every timed entry's days and runs for every country. Each site takes the
//!   container in RCX. Only dynamic value recalculation rewrites enabled
//!   bytes and values, reached for countries only by country modifier
//!   recalculation. Only appearance reset clears entries through the clear
//!   helper. The naval tree family forwards both country sites; the
//!   collector joins their certificate to this domain.
//!
//! Every site takes at most four integer or pointer arguments and returns
//! RAX unchanged from its original.

pub use crate::advisor_mutations::MutationSite;

/// Which held table a site's call may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    ProvinceNames,
    RegionAccess,
    Tension,
    /// Names its receiver: the container's owning country.
    DynamicModifiers,
}

pub const DOMAINS: usize = 4;

/// Offset of a dynamic modifier container's owning country, null for a state's.
pub const CONTAINER_COUNTRY: usize = crate::cdynamic_modifier_container::COUNTRY;

const CONTAINER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

pub const MUTATIONS: [(MutationSite, Domain); 9] = [
    // Province rename effect (effect): save RDI and reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "rename_province",
            rva: 0x35_f100,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0xe5, 0x9a, 0x26, 0x03, 0x00, 0x48,
                0x8b, 0xf9,
            ],
        },
        Domain::ProvinceNames,
    ),
    // Province name reset effect (effect, scope): spill RBX at stack +0x10.
    (
        MutationSite {
            name: "reset_province_name",
            rva: 0x35_f350,
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
            rva: 0xe9_a650,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x45, 0x32, 0xdb, 0x45, 0x33, 0xc0, 0x4c, 0x8b,
                0xd2, 0x48,
            ],
        },
        Domain::RegionAccess,
    ),
    // Navy reset (navy): spill RBX at stack +8.
    (
        MutationSite {
            name: "reset_navy",
            rva: 0xe9_6510,
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
            rva: 0x60_59e0,
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
            rva: 0x60_4c60,
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
