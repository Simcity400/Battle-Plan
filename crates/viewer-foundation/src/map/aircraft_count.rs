//! Frame-only aircraft counts and sortie efficiencies are shared with dashboard products.

use installed_content_core::defines::Defines;
use std::io;

pub use replay_derive::aircraft_count::*;

/// Read the scenario count and mine-admission defines from recorded content.
pub fn load_defines(defines: &Defines) -> io::Result<SortieDefines> {
    Ok(SortieDefines {
        fuel_penalty_raw: defines.fixed5("NAir", "MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL")?,
        missile_launcher_slots: defines.int32("NAir", "MISSILE_LAUNCHER_SLOTS")?.max(1),
        // An empty table refuses only the carrier wings whose stance it lacks.
        carrier_stance_ratios: defines
            .fixed5_array("NNavy", "CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO")?,
        mine_maximum_raw: defines
            .fixed5("NNavy", "NAVAL_MINES_IN_REGION_MAX")?
            .max(ONE),
    })
}
