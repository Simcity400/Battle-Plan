//! Runtime writers of everything an AirBaseRegistrations subtree reads from
//! a state air base. Observing them selects bases to reread; recording never
//! calls them.
//!
//! Registration rows read the base's slot vector
//! (`cair_base::COUNTRY_CONTAINERS`) and dense list
//! (`cair_base::REGISTERED_CONTAINERS`); each listed or occupied country
//! container's country, pool vector and capacity (`ccountry_air_container::COUNTRY`,
//! `POOLS`, `CAPACITY_RAW`); each pool's vtable and wing vector
//! (`cair_wing_pool::WINGS`); and each wing's identity and pool backlink
//! (`cair_wing::POOL`). Container creation copies the base handle
//! (`cair_base::HANDLE`) into the container's reference
//! (`ccountry_air_container::REFERENCE_CANDIDATE`). A pool's base setter writes its
//! base handle (`cair_wing_pool::AIR_BASE_REFERENCE`) before registering it; a
//! wing names its pool directly. Each site names its changed base by that
//! handle, through the base or the pool in RCX. An unlinked pool's handle is
//! zero or the shared invalid reference, both before first linking and after
//! erasure. Such pools belong to no base's pool vector, so their writes change
//! no base rows. The collector carries a base only while every pool it lists
//! names that base.
//!
//! - Only country-container creation and country-access removal write the
//!   slot vector and dense list at runtime. Creation is reached from initial
//!   population, container repair, access refresh, container lookup and two
//!   other base helpers; it also creates the country and default capacity.
//!   Removal is reached from withdrawal, two other access helpers and ship
//!   transfer; it compacts the list, nulls the slot and frees the container.
//! - Pool registration appends to a container's pool vector; pool erasure
//!   compacts it. Both are reached by base assignment and re-registration;
//!   erasure also by pool destruction. The capacity setter belongs to
//!   [`crate::raid_mutations`], whose detour forwards here.
//! - Wing addition writes the pool's wing vector and wing backlinks, reached
//!   by wing creation, creation at a base and transfer to a base. Wing removal,
//!   destruction of all wings (which retires without wing removal), and wing
//!   splitting (which appends without wing addition) also write them. The
//!   pool's virtual deleting destructor destroys its wings before erasing the
//!   pool from its base. It runs during the hourly empty-pool purge,
//!   withdrawal, and base and country teardown.
//! - Pool-definition rebind replaces a pool's subtype with its owner's current
//!   local definition clone; this invalidates the independent metadata carried
//!   with the registration segment. Construction and loading establish the
//!   initial subtype; other pool members and scalar fuel remain separate.
//! - Base destruction inlines container pool erases and retires both vectors.
//!   A later base can reuse its address with a different handle. This site
//!   belongs to [`crate::ship_model_mutations`], whose detour forwards here.
//! - A fresh pool has the shared null base reference when wing creation adds
//!   its first wing; subsequent pool registration names the base. Otherwise
//!   only constructors, the six save readers and four load completions write
//!   base, container, pool and wing identities. Loading invalidates the hour.
//! - Every site runs on the simulation thread. The hourly strategic-air stage
//!   joins its worker task group and asserts the main thread before relocation,
//!   purge and registration.
//!
//! AirBases reads, for each state base, the manager's three state-indexed
//! base slots (`cstrategic_air_manager::STATE_AIR_BASES`, `STATE_ROCKET_BASES`,
//! `STATE_GUN_BASES`), the base's identity (`cair_base::HANDLE`) and its host,
//! state, province, capacity and kind (`cair_base::HOST_REFERENCE`, `STATE`,
//! `PROVINCE`, `CAPACITY`, `KIND`), and the state's and province's identities
//! and the province's state link (`cstate::ID`, `cprovince::ID`,
//! `cprovince::STATE`). Stateless bases are read every hour.
//!
//! - Only the three state-base creators store a nonnull slot. Each returns the
//!   base of one kind in a state's slot, or creates one for an empty slot,
//!   registers its handle and stores it. They are reached from initial
//!   population, the building level observer and the province-controller
//!   listener, and name no base: each notifies the slots
//!   ([`Receiver::Manager`]). The building level observer nulls a rocket or
//!   gun slot right after destroying its base, whose forwarded destruction
//!   also notifies the slots.
//! - The state-base constructor writes host (the shared null reference),
//!   state, province, kind and capacity. Afterwards only capacity refresh
//!   writes capacity, from the constructors, building level changes, carrier
//!   creation, refit and ship equipment recalculation. Only load completion
//!   rewrites a state base's province; its host, state and kind have no other
//!   writer.
//! - State and province identities and the province's state link change only
//!   with load, game reset and the developer map editor; entering the editor
//!   is forwarded here by the building family.

pub use crate::advisor_mutations::MutationSite;

