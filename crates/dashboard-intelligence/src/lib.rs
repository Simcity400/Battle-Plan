//! Intelligence: agency comparison, the espionage log, decryption, the operatives roster and operative placements, with the content they read and the observer that prepares them.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports them, registers them and prepares
//! and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod agency_comparison;
    mod country_name_cache;
    pub mod decryption;
    pub mod espionage_log;
    pub mod intel_details;
    mod intel_explanations;
    mod intel_ranges;
    pub mod intelligence_capture;
    pub mod intelligence_content;
    pub mod intelligence_prepared;
    pub mod operation_description;
    pub mod operative_content;
    pub mod operative_placements;
    mod operative_rows;
    pub mod operatives_roster;

    dashboard_base::reply_warnings!(
        agency_comparison::AgencyComparisonReply,
        intel_details::IntelDetailsReply,
        decryption::DecryptionReply,
        espionage_log::EspionageLogReply,
        operation_description::OperationDescriptionReply,
        operative_placements::OperativePlacementReply,
        operatives_roster::OperativeRosterReply,
    );
}
