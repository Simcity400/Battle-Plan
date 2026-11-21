
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "add_combat",
        rva: Rva(0xee_aeb0),
    },
    MutationSite {
        name: "remove_combat",
        rva: Rva(0xef_1040),
    },
];
