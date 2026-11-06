//! Writers of demanded division equipment-pool membership.
//! The design family forwards variant rebuild and assignment writers. Pool addition,
//! setting, scaling, clearing and assignment cover membership; subtraction
//! delegates to addition. Construction and loading revoke all certificates.
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;
pub const MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "add_pool",
        rva: Rva(0x1092970),
    },
    MutationSite {
        name: "add_variant",
        rva: Rva(0x1092b70),
    },
    MutationSite {
        name: "set_variant",
        rva: Rva(0x10980e0),
    },
    MutationSite {
        name: "scale_pool",
        rva: Rva(0x1096e40),
    },
    MutationSite {
        name: "clear_pool",
        rva: Rva(0x1093830),
    },
    MutationSite {
        name: "assign_pool",
        rva: Rva(0xccaca0),
    },
];
