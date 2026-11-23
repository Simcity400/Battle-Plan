use super::*;
use crate::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationAddedTarget {
    pub value: i64,
    pub controller: i32,
    pub occupied: i32,
    pub localisation: Vec<u8>,
}
value_struct!(OccupationAddedTarget { value: i64, controller: i32, occupied: i32, localisation: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationEffect {
    pub key: u32,
    pub value: i64,
}
value_struct!(OccupationEffect {
    key: u32,
    value: i64
});

pub use game_model::occupation_effect::EffectExtra as OccupationEffectExtra;
value_struct!(OccupationEffectExtra {
    child_sources: u32,
    custom_tooltips: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationHover {
    pub exile_host: i32,
    pub legitimacy: i64,
    pub flow_from: u32,
    pub flow: i64,
    pub gain_ratio: i64,
    pub added: Vec<OccupationAddedTarget>,
    pub resistance_effects: Vec<OccupationEffect>,
    pub compliance_effects: Vec<OccupationEffect>,
    pub resistance_extra: OccupationEffectExtra,
    pub compliance_extra: OccupationEffectExtra,
}
value_struct!(OccupationHover { exile_host: i32, legitimacy: i64, flow_from: u32, flow: i64, gain_ratio: i64, added: Vec<OccupationAddedTarget>, resistance_effects: Vec<OccupationEffect>, compliance_effects: Vec<OccupationEffect>, resistance_extra:OccupationEffectExtra, compliance_extra:OccupationEffectExtra });
opaque_value!(Recorded<Option<OccupationHover>>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationModifierSource {
    pub section: u8,
    pub source: u8,
    pub key: u32,
    pub total: Option<i64>,
    pub nodes: Vec<ModifierBreakdownNode>,
    pub failure: Vec<u8>,
}
value_struct!(OccupationModifierSource { section: u8, source: u8, key: u32, total: Option<i64>, nodes: Vec<ModifierBreakdownNode>, failure: Vec<u8> });
opaque_value!(Recorded<Vec<OccupationModifierSource>>);

pub use game_model::occupation_map::Markers as OccupationMap;
value_struct!(OccupationMap { stored_target: Option<i64>, core_country: Option<u32>, resistance: Vec<Vec<u8>>, compliance: Vec<Vec<u8>> });
opaque_value!(Recorded<Option<OccupationMap>>);
