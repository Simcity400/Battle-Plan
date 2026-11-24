dashboard_base::register_products! {
    NATIONAL_FOCUS_NATIONALFOCUS => crate::products::national_focus::NationalFocus,
    TECHNOLOGY_ARTWORK_TECHNOLOGYARTWORK => crate::products::technology_artwork::TechnologyArtwork,
    LOGS_LOGPAGE => crate::products::logs::LogPage,
    LOGS_LOGTAIL => crate::products::logs::LogTail,
    LOGS_LOGCOUNTS => crate::products::logs::LogCounts,
    POLITICS_HISTORY_POLITICSSTATEHISTORY => crate::products::politics::history::PoliticsStateHistory,
    POLITICS_CIVIL_WARS_CIVILWARS => crate::products::politics::civil_wars::CivilWars,
    FOCUS_CARD_FOCUSCARD => crate::products::focus_card::FocusCard,
    IDEAS_IDEAHISTORY => crate::products::ideas::IdeaHistory,
    PARTY_HISTORY_PARTYHISTORY => crate::products::party_history::PartyHistory,
    STABILITY_MARKERS_STABILITYMARKERS => crate::products::stability_markers::StabilityMarkers,
    FEED_EVENTFEEDTAIL => crate::products::feed::EventFeedTail,
    FEED_EVENTCHOICESTAIL => crate::products::feed::EventChoicesTail,
    FEED_DECISIONSTAIL => crate::products::feed::DecisionsTail,
    FEED_EVENTFEEDSPAN => crate::products::feed::EventFeedSpan,
    POWER_LEDGER_BREAKDOWN_POWERSPENDINGBREAKDOWN => crate::products::power_ledger::breakdown::PowerSpendingBreakdown,
}

dashboard_base::register_hourly_products! {
    POLITICS_STATE_POLITICSSTATE => crate::products::politics::state::PoliticsState,
    POLITICS_ELECTIONS_NEXTELECTIONS => crate::products::politics::elections::NextElections,
    STABILITY_COUNTRYSTABILITYBREAKDOWN => crate::products::stability::CountryStabilityBreakdown,
    STABILITY_STABILITYPARTNAMES => crate::products::stability::StabilityPartNames,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_POLITICSSTATE: ExactPanel = exact_panel::<crate::products::politics::state::PoliticsState>;
