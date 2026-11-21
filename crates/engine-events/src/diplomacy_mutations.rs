
use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

pub const MUTATIONS: [MutationSite; 4] = [
    MutationSite {
        name: "rebuild_wartime_allies",
        rva: Rva(0xdac3f0),
    },
    MutationSite {
        name: "diplomacy_reset",
        rva: Rva(0xd9dbf0),
    },
    MutationSite {
        name: "add_reciprocal_link",
        rva: Rva(0x718f60),
    },
    MutationSite {
        name: "remove_reciprocal_link",
        rva: Rva(0x756a70),
    },
];
