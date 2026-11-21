
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "party_names",
        rva: Rva(0x123_2e70),
    },
    MutationSite {
        name: "construct_party",
        rva: Rva(0x123_0300),
    },
];
