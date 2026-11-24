dashboard_base::register_products! {
    RESEARCH_RESEARCH => crate::products::research::Research,
    RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN => crate::products::research::breakdown::ResearchBreakdown,
    RESEARCH_HISTORY_RESEARCHHISTORYAT => crate::products::research::history::ResearchHistoryAt,
}

dashboard_base::register_hourly_products! {
    RESEARCH_SLOTS_RESEARCHSLOTS => crate::products::research::slots::ResearchSlots,
}

dashboard_base::register_products! {
    SPECIAL_PROJECTS_SPECIALPROJECTS => crate::products::special_projects::SpecialProjects,
}

dashboard_base::register_hourly_products! {
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTDETAILSAT => crate::products::special_projects::details::SpecialProjectDetailsAt,
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTEFFECTSAT => crate::products::special_projects::details::SpecialProjectEffectsAt,
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTITERATIONAT => crate::products::special_projects::details::SpecialProjectIterationAt,
    TECHNOLOGY_EFFECTS_TECHNOLOGYEFFECTSAT => crate::products::technology_effects::TechnologyEffectsAt,
    ENTITY_STATS_ENTITYSTATSAT => crate::products::entity_stats::EntityStatsAt,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_SPECIALPROJECTDETAILSAT: ExactPanel = exact_panel::<crate::products::special_projects::details::SpecialProjectDetailsAt>;
pub const EXACT_RESEARCHSLOTS: ExactPanel = exact_panel::<crate::products::research::slots::ResearchSlots>;
