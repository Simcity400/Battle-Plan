//! Runtime writers of country air-history windows. Observing them selects the
//! histories to reread; recording never calls them.
//!
//! Adds allocate a missing history when needed and modify the current record.
//! Advances move the current slot, zero its new record and fold the window into
//! monthly and yearly windows. Construction builds the windows; construction
//! and loading rebuild record storage. Shared non-air history receivers are
//! never registered.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receivers: a CStrategicAir (with the region) for the first, a history's
/// `CLoopHistory+8` window list for the second.
pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "region_history",
        rva: 0xc40870,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
    MutationSite {
        name: "advance_window",
        rva: 0x15050b0,
        sentinel: [
            0x48, 0x89, 0x6c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x51, 0x08,
            0x33, 0xed,
        ],
    },
];
