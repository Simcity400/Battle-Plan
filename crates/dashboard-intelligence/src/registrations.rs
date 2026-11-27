dashboard_base::register_products! {
    INTELLIGENCE_ESPIONAGE_TAIL_ESPIONAGELOGTAIL => crate::products::intelligence::espionage_tail::EspionageLogTail; LogTail(crate::products::intelligence::espionage_tail::dated_panel),
    RAIDS_RAIDLOG => crate::products::raids::RaidLog,
    RAIDS_TAIL_RAIDLOGTAIL => crate::products::raids::tail::RaidLogTail; LogTail(crate::products::raids::tail::dated_panel),
    RAIDS_RAIDTALLY => crate::products::raids::tally::RaidTally,
    RAIDS_RAIDTALLYAT => crate::products::raids::RaidTallyAt,

    @hourly
    INTELLIGENCE_AGENCIES_AGENCYCOMPARISON => exact<crate::products::intelligence::agencies::AgencyComparison>,
    INTELLIGENCE_DECRYPTION_DECRYPTION => exact<crate::products::intelligence::decryption::Decryption>,
    INTELLIGENCE_OPERATIVE_MATRIX_OPERATIVEMATRIX => exact<crate::products::intelligence::operative_matrix::OperativeMatrix>,
    INTELLIGENCE_ROSTER_OPERATIVESROSTER => exact<crate::products::intelligence::roster::OperativesRoster>,
    INTEL_COVERAGE_INTELNETWORKCOVERAGE => exact<crate::products::intel_coverage::IntelNetworkCoverage>,
}
