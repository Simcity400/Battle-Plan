
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Pool,
    Tokens,
    Unnamed,
}

const DESTROY_POOL_RVA: Rva = Rva(0x1170690);
const UNAPPOINT_ADVISOR_RVA: Rva = Rva(0x1172b90);

pub const MUTATIONS: [(MutationSite, Receiver); 15] = [
    (
        MutationSite {
            name: "update_advisor_status",
            rva: Rva(0x1176330),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "add_advisor",
            rva: Rva(0x1170bc0),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "retire_character",
            rva: Rva(0x1175260),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "transfer_characters",
            rva: Rva(0x1175a10),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "remove_advisor_role",
            rva: Rva(0x374f80),
        },
        Receiver::Unnamed,
    ),
    (
        MutationSite {
            name: "construct_pool",
            rva: Rva(0x1170440),
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
            name: "update_advisor_tokens",
            rva: Rva(0x702600),
        },
        Receiver::Tokens,
    ),
    (
        MutationSite {
            name: "clear_character_manager",
            rva: Rva(0x7016f0),
        },
        Receiver::Tokens,
    ),
    (
        MutationSite {
            name: "construct_character_manager",
            rva: Rva(0x6fc660),
        },
        Receiver::Tokens,
    ),
    (
        MutationSite {
            name: "destroy_character_manager",
            rva: Rva(0x6fc920),
        },
        Receiver::Tokens,
    ),
    (
        MutationSite {
            name: "insert_appointed_advisor",
            rva: Rva(0x1171600),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "register_character",
            rva: Rva(0x1171070),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "copy_retired_characters",
            rva: Rva(0x1172250),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "unappoint_advisor",
            rva: UNAPPOINT_ADVISOR_RVA,
        },
        Receiver::Pool,
    ),
];
