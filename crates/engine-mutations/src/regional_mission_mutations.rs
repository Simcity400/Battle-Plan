
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "register_mission",
        rva: Rva(0xcb_a880),
    },
    MutationSite {
        name: "unregister_mission",
        rva: Rva(0xcb_aa90),
    },
];

