//! Widget data for the desktop dashboard: recording sources, data products,
//! the host/viewer protocol and the native data service. No map assets are
//! rendered here and no live game memory is read.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod answer;
pub mod content_check;
pub mod products;
pub mod reply_check;
mod reply_dto;
pub mod service;

pub use dashboard_base::{catalogue, content, presentation, ranking, read_runs, source, span};
pub(crate) use dashboard_core::invalid;
pub use dashboard_core::{
    QueryError, blocks, config, dto::*, map_display, map_settings, protocol, refusal, refused,
    transport,
};
pub use dashboard_text::{
    artwork, character_scope, cryptology, effect_description, flag_choices, identity, metadata,
    modifier_text, naval_battle_name, scope, scripted_text, state_resources, triggers, unit_names,
};

pub use config::Config;
pub use reply_dto::*;
pub use service::Service;
pub use source::{Source, SourceKey};

/// The generated TypeScript contract: shared types, then every product's
/// parameters and reply. One deterministic file, exported explicitly.
#[cfg(any(test, feature = "export-types"))]
pub fn typescript_contract() -> String {
    #[cfg(any(test, feature = "export-types"))]
    use ts_rs::TS;
    let mut declarations = vec![
        QueryError::decl(),
        ReplayMetadata::decl(),
        CaptureMode::decl(),
        PartialRecording::decl(),
        DamagedRecording::decl(),
        BranchMetadata::decl(),
        BranchFork::decl(),
        JoinEvidence::decl(),
        BranchJoin::decl(),
        JoinChoice::decl(),
        RecordingGap::decl(),
        RecordingEventLoss::decl(),
        CountryMetadata::decl(),
        Artwork::decl(),
        FocusDatabase::decl(),
        FocusEntry::decl(),
        FocusOrder::decl(),
        FocusSegment::decl(),
        FocusProgress::decl(),
        FocusHistory::decl(),
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
        transport::SourceReply::decl(),
        dashboard_query::chart::ChartBucket::decl(),
        dashboard_query::chart::ChartColumns::decl(),
        dashboard_query::chart::ChartRun::decl(),
        dashboard_query::chart::ChartSummary::decl(),
        dashboard_query::chart::MarkCluster::decl(),
        dashboard_query::chart::MarkSummary::decl(),
        dashboard_query::series::MissingSpan::decl(),
        dashboard_query::identity::Country::decl(),
        dashboard_query::series::SeriesRun::decl(),
        dashboard_query::series::SeriesDrop::decl(),
        dashboard_query::series::SeriesState::decl(),
        dashboard_query::series::SeriesStateReply::decl(),
        dashboard_query::series::SeriesKeyReply::decl(),
        dashboard_query::series::SeriesEvent::decl(),
        dashboard_query::series::SeriesEventPage::decl(),
        dashboard_query::tile::TilePart::decl(),
        dashboard_query::tile::TileMarks::decl(),
        dashboard_query::tile::TileLine::decl(),
        dashboard_query::tile::SeriesTileReply::decl(),
        dashboard_query::series::NavigationKey::decl(),
        dashboard_query::series::SeriesRead::decl(),
        dashboard_query::series::SeriesLineAsk::decl(),
        dashboard_query::series::SeriesPartLife::decl(),
        dashboard_query::series::SeriesOwnerParts::decl(),
        dashboard_query::series::SeriesPartsReply::decl(),
        dashboard_query::series::ranked::RankedSpan::decl(),
        dashboard_query::series::ranked::SeriesRankedLine::decl(),
        dashboard_query::series::ranked::SeriesRankedReply::decl(),
        products::army::ArmyChartLine::decl(),
        products::army::ArmyChartReply::decl(),
    ];
    for product in products::PRODUCTS {
        declarations.extend(product.types.expect("every exported product registers its types")());
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
    text.push_str(&format!(
        "/** Data requests outstanding at once; one past it is refused for want of room. */\nexport const MAX_PENDING_REQUESTS = {};\n",
        transport::MAX_PENDING_REQUESTS
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
    let replay_products = products::PRODUCTS
        .iter()
        .filter(|product| product.replays)
        .map(|product| format!("{:?}", product.key))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!(
        "/** Pages that replay recorded tables to their rows' hours: served apart, so no other request waits behind them. */\nexport const REPLAY_PRODUCTS = [{replay_products}] as const;\n"
    ));
    {
        use map_display::option_defaults as defaults;
        text.push_str(&format!(
            "/** Each map option until the user chooses: the game's own default for those it saves in settings.txt, the app's for the rest. */\nexport const MAP_OPTION_DEFAULTS = {{ dayNight: {}, alliedPlans: {}, fog: {}, countryCounterColours: {}, playerCounters: {}, radar: {} }} as const;\n",
            defaults::DAY_NIGHT,
            defaults::ALLIED_PLANS,
            defaults::FOG,
            defaults::COUNTRY_COUNTER_COLOURS,
            defaults::PLAYER_COUNTERS,
            defaults::RADAR,
        ));
    }
    text
}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
pub(crate) use dashboard_base::test_fixtures;
