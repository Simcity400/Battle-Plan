
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const COUNTRY_LEADER_MODIFIERS: usize = 0;
pub const COUNTRY_LEADER_DESTROY: usize = 1;
pub const COUNTRY_LEADER_MAP_INSERT: usize = 2;
pub const COUNTRY_LEADER_CREATE: usize = 3;
pub const ADVISOR_MODIFIERS: usize = 4;
pub const COUNTRY_LEADER_IDEOLOGY: usize = 5;
pub const CHARACTER_NAME: usize = 6;
pub const CHARACTER_REPAIR: usize = 7;

pub const MUTATIONS: [MutationSite; 8] = [
    MutationSite {
        name: "country_leader_modifiers",
        rva: Rva(0x1050ba0),
    },
    MutationSite {
        name: "country_leader_destroy",
        rva: Rva(0x104e6e0),
    },
    MutationSite {
        name: "country_leader_map_insert",
        rva: Rva(0x343ef0),
    },
    MutationSite {
        name: "country_leader_create",
        rva: Rva(0x104f8d0),
    },
    MutationSite {
        name: "advisor_modifiers",
        rva: Rva(0x1334c90),
    },
    MutationSite {
        name: "country_leader_ideology",
        rva: Rva(0x379e30),
    },
    MutationSite {
        name: "character_name",
        rva: Rva(0x102a770),
    },
    MutationSite {
        name: "character_repair",
        rva: Rva(0x1028950),
    },
];
