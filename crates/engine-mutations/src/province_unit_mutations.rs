
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "add_unit",
        rva: Rva(0xee_b1d0),
    },
    MutationSite {
        name: "remove_unit",
        rva: Rva(0xef_10d0),
    },
];

