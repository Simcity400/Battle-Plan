use dashboard_base::products::at_hour::{at_hour, before};

dashboard_base::register_products! {
    NATIONAL_FOCUS_NATIONALFOCUS => crate::products::national_focus::NationalFocus,
    TECHNOLOGY_ARTWORK_TECHNOLOGYARTWORK => crate::products::technology_artwork::TechnologyArtwork,
    LOGS_LOGTAIL => crate::products::logs::LogTail; LogTail(crate::products::logs::dated_panel),
    POLITICS_HISTORY_POLITICSSTATEHISTORY => crate::products::politics::history::PoliticsStateHistory,
    POLITICS_CIVIL_WARS_CIVILWARS => crate::products::politics::civil_wars::CivilWars,
    FOCUS_CARD_FOCUSCARD => crate::products::focus_card::FocusCard; interval(at_hour, |r| r.valid_through),
    IDEAS_IDEAHISTORY => crate::products::ideas::IdeaHistory; interval(at_hour, |r| before(r.next_change)),
    PARTY_HISTORY_PARTYHISTORY => crate::products::party_history::PartyHistory,
    STABILITY_MARKERS_STABILITYMARKERS => crate::products::stability_markers::StabilityMarkers; interval(at_hour, |r| before(r.next_change)),
    FEED_EVENTFEEDTAIL => crate::products::feed::EventFeedTail; LogTail(crate::products::feed::event_feed_dated_panel),
    FEED_EVENTCHOICESTAIL => crate::products::feed::EventChoicesTail; LogTail(crate::products::feed::event_choices_dated_panel),
    FEED_DECISIONSTAIL => crate::products::feed::DecisionsTail; LogTail(crate::products::feed::decisions_dated_panel),
    FEED_EVENTFEEDACTIVITY => crate::products::feed::EventFeedActivity,
    FEED_EVENTFEEDKINDS => crate::products::feed::EventFeedKinds,
    POWER_LEDGER_BREAKDOWN_POWERSPENDINGBREAKDOWN => crate::products::power_ledger::breakdown::PowerSpendingBreakdown; interval(at_hour, |r| r.through),
    POWER_LEDGER_MARKERS_POWERSPENDMARKERS => crate::products::power_ledger::markers::PowerSpendMarkers; interval(at_hour, |r| before(r.next_change)),

    @hourly
    POLITICS_STATE_POLITICSSTATE => exact<crate::products::politics::state::PoliticsState>,
    POLITICS_ELECTIONS_NEXTELECTIONS => exact<crate::products::politics::elections::NextElections>,
    STABILITY_COUNTRYSTABILITYBREAKDOWN => exact<crate::products::stability::CountryStabilityBreakdown>,
    STABILITY_STABILITYPARTNAMES => exact<crate::products::stability::StabilityPartNames>,
}
