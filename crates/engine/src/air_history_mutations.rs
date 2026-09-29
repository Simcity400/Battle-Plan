//! The runtime writers of the country air histories `141237D20` reads
//! (`CStrategicAir+0x158[region]`, window `CLoopHistory+0x10`), for
//! hash-pinned a729. Observing them tells the collector which histories to
//! re-read; recording never calls them.
//!
//! - Adds: `140C4CE80`, `140C4D180`, `140C4D620`, `140C4D7F0`, `140C4D9B0`,
//!   `140C4DE50`, `140C4E010` and `140C4E1A0` all reach the history through
//!   `140C40870` (which also allocates a missing one) and add to the window's
//!   current record (`GetNestedQword10_30@141504620`,
//!   `AddClampedInt64ArrayValue@141504630`). The loader
//!   `CStrategicAir::vf4@140C51F00` is its only other caller.
//! - Advances: `141504670` → `1415050B0` moves the window's current slot,
//!   zeroes the new current record and folds the window into the monthly and
//!   yearly ones. Its air callers are the periodic pass `140C3A9C0` and
//!   `140C41D30`, which starts a null current record.
//! - `AdvanceLoopHistoryWindows@141504690` only builds the windows of a new
//!   history (from `140C40870`); `RebuildRecordStorage@141505750` runs only
//!   from the constructor and the loader.
//!
//! No other runtime writer exists. `1415050B0` also advances non-air
//! histories; those receivers are never registered.

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
