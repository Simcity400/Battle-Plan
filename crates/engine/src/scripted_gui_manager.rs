//! Scripted GUI manager presentation layout.
//! Its callback lifetime does not establish an hourly walk.

/// Manager update entry.
pub const UPDATE_RVA: usize = 0x00b7_9c90;
/// Exact manager-update entry sentinel.
pub const UPDATE_SENTINEL: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
    0x48, 0x83, 0xec, 0x30,
];
/// Updater-pointer array.
pub const UPDATERS: usize = 0x20;
/// Updater count, u32, compared with observed callbacks.
pub const UPDATER_COUNT: usize = 0x2c;
/// Rebuild generation, u32.
pub const REBUILD_GENERATION: usize = 0x68;
/// Generation sentinel refused during transaction sealing; its writer is not established.
pub const INVALID_REBUILD_GENERATION: u32 = u32::MAX;
