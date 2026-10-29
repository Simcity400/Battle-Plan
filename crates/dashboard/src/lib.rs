//! Widget data for the desktop dashboard: recording sources, data products,
//! the host/viewer protocol and the native data service. No map assets are
//! rendered here and no live game memory is read.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod allocator;
pub mod content_check;
pub mod prepared;
pub mod prepared_focus;
pub use dashboard_stability::prepared_stability;
pub mod products;
pub mod readiness;
pub mod reply_cache;
mod reply_dto;
pub mod service;

pub use dashboard_base::{
    catalogue, content, prepared_chart, prepared_measure, prepared_production, presentation,
    ranking, read_runs, research_dto::*, source,
};
pub(crate) use dashboard_core::invalid;
pub use dashboard_core::{blocks, config, dto::*, map_display, map_settings, protocol, transport};
pub(crate) use dashboard_text::party_names;
pub use dashboard_text::{
    character_scope, cryptology, effect_description, flag_choices, identity, metadata,
    modifier_text, naval_battle_name, scope, scripted_text, state_resources, triggers, unit_names,
};

pub use config::Config;
#[cfg(test)]
pub(crate) use products::{
    land_owners::land_owners,
    national_focus::{FocusCache, focus_branch, focus_history, focus_order},
};
pub use reply_dto::*;
pub use service::Service;
pub use source::{Source, SourceKey};

