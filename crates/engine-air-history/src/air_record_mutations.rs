//! The runtime writer of the retained regional air-combat records (the country air object's
//! region-combat table, by region, [`crate::cair_region_combat_data`]). Observing it selects
//! holders to reread; recording never calls it.
//!
//! - The record updater is the only writer of a record: it finds or appends
//!   the record through a helper only it calls, raises the plane high-water mark
//!   with attribution, copied equipment and copied flag, ORs the mission mask and adds a
//!   positive amount to a damage vector through another helper only it calls,
//!   then clamps the first cause-3 entry. Its ten callers obtain the holder
//!   through the regional-holder lookup, which allocates a missing one; the
//!   lookup's only other caller is the country air object's load reader.
//! - Pruning removes each vector's leading records dated before the game
//!   state's clock minus the configured air-log timeout in hours.
//!   Its only caller is the country air daily pass, dispatched by the air
//!   manager from the game state's daily update. Playback derives that
//!   pruning instead of observing it.
//!
//! The updater's caller set runs inside the hourly tick after the clock
//! advance, so every record is dated at or after the previous sample's hour.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receiver: the regional air-combat holder in RCX.
pub const MUTATIONS: [MutationSite; 1] = [MutationSite {
    name: "update_record",
    rva: crate::cair_region_combat_data::UPDATE_RECORD_RVA,
    // Saves RBX/RBP/R15, reserves 0x40 stack bytes and copies R9 into RBX. Sign-extends R8D into
    // R15, then zero-extends the byte in its stack argument slot into R9D; the sentinel includes
    // the first byte of that last instruction.
    sentinel: [
        0x40, 0x53, 0x55, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40, 0x49, 0x8b, 0xd9, 0x4d, 0x63, 0xf8,
        0x44,
    ],
}];
