//! Runtime writers of country air-history windows. Observing them selects the
//! histories to reread; recording never calls them.
//!
//! Adds allocate a missing history when needed and modify the current record.
//! Advances move the current slot, zero its new record and fold the window into
//! monthly and yearly windows. Construction builds the windows; construction
//! and loading rebuild record storage. Shared non-air history receivers are
//! never registered.

use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

/// Receivers: a country air object (with the region) for the first, a history's history's window
/// list for the second.
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
