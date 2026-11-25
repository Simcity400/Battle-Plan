dashboard_base::register_products! {
    INTELLIGENCE_ESPIONAGE_TAIL_ESPIONAGELOGTAIL => crate::products::intelligence::espionage_tail::EspionageLogTail,
}

dashboard_base::register_hourly_products! {
    INTELLIGENCE_AGENCIES_AGENCYCOMPARISON => crate::products::intelligence::agencies::AgencyComparison,
    INTELLIGENCE_DECRYPTION_DECRYPTION => crate::products::intelligence::decryption::Decryption,
    INTELLIGENCE_OPERATIVE_MATRIX_OPERATIVEMATRIX => crate::products::intelligence::operative_matrix::OperativeMatrix,
    INTELLIGENCE_ROSTER_OPERATIVESROSTER => crate::products::intelligence::roster::OperativesRoster,
}

dashboard_base::register_products! {
    RAIDS_RAIDLOG => crate::products::raids::RaidLog,
    RAIDS_TAIL_RAIDLOGTAIL => crate::products::raids::tail::RaidLogTail,
    RAIDS_RAIDTALLY => crate::products::raids::tally::RaidTally,
    RAIDS_RAIDTALLYAT => crate::products::raids::RaidTallyAt,
}

dashboard_base::register_hourly_products! {
    INTEL_COVERAGE_INTELNETWORKCOVERAGE => crate::products::intel_coverage::IntelNetworkCoverage,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_AGENCYCOMPARISON: ExactPanel = exact_panel::<crate::products::intelligence::agencies::AgencyComparison>;
pub const EXACT_INTELNETWORKCOVERAGE: ExactPanel = exact_panel::<crate::products::intel_coverage::IntelNetworkCoverage>;
