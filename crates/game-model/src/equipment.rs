#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantStatEntry {
    pub index: u32,
    pub value: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantModuleMissionExclusion {
    pub module_token: u32,
    pub mask: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantMissionStats {
    pub mask: u32,
    pub stats: Vec<VariantStatEntry>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantNamedBonus {
    pub owner: u32,
    pub handle: u32,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantBonusSource {
    pub named: Option<VariantNamedBonus>,
    pub idea: Option<Vec<u8>>,
    pub idea_token: Option<u32>,
    pub stats: Vec<VariantStatEntry>,
    pub terrain_attrition: Vec<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct VariantDesignState<S = VariantBonusSource> {
    pub parent: Option<u64>,
    pub max_version: u8,
    pub is_frame: u8,
    pub manpower: i32,
    pub auto_upgraded: u8,
    pub highlight: u8,
    pub can_upgrade_type: u8,
    pub can_upgrade_variant: u8,
    pub can_upgrade_modules: u8,
    pub role_icon_index: i32,
    pub division_names_group: Option<Vec<u8>>,
    pub pending_parent_version: i16,
    pub module_priority_sum: i32,
    pub module_mission_exclusions: Vec<VariantModuleMissionExclusion>,
    pub terrain_attrition: Vec<i64>,
    pub niches: Vec<u32>,
    pub build_cost: i64,
    pub build_resources: Vec<VariantStatEntry>,
    pub design_team: u64,
    pub design_team_serialized: bool,
    pub design_team_traits: i32,
    pub design_team_bonus: Vec<VariantStatEntry>,
    pub named_bonuses: Vec<u32>,
    pub bonus_sources: Vec<S>,
    pub named_sources: Vec<S>,
    pub mission_stats: Vec<VariantMissionStats>,
    pub unresolved_stats: Vec<VariantStatEntry>,
}

impl<S> VariantDesignState<S> {
    pub fn with_sources<T>(
        self,
        bonus_sources: Vec<T>,
        named_sources: Vec<T>,
    ) -> VariantDesignState<T> {
        VariantDesignState {
            parent: self.parent,
            max_version: self.max_version,
            is_frame: self.is_frame,
            manpower: self.manpower,
            auto_upgraded: self.auto_upgraded,
            highlight: self.highlight,
            can_upgrade_type: self.can_upgrade_type,
            can_upgrade_variant: self.can_upgrade_variant,
            can_upgrade_modules: self.can_upgrade_modules,
            role_icon_index: self.role_icon_index,
            division_names_group: self.division_names_group,
            pending_parent_version: self.pending_parent_version,
            module_priority_sum: self.module_priority_sum,
            module_mission_exclusions: self.module_mission_exclusions,
            terrain_attrition: self.terrain_attrition,
            niches: self.niches,
            build_cost: self.build_cost,
            build_resources: self.build_resources,
            design_team: self.design_team,
            design_team_serialized: self.design_team_serialized,
            design_team_traits: self.design_team_traits,
            design_team_bonus: self.design_team_bonus,
            named_bonuses: self.named_bonuses,
            bonus_sources,
            named_sources,
            mission_stats: self.mission_stats,
            unresolved_stats: self.unresolved_stats,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[derive(replay_spool::Spooled)]
pub struct OrganisationBonus {
    pub key: u32,
    pub value_raw: i64,
}
