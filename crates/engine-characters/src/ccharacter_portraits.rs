//! Portrait-holder writer boundaries observing normal execution; recording
//! never invokes them for a value. Insertion and replacement include parsing
//! and public setters. Whole-holder copies and bulk entry copies have separate
//! boundaries. Construction and destruction invalidate lifetimes, including
//! address reuse. Renderer-cache payloads are excluded from recorded descriptors.

use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

pub const MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "assign_entry",
        rva: Rva(0x143aa50),
    },
    MutationSite {
        name: "copy_holder",
        rva: Rva(0x10238a0),
    },
    MutationSite {
        name: "construct",
        rva: Rva(0x1439f20),
    },
    MutationSite {
        name: "copy_construct",
        rva: Rva(0x345070),
    },
    MutationSite {
        name: "destroy",
        rva: Rva(0x1439f50),
    },
    MutationSite {
        name: "insert_range",
        rva: Rva(0x3bdd40),
    },
];
