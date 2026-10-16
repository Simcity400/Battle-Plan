//! Runtime writers of everything an AirBaseRegistrations subtree reads from
//! a state air base. Observing them selects bases to reread; recording never
//! calls them.
//!
//! Registration rows read the base's slot vector `+0x30` and dense list
//! `+0x48`; each listed or occupied country container's country `+0xB0`, pool
//! vector `+0xB8` and capacity `+0xD0`; each pool's vtable and wing vector
//! `+0x28`; and each wing's identity and pool backlink `+0x38`. Container
//! creation copies the base handle `+0x8` into container `+0xA8`. A pool's
//! base setter writes its base handle `+0x18` before registering it; a wing
//! names its pool directly. Each site names its changed base by that handle,
//! through the base in RCX (`+0x8`) or pool in RCX (`+0x18`). An unlinked
//! pool's handle is zero or the shared invalid reference, both before first
//! linking and after erasure. Such pools belong to no base's pool vector, so
//! their writes change no base rows. The collector carries a base only while
//! every pool it lists names that base.
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

pub use crate::advisor_mutations::MutationSite;

/// How a site's first argument (RCX) names the base it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the air base; its handle is at `+0x8`.
    Base,
    /// RCX is an air-wing pool; its base's handle is at `+0x18`.
    Pool,
}

impl Receiver {
    /// The offset of the base handle in RCX's object.
    pub const fn handle(self) -> usize {
        match self {
            Self::Base => crate::cair_base::HANDLE,
            Self::Pool => crate::cair_wing_pool::AIR_BASE_REFERENCE,
        }
    }
}

const CREATE_COUNTRY_CONTAINER_RVA: usize = 0xc3_c2b0;
const REGISTER_POOL_RVA: usize = 0xc3_c860;
const ERASE_POOL_RVA: usize = 0xc5_4220;
const DESTROY_POOL_RVA: usize = 0xf4_5860;

/// Every site with its receiver; each takes at most four integer or pointer
/// arguments and returns RAX unchanged from its original.
pub const MUTATIONS: [(MutationSite, Receiver); 9] = [
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
            rva: 0xc5_3be0,
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
            rva: 0xf4_60f0,
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
            rva: 0xf5_18c0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x48,
            ],
        },
        Receiver::Pool,
    ),
    // Wing destruction (pool): stack-pointer copy to RAX and RBX store at
    // RAX+8 relocate together in seven bytes.
    (
        MutationSite {
            name: "destroy_all_wings",
            rva: 0xf4_8a20,
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
            rva: 0xf5_2360,
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
];
