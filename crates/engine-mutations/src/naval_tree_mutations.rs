
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const COUNTRY_SITES: usize = 2;

pub const MUTATIONS: [MutationSite; 7] = [
    MutationSite {
        name: "recalculate_modifiers",
        rva: Rva(0x722930),
    },
    MutationSite {
        name: "reset_country_appearance",
        rva: Rva(0x72a990),
    },
    MutationSite {
        name: "recalculate_leader_modifiers",
        rva: Rva(0xc77590),
    },
    MutationSite {
        name: "update_hq_scaled_modifiers",
        rva: Rva(0xc77be0),
    },
    MutationSite {
        name: "construct_leader",
        rva: Rva(0xc61da0),
    },
    MutationSite {
        name: "copy_construct_leader",
        rva: Rva(0xc61d00),
    },
    MutationSite {
        name: "destroy_leader",
        rva: Rva(0xc62e40),
    },
];

