//! Country-vector mutation boundaries for flying-aircraft map demand.
//! Observation tells the collector which countries to reread; recording never
//! calls these writers.
//!
//! Enemy and wartime-ally vectors rebuild for war creation, joining, ending,
//! capitulation and derived-state refreshes. Constructor, appearance, subject
//! and destruction resets clear both vectors. Reciprocal-donor links change
//! through their add/remove operations; loading fills them at a world discontinuity.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receivers: a diplomacy status for the first two, a country for the rest.
pub const MUTATIONS: [MutationSite; 4] = [
    MutationSite {
        name: "rebuild_wartime_allies",
        rva: 0xd31e20,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
            0x56, 0x41,
        ],
    },
    MutationSite {
        name: "diplomacy_reset",
        rva: 0xd25970,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x55,
        ],
    },
    MutationSite {
        name: "add_reciprocal_link",
        rva: 0x6c48d0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
            0x20, 0x55,
        ],
    },
    MutationSite {
        name: "remove_reciprocal_link",
        rva: 0x701c60,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
            0x18, 0x55,
        ],
    },
];
