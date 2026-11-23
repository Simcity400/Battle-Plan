use crate::value::value_struct;

pub use game_model::equipment::{
    VariantBonusSource, VariantMissionStats, VariantModuleMissionExclusion, VariantNamedBonus,
    VariantStatEntry,
};
pub type VariantDesignState = game_model::equipment::VariantDesignState<u64>;
type NativeVariantDesignState = game_model::equipment::VariantDesignState;
value_struct!(VariantStatEntry {
    index: u32,
    value: i64
});
value_struct!(VariantModuleMissionExclusion {
    module_token: u32,
    mask: u32
});
value_struct!(VariantMissionStats { mask: u32, stats: Vec<VariantStatEntry> });
value_struct!(VariantNamedBonus { owner: u32, handle: u32, name: Vec<u8>, description: Vec<u8> });
value_struct!(VariantBonusSource { named: Option<VariantNamedBonus>, idea: Option<Vec<u8>>, idea_token: Option<u32>, stats: Vec<VariantStatEntry>, terrain_attrition: Vec<i64> });
macro_rules! design_value {
    ($state:ty, $sources:ty) => { value_struct!($state {
    parent: Option<u64>, max_version: u8, is_frame: u8, manpower: i32,
    auto_upgraded: u8, highlight: u8, can_upgrade_type: u8,
    can_upgrade_variant: u8, can_upgrade_modules: u8, role_icon_index: i32,
    division_names_group: Option<Vec<u8>>, pending_parent_version: i16,
    module_priority_sum: i32, module_mission_exclusions: Vec<VariantModuleMissionExclusion>,
    terrain_attrition: Vec<i64>, niches: Vec<u32>, build_cost: i64,
    build_resources: Vec<VariantStatEntry>, design_team: u64, design_team_serialized: bool, design_team_traits: i32,
    design_team_bonus: Vec<VariantStatEntry>, named_bonuses: Vec<u32>,
    bonus_sources: $sources, named_sources: $sources, mission_stats: Vec<VariantMissionStats>, unresolved_stats: Vec<VariantStatEntry>
    }); };
}
design_value!(VariantDesignState, Vec<u64>);
design_value!(NativeVariantDesignState, Vec<VariantBonusSource>);
opaque_value!(VariantDesignState);

opaque_value!(VariantBonusSource);