/// The generated TypeScript contract: shared types, then every product's
/// parameters and reply. One deterministic file, exported explicitly.
#[cfg(any(test, feature = "export-types"))]
pub fn typescript_contract() -> String {
    use ts_rs::TS;
    let mut declarations = vec![
        ReplayMetadata::decl(),
        CaptureMode::decl(),
        PartialRecording::decl(),
        DamagedRecording::decl(),
        BranchMetadata::decl(),
        RecordingGap::decl(),
        RecordingEventLoss::decl(),
        CountryMetadata::decl(),
        Artwork::decl(),
        FocusDatabase::decl(),
        LandOwners::decl(),
        FocusEntry::decl(),
        FocusOrder::decl(),
        FocusSegment::decl(),
        FocusProgress::decl(),
        FocusHistory::decl(),
        ResearchCoverage::decl(),
        ResearchOutcome::decl(),
        ResearchEntry::decl(),
        ResearchSlotState::decl(),
        ResearchDetails::decl(),
        ResearchModifiers::decl(),
        ResearchModifier::decl(),
        ResearchModifierSource::decl(),
        ResearchBonus::decl(),
        ResearchSlotHistory::decl(),
        ResearchProgress::decl(),
        ResearchHistory::decl(),
        ResearchFolder::decl(),
        ResearchTechnologyFolder::decl(),
        ResearchDefinitionText::decl(),
        products::research::prepared::ResearchStateSlot::decl(),
        products::research::prepared::ResearchState::decl(),
        products::research::prepared::ResearchStateReply::decl(),
        products::research::grants::GrantRestriction::decl(),
        products::research::grants::ResearchGrant::decl(),
        products::research::grants::GrantInventory::decl(),
        protocol::MapAudio::decl(),
        protocol::MapGraphics::decl(),
        protocol::MapGraphicsOverrides::decl(),
        protocol::MapAudio::decl(),
        protocol::MapAudioOverrides::decl(),
        protocol::CounterFilter::decl(),
        protocol::CameraMode::decl(),
        protocol::CampaignMapSettings::decl(),
        protocol::FlyAircraftStatus::decl(),
        protocol::FlyAircraftState::decl(),
        map_settings::RecordedMod::decl(),
        protocol::ResolvedMapSettings::decl(),
        map_settings::Reply::decl(),
        transport::HistoryCacheReply::decl(),
        transport::SourceReply::decl(),
        prepared::State::decl(),
        prepared::PauseReason::decl(),
        prepared::Action::decl(),
        prepared::Section::decl(),
        prepared::Reply::decl(),
        prepared::pipeline::Coverage::decl(),
        prepared::prefix::Prefix::decl(),
        prepared::Preferences::decl(),
        prepared_focus::PresentedFocus::decl(),
        prepared_focus::Pages::decl(),
        prepared_focus::Hour::decl(),
        prepared_focus::TableRefusal::decl(),
        prepared_focus::UnavailableHour::decl(),
        prepared_chart::ChartBucket::decl(),
        prepared_chart::ChartRun::decl(),
        prepared_chart::ChartSummary::decl(),
        prepared_chart::MarkCluster::decl(),
        prepared_chart::MarkSummary::decl(),
        prepared_measure::MeasureRun::decl(),
        prepared_measure::MeasureActor::decl(),
        prepared_measure::query::MeasureKey::decl(),
        prepared_measure::query::MeasureRead::decl(),
        prepared_measure::query::MeasureState::decl(),
        prepared_measure::query::MeasureStateReply::decl(),
        prepared_measure::query::MeasureSummaryLine::decl(),
        prepared_measure::query::MeasureSummaryReply::decl(),
        prepared_measure::query::MeasureKeyReply::decl(),
        prepared_measure::query::MeasureDropReply::decl(),
        prepared_measure::query::MeasureEvent::decl(),
        prepared_measure::query::MeasureMarkReply::decl(),
        prepared_production::SelectedValue::decl(),
        prepared_production::SelectedCountry::decl(),
        prepared_production::SelectionPage::decl(),
    ];
    for product in products::PRODUCTS {
        declarations.extend((product.types)());
    }
    // Products may share a type (CountryParams, LandOwners); declare each name
    // once. Two different types under one name would silently drop one, so
    // that refuses the contract.
    let mut seen = std::collections::BTreeMap::new();
    declarations.retain(|declaration| {
        let name = declaration
            .strip_prefix("type ")
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or_default()
            .to_owned();
        match seen.entry(name) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(declaration.clone());
                true
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                assert!(
                    entry.get() == declaration,
                    "two exported types are named `{}`; rename one",
                    entry.key()
                );
                false
            }
        }
    });
    let mut text = String::from(
        "// Generated by cargo run -p dashboard --features export-types --bin export-types. Do not edit.\n",
    );
    for declaration in declarations {
        text.push_str("export ");
        text.push_str(
            &declaration
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        text.push('\n');
    }
    text.push_str(&format!(
        "/** The native command byte budget, including its envelope. */\nexport const MAX_MESSAGE_BYTES = {};\n",
        protocol::MAX_MESSAGE_BYTES
    ));
    // The refusals that say nothing of the product, only that a host or the service had
    // no room: a reader tries these again; other errors remain failures until retry
    // or a content/configuration change. Each is named, so a frontend producer and tests use
    // the constant, never their own text.
    text.push_str(
        "/** Refusals for want of room: a message starting with one may be tried again. */\n",
    );
    for (name, message) in transport::TRANSIENT_ERROR_NAMES {
        text.push_str(&format!("export const {name} = {message:?};\n"));
    }
    let transient = transport::TRANSIENT_ERROR_NAMES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!(
        "/** Errors that only say there was no room: the request may be tried again. */\nexport const TRANSIENT_ERRORS = [{transient}] as const;\n"
    ));
    let content_products = products::PRODUCTS
        .iter()
        .filter(|product| product.content_dependent)
        .map(|product| format!("{:?}", product.key))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!(
        "/** Products whose replies depend on configured game content. */\nexport const CONTENT_PRODUCTS = [{content_products}] as const;\n"
    ));
    let interactive_products = products::PRODUCTS
        .iter()
        .filter(|product| product.interactive)
        .map(|product| format!("{:?}", product.key))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!(
        "/** Products served separately from whole-branch history reads. */\nexport const INTERACTIVE_PRODUCTS = [{interactive_products}] as const;\n"
    ));
    text.push_str(&format!(
        "/** How a country-colours reply past the product byte budget is refused: ask for fewer hours. */\nexport const COUNTRY_COLOURS_OVER_BUDGET = {:?};\n",
        products::country_colours::OVER_BUDGET
    ));
    text
}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod mod_fixtures;
#[cfg(test)]
pub(crate) use dashboard_base::test_fixtures;
