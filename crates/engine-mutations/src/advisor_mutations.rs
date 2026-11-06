//! Advisor mutation boundaries for the supported executable. Observers never
//! invoke game functions to obtain data.
//!
//! Construction, destruction, idea changes and role parsing cover captured role
//! fields. Factories finish initialization after construction; map publication
//! is a separate boundary. Insertion helpers are shared with templates: observe
//! exact map addresses, without inferring the receiver type. Merge has its own
//! boundary because it inlines insertion. Range erase and recursive clear destroy
//! values; character destruction also covers empty maps. Fresh-read diagnostics
//! do not replace complete writer coverage.

use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

const CONSTRUCT_RVA: Rva = Rva(0x132da90);
const DESTROY_RVA: Rva = Rva(0x132dd10);

pub const MUTATIONS: [MutationSite; 11] = [
    MutationSite {
        name: "construct",
        rva: CONSTRUCT_RVA,
    },
    MutationSite {
        name: "destroy",
        rva: DESTROY_RVA,
    },
    MutationSite {
        name: "set_idea",
        rva: Rva(0x1334a80),
    },
    MutationSite {
        name: "read_role",
        rva: Rva(0x13338f0),
    },
    MutationSite {
        name: "insert_map",
        rva: Rva(0x342750),
    },
    MutationSite {
        name: "insert_map_value",
        rva: Rva(0x1022db0),
    },
    MutationSite {
        name: "erase_map",
        rva: Rva(0x3d2a00),
    },
    MutationSite {
        name: "clear_map",
        rva: Rva(0x342af0),
    },
    MutationSite {
        name: "merge_character",
        rva: Rva(0x1025580),
    },
    MutationSite {
        name: "read_character",
        rva: Rva(0x1029640),
    },
    MutationSite {
        name: "destroy_character",
        rva: Rva(0x1023710),
    },
];
