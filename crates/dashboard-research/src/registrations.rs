dashboard_base::register_products! {
    RESEARCH_RESEARCH => crate::products::research::Research,
    RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN => crate::products::research::breakdown::ResearchBreakdown,
    RESEARCH_HISTORY_RESEARCHHISTORYAT => crate::products::research::history::ResearchHistoryAt,
    SPECIAL_PROJECTS_SPECIALPROJECTS => crate::products::special_projects::SpecialProjects; ExactHourly(crate::products::special_projects::dated_panel),

    @hourly
    RESEARCH_SLOTS_RESEARCHSLOTS => exact<crate::products::research::slots::ResearchSlots>,
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTDETAILSAT => exact<crate::products::special_projects::details::SpecialProjectDetailsAt>,
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTEFFECTSAT => whole_frame<crate::products::special_projects::details::SpecialProjectEffectsAt>,
    SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTITERATIONAT => whole_frame<crate::products::special_projects::details::SpecialProjectIterationAt>,
    TECHNOLOGY_EFFECTS_TECHNOLOGYEFFECTSAT => whole_frame<crate::products::technology_effects::TechnologyEffectsAt>,
    ENTITY_STATS_ENTITYSTATSAT => whole_frame<crate::products::entity_stats::EntityStatsAt>,
}
