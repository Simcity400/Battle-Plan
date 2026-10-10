//! Relation-opinion writer observation sites. They tell collector which
//! statuses to reread through `crelation_status::Baseline`; recording never
//! calls them.
//!
//! Recomputation stores unconditionally on daily workers as well as the main
//! thread, so observers must be thread-safe and mark only changed values. Reset
//! can run during civil war, annexation, state transfer or dynamic-country reuse.
//! Loading and construction run while game state is loaded or rebuilt. Statuses
//! are allocated and freed with their diplomacy owner and are not copied whole.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receivers (rcx): a live `CRelationStatus*` for all but the constructor,
/// whose receiver is its raw 0x9C0-byte storage. None takes more than four
/// integer arguments or any floating-point argument.
pub const MUTATIONS: [MutationSite; 4] = [
    // Opinion recomputation with a status receiver.
    MutationSite {
        name: "recompute_opinion",
        rva: 0xd19090,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x51, 0xfb,
            0x8a, 0x02,
        ],
    },
    // Status reset with two byte arguments and an integer pointer.
    MutationSite {
        name: "reset",
        rva: 0xd09d50,
        sentinel: [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
            0x55, 0x53,
        ],
    },
    // Status token loading with reader and token arguments.
    MutationSite {
        name: "load_token",
        rva: 0xd16ae0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x50, 0x41,
            0x8b, 0xd8,
        ],
    },
    // Status construction with diplomacy and target arguments.
    MutationSite {
        name: "construct",
        rva: crate::crelation_status::CONSTRUCTOR_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x56,
        ],
    },
];
