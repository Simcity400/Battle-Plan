dashboard_base::register_products! {
    RESEARCH_RESEARCH => crate::products::research::Research,
    RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN => crate::products::research::breakdown::ResearchBreakdown,
}

dashboard_base::register_hourly_products! {
    RESEARCH_SLOTS_RESEARCHSLOTS => crate::products::research::slots::ResearchSlots,
}
