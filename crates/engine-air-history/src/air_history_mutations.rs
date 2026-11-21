
use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "region_history",
        rva: Rva(0xcad2d0),
    },
    MutationSite {
        name: "advance_window",
        rva: Rva(0x15b2820),
    },
];
