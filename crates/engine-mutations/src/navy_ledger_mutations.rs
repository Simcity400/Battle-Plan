
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "register_mission",
        rva: Rva(0xf1_e560),
    },
    MutationSite {
        name: "unregister_mission",
        rva: Rva(0xf2_24a0),
    },
];

