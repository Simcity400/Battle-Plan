//! The runtime writers of each country's local sub-unit clones (deployment
//! status vector `cdeployment_status::LOCAL_SUBUNITS` and its count) and each
//! clone's archetype requirements (definition pool
//! `csub_unit_definition::ARCHETYPE_REQUIREMENTS`, data
//! `ARCHETYPE_REQUIREMENTS_DATA`), which CountryLocalSubunits records.
//! Observing them tells the collector which countries' clones to reread;
//! recording never calls them.
//!
//! Writer coverage:
//! - Append: ensuring a local sub-unit copy-constructs and rebuilds a clone,
//!   then pushes it onto the vector and raises its count ([`CLONE_COUNT`]). An
//!   already-cloned token only has a flag ORed into the clone, which no row
//!   records; the count therefore identifies an append. Callers include
//!   research completion, army addition, another caller, the sub-unit unlock
//!   effect and deployment post-read repair. Default sub-unit initialization
//!   instead appends an unrebuilt copy of each admitted real definition. Only
//!   country runtime-subsystem initialization calls it, at game reset and
//!   during dynamic reuse after appearance reset. Save loading rebuilds clones
//!   from unlock and bonus state; it does not deserialize the clone vector.
//! - Requirements: rebuilding a local definition takes the global definition,
//!   the country's id field (`ccountry::ID`), the deployment status's bonus
//!   list (`BONUS_LIST`) and clone. It copy-assigns the global definition,
//!   including the requirement pool and database index
//!   (`csub_unit_definition::DATABASE_INDEX`), then applies technology and
//!   deployment bonuses. These are the only runtime writes of a clone's
//!   requirement pool. Every caller passes the deployment bonus list in R8:
//!   clone ensuring, post-read repair, rebuilding by token and rebuilding all
//!   clones. The by-token path follows affected bonus rebuilds from research
//!   completion, research reset, bonus addition and another caller. The
//!   all-clones path follows the game state's country rebuild, called only by
//!   the equipment reload callback and the two sub-unit reload callbacks, after
//!   replacing the global definitions against which rows are compared.
//! - Clear: deployment runtime-state clearing deletes every clone and zeroes
//!   the count. The destructor and appearance reset during dynamic country
//!   reuse call it.
//! - Lifetime: only country construction constructs the deployment status,
//!   stored in `ccountry::DEPLOYMENT_MANAGER`. Replacing an object first
//!   destroys the old one through the observed runtime-state clear.
//! - Shared dependencies: a row exists only when a clone's requirements
//!   differ from its global definition's, and names each archetype token.
//!   Global definitions change only through their find-or-create routine,
//!   which resets and parses them. Three content loaders call it, also from
//!   the two sub-unit reload callbacks; the equipment reload callback replaces
//!   archetypes. Neither operation names a country, so both revoke every
//!   certificate.

pub use crate::advisor_mutations::MutationSite;

/// How a site names the written deployment status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the deployment status.
    Status,
    /// R8 is the deployment status's bonus list, at `BONUS_LIST`.
    BonusList,
    /// A shared dependency of every country's rows: every certificate is
    /// revoked.
    Unnamed,
}

/// The offset of [`Receiver::BonusList`] within the deployment status: its persistent
/// unit-bonus list.
pub const BONUS_LIST: usize = crate::cdeployment_status::UNIT_BONUSES;

/// The clone vector's count within the deployment status (the count word after
/// `crate::cdeployment_status::LOCAL_SUBUNITS`'s data pointer and capacity).
pub const CLONE_COUNT: usize = crate::cdeployment_status::LOCAL_SUBUNITS + 0xc;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 6] = [
    // Ensure a clone (deployment status, global definition, flag).
    (
        MutationSite {
            name: "ensure_local_subunit",
            rva: 0xd6_f950,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Status,
    ),
    // Rebuild a clone (global definition, country id field, status bonus list,
    // clone).
    (
        MutationSite {
            name: "rebuild_local_subunit",
            rva: 0x10a_0ff0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::BonusList,
    ),
    // Clear deployment runtime state (status).
    (
        MutationSite {
            name: "clear_local_subunits",
            rva: 0xd7_04e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::Status,
    ),
    // Initialize default sub-units (status).
    (
        MutationSite {
            name: "initialize_default_subunits",
            rva: 0xd7_54f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Status,
    ),
    // Find or create a definition (database, reader). The stack pointer
    // is copied to RAX before spilling RBX through it to R8's home slot; seven
    // bytes relocate.
    (
        MutationSite {
            name: "find_or_create_subunit_definition",
            rva: 0xb1_50b0,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55,
                0x41, 0x56,
            ],
        },
        Receiver::Unnamed,
    ),
    // The equipment content reload callback (context).
    (
        MutationSite {
            name: "reload_equipment_content",
            rva: 0x17_e760,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Unnamed,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sentinel opens with the RBX spill the hook relocates: five bytes,
    /// or seven with the preceding stack-pointer copy to RAX. Sites are distinct.
    #[test]
    fn sentinels_open_with_the_relocated_spill() {
        for (site, _) in MUTATIONS {
            // RBX spills through RAX after a stack-pointer copy, otherwise through RSP.
            let spill = match site.sentinel[..3] {
                [0x48, 0x8b, 0xc4] => (&site.sentinel[3..6], [0x48, 0x89, 0x58]),
                _ => (&site.sentinel[..3], [0x48, 0x89, 0x5c]),
            };
            assert_eq!(spill.0, spill.1.as_slice(), "{}", site.name);
        }
        let mut rvas = MUTATIONS.map(|(site, _)| site.rva);
        rvas.sort_unstable();
        assert!(rvas.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
