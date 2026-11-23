
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Base,
    Pool,
    Manager,
}

impl Receiver {
    pub const fn handle(self) -> Option<usize> {
        match self {
            Self::Base => Some(engine_native_parts::cair_base_layout::HANDLE),
            Self::Pool => Some(engine_native_parts::cair_wing_pool::AIR_BASE_REFERENCE),
            Self::Manager => None,
        }
    }
}

const CREATE_COUNTRY_CONTAINER_RVA: Rva = Rva(0xca_8ca0);
const REGISTER_POOL_RVA: Rva = Rva(0xca_9250);
const ERASE_POOL_RVA: Rva = Rva(0xcc_0700);
const DESTROY_POOL_RVA: Rva = Rva(0xfd_ef50);
pub const MUTATIONS: [(MutationSite, Receiver); 14] = [
    (
        MutationSite {
            name: "create_country_container",
            rva: CREATE_COUNTRY_CONTAINER_RVA,
        },
        Receiver::Base,
    ),
    (
        MutationSite {
            name: "remove_country_access",
            rva: Rva(0xcc_00c0),
        },
        Receiver::Base,
    ),
    (
        MutationSite {
            name: "register_pool",
            rva: REGISTER_POOL_RVA,
        },
        Receiver::Base,
    ),
    (
        MutationSite {
            name: "erase_pool",
            rva: ERASE_POOL_RVA,
        },
        Receiver::Base,
    ),
    (
        MutationSite {
            name: "add_wing",
            rva: Rva(0xfd_f7e0),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "remove_wing",
            rva: Rva(0xfe_af30),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "destroy_all_wings",
            rva: Rva(0xfe_2170),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "split_wing",
            rva: Rva(0xfe_b9d0),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "destroy_pool",
            rva: DESTROY_POOL_RVA,
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "rebind_pool_definition",
            rva: Rva(0xfe_a690),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "ensure_state_air_base",
            rva: Rva(0xca_83c0),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "ensure_rocket_air_base",
            rva: Rva(0xca_9190),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "ensure_gun_air_base",
            rva: Rva(0xca_8e90),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "refresh_capacity",
            rva: Rva(0xcc_2230),
        },
        Receiver::Base,
    ),
];
