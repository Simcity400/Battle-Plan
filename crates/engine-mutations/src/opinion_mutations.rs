//! Relation-opinion writer observation sites. They tell collector which
//! statuses to reread through `crelation_status::Baseline`; recording never
//! calls them.
//!
//! Recomputation stores unconditionally on daily workers as well as the main
//! thread, so observers must be thread-safe and mark only changed values. Reset
//! can run during civil war, annexation, state transfer or dynamic-country reuse.
//! Loading and construction run while game state is loaded or rebuilt. Statuses
//! are allocated and freed with their diplomacy owner and are not copied whole.

use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

const RESET_RVA: Rva = Rva(0xd7fdf0);
const LOAD_TOKEN_RVA: Rva = Rva(0xd8cd80);

/// Receivers (rcx): a live relation status pointer for all but the constructor,
/// whose receiver is its raw 0x9C0-byte storage. None takes more than four
/// integer arguments or any floating-point argument.
pub const MUTATIONS: [MutationSite; 4] = [
    // Opinion recomputation with a status receiver.
    MutationSite {
        name: "recompute_opinion",
        rva: Rva(0xd8f350),
    },
    // Status reset with two byte arguments and an integer pointer.
    MutationSite {
        name: "reset",
        rva: RESET_RVA,
    },
    // Status token loading with reader and token arguments.
    MutationSite {
        name: "load_token",
        rva: LOAD_TOKEN_RVA,
    },
    // Status construction with diplomacy and target arguments.
    MutationSite {
        name: "construct",
        rva: crate::crelation_status::CONSTRUCTOR_RVA,
    },
];

/// Writers of a status's serializable relation vector (data, capacity and
/// count from `crelation_status::SERIALIZABLE_RELATIONS`), recorded with each
/// element's index by RelationTuples, besides reset above and the war
/// replacement body owned and forwarded by [`crate::country_input_mutations`].
/// Each specializes one typed slot replacement: status, new relation and slot
/// arrive in RCX, RDX and R8 and are retained in RDI, RSI and RBX. It
/// swap-removes the old relation from the subrelations
/// (`crelation_status::SUBRELATIONS`) and, when the owner's country (diplomacy
/// `crelation_status::DIPLOMACY_STATUS` to `cdiplomacy_status::COUNTRY` to
/// `ccountry::ID`) is the relation's first country, from the serializable
/// relations, moving the last element into its place. It deletes the old
/// relation, replaces the slot and uses shared append for the subrelations
/// and, for the first country's status, the serializable relations. Apply and
/// clear call it for both endpoints, so named statuses are a superset of
/// those whose vectors changed. Construction, initialization and save
/// loading write type, endpoints, start/end hours, cancel and retired bytes
/// before apply reaches a replacement; no later nonwar writer was found.
/// Reset deletes and swap-removes residual elements. All sites share one
/// prologue, spilling RBX to RCX's home slot.
pub const RELATIONS: [MutationSite; 15] = [
    // Air access and offered-air-access slot replacement (status, new, slot).
    MutationSite {
        name: "air_relation",
        rva: Rva(0xd7_6610),
    },
    // Attache slot replacement (status, new, slot).
    MutationSite {
        name: "attache_relation",
        rva: Rva(0xd7_68c0),
    },
    // Party-popularity boost slot replacement (status, new, slot).
    MutationSite {
        name: "boost_party_relation",
        rva: Rva(0xd7_6b70),
    },
    // Subject slot replacement (status, new, slot).
    MutationSite {
        name: "subject_relation",
        rva: Rva(0xd7_6e20),
    },
    // Docking rights and offered-rights slot replacement (status, new, slot).
    MutationSite {
        name: "docking_relation",
        rva: Rva(0xd7_70d0),
    },
    // Embargo slot replacement (status, new, slot), from apply and clear;
    // the daily parallel sweep reaches clear.
    MutationSite {
        name: "embargo_relation",
        rva: Rva(0xd7_7380),
    },
    // Equipment-purchase slot replacement (status, new, slot).
    MutationSite {
        name: "equipment_purchase_relation",
        rva: Rva(0xd7_7630),
    },
    // Guarantee slot replacement (status, new, slot).
    MutationSite {
        name: "guarantee_relation",
        rva: Rva(0xd7_78e0),
    },
    // Improve-relations slot replacement (status, new, slot).
    MutationSite {
        name: "improve_relation",
        rva: Rva(0xd7_7b90),
    },
    // Lend-lease slot replacement (status, new, slot).
    MutationSite {
        name: "lend_lease_relation",
        rva: Rva(0xd7_7e40),
    },
    // Production-licence slot replacement (status, new, slot).
    MutationSite {
        name: "licence_relation",
        rva: Rva(0xd7_80f0),
    },
    // Market-access slot replacement (status, new, slot).
    MutationSite {
        name: "market_access_relation",
        rva: Rva(0xd7_83a0),
    },
    // Military access and offered-access slot replacement (status, new, slot).
    MutationSite {
        name: "military_access_relation",
        rva: Rva(0xd7_8650),
    },
    // Blockade slot replacement (status, new, slot).
    MutationSite {
        name: "blockade_relation",
        rva: Rva(0xd7_8900),
    },
    // Non-aggression slot replacement (status, new, slot).
    MutationSite {
        name: "non_aggression_relation",
        rva: Rva(0xd7_8bb0),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_bodies_are_distinct_and_patched_by_no_other_family() {
        for (index, site) in RELATIONS.iter().enumerate() {
            assert!(
                RELATIONS[..index]
                    .iter()
                    .all(|other| other.rva != site.rva && other.name != site.name)
            );
            assert!(MUTATIONS.iter().all(|other| other.rva != site.rva));
            assert!(
                crate::country_input_mutations::MUTATIONS
                    .iter()
                    .all(|(other, _, _)| other.rva != site.rva)
            );
        }
        // The war body is the stability family's site, forwarded.
        assert!(
            crate::country_input_mutations::MUTATIONS
                .iter()
                .any(|(other, _, _)| other.name == "replace_relation")
        );
    }
}
