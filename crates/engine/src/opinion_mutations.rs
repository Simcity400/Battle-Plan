//! Relation-opinion writer observation sites. They tell collector which
//! statuses to reread through `crelation_status::Baseline`; recording never
//! calls them.
//!
//! Recomputation stores unconditionally on daily workers as well as the main
//! thread, so observers must be thread-safe and mark only changed values. Reset
//! can run during civil war, annexation, state transfer or dynamic-country reuse.
//! Loading and construction run while game state is loaded or rebuilt. Statuses
//! are allocated and freed with their diplomacy owner and are not copied whole.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

const RESET_RVA: usize = 0xd09d50;
const LOAD_TOKEN_RVA: usize = 0xd16ae0;

/// Receivers (rcx): a live relation status pointer for all but the constructor,
/// whose receiver is its raw 0x9C0-byte storage. None takes more than four
/// integer arguments or any floating-point argument.
pub const MUTATIONS: [MutationSite; 4] = [
    // Opinion recomputation with a status receiver.
    MutationSite {
        name: "recompute_opinion",
        rva: 0xd19090,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x51, 0xfb,
            0x8a, 0x02,
        ],
    },
    // Status reset with two byte arguments and an integer pointer.
    MutationSite {
        name: "reset",
        rva: RESET_RVA,
        sentinel: [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
            0x55, 0x53,
        ],
    },
    // Status token loading with reader and token arguments.
    MutationSite {
        name: "load_token",
        rva: LOAD_TOKEN_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x50, 0x41,
            0x8b, 0xd8,
        ],
    },
    // Status construction with diplomacy and target arguments.
    MutationSite {
        name: "construct",
        rva: crate::crelation_status::CONSTRUCTOR_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x56,
        ],
    },
];

/// Writers of a status's serializable relation vector (+0xE8 data,
/// +0xF0 capacity, +0xF4 count), recorded with each element's index by
/// RelationTuples, besides reset above and the war replacement body owned
/// and forwarded by [`crate::country_input_mutations`]. Each specializes one
/// typed slot replacement: status, new relation and slot arrive in RCX, RDX
/// and R8 and are retained in RDI, RSI and RBX. It swap-removes the old
/// relation from +0xD0 and, when the owner's country (+0x1F0 to +0x288 to +8)
/// is the relation's first country, from +0xE8, moving the last element into
/// its place. It deletes the old relation, replaces the slot and uses shared
/// append for +0xD0 and, for the first country's status, +0xE8. Apply and
/// clear call it for both endpoints, so named statuses are a superset of
/// those whose vectors changed. Construction, initialization and save
/// loading write type, endpoints, start/end hours, cancel and retired bytes
/// before apply reaches a replacement; no later nonwar writer was found.
/// Reset deletes and swap-removes residual elements. All sites share one
/// prologue, spilling RBX at stack +8.
pub const RELATIONS: [MutationSite; 15] = [
    // Air access and offered-air-access slot replacement (status, new, slot).
    MutationSite {
        name: "air_relation",
        rva: 0xd0_05a0,
        sentinel: RELATION_SENTINEL,
    },
    // Attache slot replacement (status, new, slot).
    MutationSite {
        name: "attache_relation",
        rva: 0xd0_0850,
        sentinel: RELATION_SENTINEL,
    },
    // Party-popularity boost slot replacement (status, new, slot).
    MutationSite {
        name: "boost_party_relation",
        rva: 0xd0_0b00,
        sentinel: RELATION_SENTINEL,
    },
    // Subject slot replacement (status, new, slot).
    MutationSite {
        name: "subject_relation",
        rva: 0xd0_0db0,
        sentinel: RELATION_SENTINEL,
    },
    // Docking rights and offered-rights slot replacement (status, new, slot).
    MutationSite {
        name: "docking_relation",
        rva: 0xd0_1060,
        sentinel: RELATION_SENTINEL,
    },
    // Embargo slot replacement (status, new, slot), from apply and clear;
    // the daily parallel sweep reaches clear.
    MutationSite {
        name: "embargo_relation",
        rva: 0xd0_1310,
        sentinel: RELATION_SENTINEL,
    },
    // Equipment-purchase slot replacement (status, new, slot).
    MutationSite {
        name: "equipment_purchase_relation",
        rva: 0xd0_15c0,
        sentinel: RELATION_SENTINEL,
    },
    // Guarantee slot replacement (status, new, slot).
    MutationSite {
        name: "guarantee_relation",
        rva: 0xd0_1870,
        sentinel: RELATION_SENTINEL,
    },
    // Improve-relations slot replacement (status, new, slot).
    MutationSite {
        name: "improve_relation",
        rva: 0xd0_1b20,
        sentinel: RELATION_SENTINEL,
    },
    // Lend-lease slot replacement (status, new, slot).
    MutationSite {
        name: "lend_lease_relation",
        rva: 0xd0_1dd0,
        sentinel: RELATION_SENTINEL,
    },
    // Production-licence slot replacement (status, new, slot).
    MutationSite {
        name: "licence_relation",
        rva: 0xd0_2080,
        sentinel: RELATION_SENTINEL,
    },
    // Market-access slot replacement (status, new, slot).
    MutationSite {
        name: "market_access_relation",
        rva: 0xd0_2330,
        sentinel: RELATION_SENTINEL,
    },
    // Military access and offered-access slot replacement (status, new, slot).
    MutationSite {
        name: "military_access_relation",
        rva: 0xd0_25e0,
        sentinel: RELATION_SENTINEL,
    },
    // Blockade slot replacement (status, new, slot).
    MutationSite {
        name: "blockade_relation",
        rva: 0xd0_2890,
        sentinel: RELATION_SENTINEL,
    },
    // Non-aggression slot replacement (status, new, slot).
    MutationSite {
        name: "non_aggression_relation",
        rva: 0xd0_2b40,
        sentinel: RELATION_SENTINEL,
    },
];

const RELATION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x49,
];

#[cfg(test)]
mod tests {
    use super::*;

    const WAR_RELATION_RVA: usize = 0xd0_2df0;

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
                .any(|(other, _, _)| other.rva == WAR_RELATION_RVA
                    && other.sentinel == RELATION_SENTINEL)
        );
    }
}