/// How a site's first argument (RCX) names the base it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the air base; its handle is `cair_base::HANDLE`.
    Base,
    /// RCX is an air-wing pool; its base's handle is `cair_wing_pool::AIR_BASE_REFERENCE`.
    Pool,
    /// RCX is the strategic-air manager; the site writes its state base slots.
    Manager,
}

impl Receiver {
    /// The offset of the base handle in RCX's object; `None` when RCX names
    /// no base.
    pub const fn handle(self) -> Option<usize> {
        match self {
            Self::Base => Some(crate::cair_base::HANDLE),
            Self::Pool => Some(crate::cair_wing_pool::AIR_BASE_REFERENCE),
            Self::Manager => None,
        }
    }
}

const CREATE_COUNTRY_CONTAINER_RVA: usize = 0xca_8ca0;
const REGISTER_POOL_RVA: usize = 0xca_9250;
const ERASE_POOL_RVA: usize = 0xcc_0700;
const DESTROY_POOL_RVA: usize = 0xfd_ef50;
/// Each state-base creator saves RBX in five relocated bytes.
const CREATE_STATE_BASE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];

/// Every site with its receiver; each takes at most four integer or pointer
/// arguments and returns RAX unchanged from its original.
pub const MUTATIONS: [(MutationSite, Receiver); 14] = [
    // Country-container creation (base, tag).
    (
        MutationSite {
            name: "create_country_container",
            rva: CREATE_COUNTRY_CONTAINER_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Base,
    ),
    // Country-access removal (base, tag).
    (
        MutationSite {
            name: "remove_country_access",
            rva: 0xcc_00c0,
            sentinel: [
                0x40, 0x53, 0x56, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x83, 0x3a, 0x00, 0x48, 0x8b,
                0xda, 0x48,
            ],
        },
        Receiver::Base,
    ),
    // Pool registration (base, pool, dedupe) returns a boolean.
    (
        MutationSite {
            name: "register_pool",
            rva: REGISTER_POOL_RVA,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83,
                0xec, 0x30,
            ],
        },
        Receiver::Base,
    ),
    // Pool erasure (base, pool) returns a boolean.
    (
        MutationSite {
            name: "erase_pool",
            rva: ERASE_POOL_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Base,
    ),
    // Wing addition (pool, wing, dedupe).
    (
        MutationSite {
            name: "add_wing",
            rva: 0xfd_f7e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Pool,
    ),
    // Wing removal (pool, wing, destroy) returns a boolean.
    (
        MutationSite {
            name: "remove_wing",
            rva: 0xfe_af30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x48,
            ],
        },
        Receiver::Pool,
    ),
    // Wing destruction (pool): stack-pointer copy to RAX and the RBX store
    // through it to RCX's home slot relocate together in seven bytes.
    (
        MutationSite {
            name: "destroy_all_wings",
            rva: 0xfe_2170,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70,
                0x18, 0x48,
            ],
        },
        Receiver::Pool,
    ),
    // Wing splitting (pool, wing, size, force) returns a wing.
    (
        MutationSite {
            name: "split_wing",
            rva: 0xfe_b9d0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
                0x18, 0x55,
            ],
        },
        Receiver::Pool,
    ),
    // Air-wing pool deleting destructor (pool, flags) returns the pool.
    (
        MutationSite {
            name: "destroy_pool",
            rva: DESTROY_POOL_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Pool,
    ),
    // Rebind the pool's subtype to its owner's current definition clone.
    (
        MutationSite {
            name: "rebind_pool_definition",
            rva: 0xfe_a690,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x49, 0x40, 0x48,
                0x81, 0xc1,
            ],
        },
        Receiver::Pool,
    ),
    // Ordinary state base of a state, created for an empty slot (manager, state).
    (
        MutationSite {
            name: "ensure_state_air_base",
            rva: 0xca_83c0,
            sentinel: CREATE_STATE_BASE_SENTINEL,
        },
        Receiver::Manager,
    ),
    // Rocket site of a state, created for an empty slot (manager, state).
    (
        MutationSite {
            name: "ensure_rocket_air_base",
            rva: 0xca_9190,
            sentinel: CREATE_STATE_BASE_SENTINEL,
        },
        Receiver::Manager,
    ),
    // Gun site of a state, created for an empty slot (manager, state).
    (
        MutationSite {
            name: "ensure_gun_air_base",
            rva: 0xca_8e90,
            sentinel: CREATE_STATE_BASE_SENTINEL,
        },
        Receiver::Manager,
    ),
    // Capacity refresh (base): saving RBX and RSI and the stack reservation
    // relocate in seven bytes.
    (
        MutationSite {
            name: "refresh_capacity",
            rva: 0xcc_2230,
            sentinel: [
                0x40, 0x53, 0x56, 0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0xf1, 0x48, 0x8b, 0x49, 0x68,
                0x48, 0x85,
            ],
        },
        Receiver::Base,
    ),
];
