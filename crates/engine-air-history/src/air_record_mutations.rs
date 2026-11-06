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

use engine_sites::Rva;

pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

/// Receiver: the regional air-combat holder in RCX.
pub const MUTATIONS: [MutationSite; 1] = [MutationSite {
    name: "update_record",
    rva: crate::cair_region_combat_data::UPDATE_RECORD_RVA,
}];
