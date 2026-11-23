use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::typed_codec;
use replay_rows::{opaque_value, value_struct};
use std::io;

mod raids_projects;
pub use raids_projects::*;

pub use schema::*;
mod schema;

#[doc(hidden)]
pub mod ops {
    use super::*;
    replay_rows::row_ops! {
        ProjectPrograms => ProjectProgramRow,
        Projects => ProjectRow,
        ProjectHistory => ProjectHistoryRow,
        ProjectBreakthroughs => ProjectBreakthroughRow,
        Nukes => NukeRow,
        RaidCountries => RaidCountryRow,
        Raids => RaidRow,
        RaidTargets => RaidTargetRow,
        RaidTargetStatuses => RaidTargetStatusRow,
        RaidCooldowns => RaidCooldownRow,
        RaidPathDistances => RaidPathDistanceRow,
        RaidCountryInputs => RaidCountryInputRow,
        RaidAirSites => RaidAirSiteRow,
        AceRangeInputs => AceRangeInputRow,
        RaidPlayerAllowedTypes => RaidPlayerAllowedTypeRow,
        RaidSpeedInputs => RaidSpeedInputRow,
        RaidEquipment => RaidEquipmentRow,
        RaidFacilities => RaidFacilityRow,
        RaidAirScores => RaidAirScoreRow,
        RaidIntelInputs => RaidIntelInputRow,
        RaidTooltipOperands => RaidTooltipOperandRow,
    }
}
