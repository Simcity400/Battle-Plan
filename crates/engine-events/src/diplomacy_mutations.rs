//! Country-vector mutation boundaries for flying-aircraft map demand.
//! Observation tells the collector which countries to reread; recording never
//! calls these writers.
//!
//! Enemy and wartime-ally vectors rebuild for war creation, joining, ending,
//! capitulation and derived-state refreshes. Constructor, appearance, subject
//! and destruction resets clear both vectors. Reciprocal-donor links change
//! through their add/remove operations; loading fills them at a world discontinuity.

use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

/// Receivers: a diplomacy status for the first two, a country for the rest.
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
