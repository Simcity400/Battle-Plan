
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Building,
    Status,
    Structure,
}

const SET_LEVEL_RVA: Rva = Rva(0x1163f20);

pub const MUTATIONS: [(MutationSite, Receiver); 8] = [
    (
        MutationSite {
            name: "set_level",
            rva: SET_LEVEL_RVA,
        },
        Receiver::Building,
    ),
    (
        MutationSite {
            name: "apply_damage_step",
            rva: Rva(0x1163d70),
        },
        Receiver::Building,
    ),
    (
        MutationSite {
            name: "advance_repair",
            rva: Rva(0x1162a00),
        },
        Receiver::Building,
    ),
    (
        MutationSite {
            name: "destroy_healthy_levels",
            rva: Rva(0x1163a90),
        },
        Receiver::Building,
    ),
    (
        MutationSite {
            name: "get_or_create_building",
            rva: Rva(0x11ffa90),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "read_building_status",
            rva: Rva(0x1205680),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "clear_building_status",
            rva: Rva(0x12016b0),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "enter_nudge_mode",
            rva: Rva(0x1363f00),
        },
        Receiver::Structure,
    ),
];
