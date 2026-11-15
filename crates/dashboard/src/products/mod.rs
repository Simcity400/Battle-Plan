//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply. The service,
//! transport, host and frontend never name a product; they route by its key. A declared series
//! answers its requests with no product of its own ([`crate::answer`]).

pub mod air_aces;
pub mod air_ledger;
pub mod air_statistics;
pub mod aircraft_catalogue;
pub mod army;
pub mod at_hour;
pub mod campaign_clock;
pub mod casualties;
pub mod construction;
pub mod country_flags;
pub mod country_people;
pub mod country_ids;
pub mod diplomacy;
pub mod doctrine_mastery;
pub mod doctrines;
pub mod economy;
pub mod focus;
pub mod focus_artwork;
pub mod groups;
pub mod ideas;
pub mod identities;
pub mod intel_coverage;
pub mod intelligence;
pub mod land_owners;
pub mod leaders;
pub mod lexicon;
pub mod logs;
pub mod minefields;
pub mod party_history;
pub mod national_focus;
pub mod occupation;
pub mod occupation_reserve;
pub mod naval_management;
pub mod naval_supremacy;
pub mod naval_yards;
pub mod place_names;
pub mod peace;
pub mod player_apm;
pub mod politics;
pub mod scalars;
pub mod raids;
pub mod production;
pub mod sea_regions;
pub mod regional_weather;
pub mod research;
pub mod stockpile;
pub mod styles;
pub mod subjects;
pub mod technology_artwork;
pub mod territory;
pub mod factions;
// Forces and people (P1).
pub mod experience;
pub mod forces;
pub mod training_queue;
pub mod volunteers;
// Divisions and stockpile (P3a).
pub mod divisions;
// Production totals and spending (P8b).
pub mod production_spending;
#[cfg(test)]
mod war_fixtures;
pub mod wargoals;
// Stability's breakdown and parts (P1b).
pub mod stability;
// Fuel, trade flows and convoy health (P7b).
pub mod convoys;
pub mod trade_flows;
// Land battles and the division log (P9a).
pub mod division_log;
pub mod land_battles;
pub mod combat_log;
pub mod land_terrain;
// Naval battles, ships sunk and naval losses (P9b).
pub mod naval;
// The event feed (P6).
pub mod feed;
pub mod air_missions;
// Factions' state (P5b).
pub mod faction_state;
// Wars (P5d).
pub mod wars;
// Special projects (P12c).
pub mod special_projects;
// Air superiority (P11d).
pub mod air_superiority;
// Equipment losses (P14).
pub mod army_losses;
pub mod equipment_losses;
// The scoreboard (P13).
pub mod scoreboard;
// Equipment produced (P8d).
pub mod equipment_produced;
// The power ledger (P4c).
pub mod power_ledger;
pub mod windows;

// The product contract and the shared products, at their old paths.
pub use dashboard_base::products::*;

use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::{
    Source,
    transport::{MAX_REPLY_BYTES, bounded_json},
};

dashboard_base::reply_warnings!(
    country_flags::CountryFlagReply,
    doctrines::DoctrineReply,
    focus_artwork::FocusArtworkReply,
    national_focus::FocusReply,
    technology_artwork::TechnologyArtworkReply,
    player_apm::ApmReply,
    campaign_clock::CampaignClockReply,
);

/// A product erased to its key and JSON entry points.
#[derive(Clone, Copy)]
pub struct Registered {
    pub key: &'static str,
    pub content_dependent: bool,
    pub interactive: bool,
    pub replays: bool,
    pub load: fn(&Source, &str, serde_json::Value, &LoadContext, &AtomicBool) -> Result<String, dashboard_core::QueryError>,
    #[cfg(any(test, feature = "export-types"))]
    pub types: fn() -> Vec<String>,
}

const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        replays: P::REPLAYS,
        load: load_json::<P>,
        #[cfg(any(test, feature = "export-types"))]
        types: types::<P>,
    }
}

fn load_json<P: Product>(
    source: &Source,
    lease_id: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> Result<String, dashboard_core::QueryError> {
    let request = params;
    let params = P::Params::deserialize(&request).map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    reply_json(source, lease_id, P::KEY, P::CONTENT_DEPENDENT, context, cancelled, || P::load(source, &params, context, cancelled))
}

/// The reply `load` makes for product `key`, in the transport's envelope, with the warnings every
/// reply carries.
pub(crate) fn reply_json<R: serde::Serialize + ReplyWarnings>(
    source: &Source,
    lease_id: &str,
    key: &'static str,
    content_dependent: bool,
    context: &LoadContext,
    cancelled: &AtomicBool,
    load: impl FnOnce() -> std::io::Result<R>,
) -> Result<String, dashboard_core::QueryError> {
    crate::metadata::check_cancel(cancelled).map_err(dashboard_core::QueryError::from)?;
    // A content reply holds its folders' content while it runs, so its reads and its diagnostics
    // follow one read set, whatever the source binds meanwhile.
    let _held = context
        .content
        .as_ref()
        .filter(|_| content_dependent)
        .and_then(|content| source.content(&content.options).ok());
    let mut reply = bounded_json(
        &Envelope {
            source_id: lease_id,
            revision: &source.key().revision,
            product: key,
            data: (),
        },
        cancelled,
    )?;
    // The data serializes last.
    reply.truncate(reply.len() - "null}".len());
    // The reply names what it refused and drew in the recording's style itself, with what the
    // shared loads it reused noted, never what other consumers did: the same whichever replies
    // ran before it.
    let mut own = crate::catalogue::OwnRefusals::default();
    let data = crate::catalogue::own_refusals(&mut own, load);
    let mut data = data.map_err(dashboard_core::QueryError::from)?;
    own.diagnostics().append_to(data.warnings_mut());
    if content_dependent {
        source.append_changed_warnings(data.warnings_mut(), context.content.as_ref());
    }
    // What the presentation the request was admitted under shows, never the source's now.
    source.append_presentation_warnings(data.warnings_mut(), context.content.as_ref());
    data.warnings_mut().extend(own.fallback_line());
    let data = bounded_json(&data, cancelled)?;
    if reply.len() + data.len() + 1 > MAX_REPLY_BYTES {
        return Err("Projected response exceeds the transport byte budget".into());
    }
    reply.reserve_exact(data.len() + 1);
    reply.push_str(&data);
    reply.push('}');
    Ok(reply)
}

#[cfg(any(test, feature = "export-types"))]
fn types<P: Product>() -> Vec<String> {
    P::types()
}

/// The products that show rows at an hour ([`at_hour`]), one line each: each answers requests and
/// shows a panel at the head under its key, and is registered from this line alone.
pub const AT_HOUR: &[at_hour::Entry] = &[
    at_hour::entry::<army::TemplateRoster>(),
    at_hour::entry::<air_statistics::AirStatisticsAt>(),
    at_hour::entry::<politics::state::PoliticsState>(),
    at_hour::entry::<country_people::CountryPeople>(),
    at_hour::entry::<territory::TerritoryShare>(),
    at_hour::entry::<territory::TerritoryBreakdown>(),
    at_hour::entry::<factions::FactionRosters>(),
    at_hour::entry::<scalars::GameScore>(),
    at_hour::entry::<scalars::NuclearArsenal>(),
    at_hour::entry::<scalars::WorldTension>(),
    at_hour::entry::<scalars::FactoryParts>(),
    at_hour::entry::<experience::CountryExperience>(),
    at_hour::entry::<training_queue::TrainingQueue>(),
    at_hour::entry::<diplomacy::DiplomaticRelations>(),
    at_hour::entry::<wargoals::Wargoals>(),
    at_hour::entry::<intelligence::decryption::Decryption>(),
    at_hour::entry::<intelligence::placements::OperativePlacements>(),
    at_hour::entry::<intelligence::agencies::AgencyComparison>(),
    at_hour::entry::<intelligence::roster::OperativesRoster>(),
    at_hour::entry::<intelligence::espionage::EspionageOperations>(),
    at_hour::entry::<occupation_reserve::OccupationReserve>(),
    at_hour::entry::<intel_coverage::IntelNetworkCoverage>(),
    at_hour::entry::<economy::fuel_branches::FuelBranches>(),
    at_hour::entry::<economy::resources::ResourceBalance>(),
    at_hour::entry::<economy::lend_lease_balance::LendLeaseBalance>(),
    at_hour::entry::<production::ProductionLinesAt>(),
    at_hour::entry::<production::ProductionLicences>(),
    at_hour::entry::<production::IndustrialOrganisations>(),
    at_hour::entry::<doctrine_mastery::DoctrineMastery>(),
    at_hour::entry::<doctrine_mastery::rewards::DoctrineRewards>(),
    at_hour::entry::<minefields::Minefields>(),
    at_hour::entry::<minefields::MinefieldReports>(),
    at_hour::entry::<naval_yards::NavalRepairs>(),
    at_hour::entry::<naval_yards::NavalRefits>(),
    at_hour::entry::<leaders::UnitLeaders>(),
    at_hour::entry::<leaders::career::LeaderHour>(),
    at_hour::entry::<naval_management::NavalManagement>(),
    at_hour::entry::<air_aces::AirAcesAt>(),
    at_hour::entry::<air_aces::AirAcePortraits>(),
    at_hour::entry::<regional_weather::WeatherProvinces>(),
    at_hour::entry::<focus::FocusUnavailableAt>(),
    at_hour::entry::<stability::CountryStabilityBreakdown>(),
    at_hour::entry::<stability::StabilityPartNames>(),
    at_hour::entry::<divisions::templates::DivisionTemplates>(),
    at_hour::entry::<divisions::careers::DivisionCareers>(),
    at_hour::entry::<divisions::careers::DivisionRecord>(),
    at_hour::entry::<divisions::equipment::DivisionEquipment>(),
    at_hour::entry::<research::slots::ResearchSlots>(),
    at_hour::entry::<air_missions::AirMissionsAt>(),
    at_hour::entry::<air_missions::AirMissionWingsAt>(),
    at_hour::entry::<occupation::states::OccupiedStates>(),
    at_hour::entry::<occupation::detail::OccupationDetail>(),
    // The market (P7c).
    at_hour::entry::<economy::market::MarketLendLease>(),
    at_hour::entry::<economy::fuel::CountryFuel>(),
    at_hour::entry::<stockpile::EquipmentStockpileAt>(),
    at_hour::entry::<stockpile::StockpileArtwork>(),
    at_hour::entry::<faction_state::details::FactionDetails>(),
    at_hour::entry::<wars::contributions::WarContributions>(),
    at_hour::entry::<special_projects::details::SpecialProjectDetailsAt>(),
    at_hour::entry::<special_projects::details::SpecialProjectEffectsAt>(),
    at_hour::entry::<air_superiority::AirSuperiorityAt>(),
    at_hour::entry::<equipment_losses::EquipmentLossHead>(),
    at_hour::entry::<army_losses::ArmyLosses>(),
    // The power ledger (P4c).
    at_hour::entry::<power_ledger::day::PowerDay>(),
    // A faction goal's completion rewards, asked by its tooltip (P5e).
    at_hour::entry::<faction_state::rewards::FactionGoalRewards>(),
];

/// Every product the service answers: these, then [`AT_HOUR`]. Add a product here and in the
/// frontend widget registry; nothing else needs to change. A family not yet moved to the
/// recording's own tables has no product here.
const OWN: &[Registered] = &[
    register::<campaign_clock::CampaignClock>(),
    register::<player_apm::PlayerApm>(),
    register::<national_focus::NationalFocus>(),
    register::<focus_artwork::FocusArtwork>(),
    register::<technology_artwork::TechnologyArtwork>(),
    register::<country_flags::CountryFlags>(),
    register::<doctrines::Doctrines>(),
    register::<aircraft_catalogue::AircraftCatalogue>(),
    register::<artwork::sprites::SpriteArtwork>(),
    register::<army::ArmySize>(),
    register::<identities::CountryIdentities>(),
    register::<styles::CountryFacts>(),
    register::<styles::CountryStyles>(),
    register::<groups::CountryGroups>(),
    register::<land_owners::LandOwnersProduct>(),
    register::<logs::LogPage>(),
    register::<logs::LogCounts>(),
    register::<air_ledger::MilitaryAirLedger>(),
    register::<politics::history::PoliticsStateHistory>(),
    register::<politics::civil_wars::CivilWars>(),
    register::<factions::FactionLines>(),
    register::<scalars::ThreatEvents>(),
    register::<scalars::FactoryGroups>(),
    register::<forces::AirForceSize>(),
    register::<forces::NavySize>(),
    register::<volunteers::VolunteersExpeditionaries>(),
    register::<subjects::Subjects>(),
    register::<diplomacy::DiplomaticPacts>(),
    register::<intelligence::roster::OperativesDeparted>(),
    register::<intelligence::roster::OperativeTenures>(),
    register::<intelligence::espionage::EspionageLog>(),
    register::<intelligence::espionage::EspionageOperation>(),
    register::<intelligence::espionage::EspionageEnded>(),
    register::<intelligence::espionage::EspionageTypes>(),
    register::<intelligence::intel_details::IntelDetails>(),
    register::<raids::RaidLog>(),
    register::<economy::resources::ResourceSizes>(),
    register::<economy::lend_lease_partners::LendLeasePartners>(),
    register::<production::ProductionLines>(),
    register::<production::ProductionLineDetail>(),
    register::<production::ProductionLineBreakdown>(),
    register::<production::EquipmentDesigns>(),
    register::<place_names::PlaceNames>(),
    register::<minefields::MinefieldHistory>(),
    register::<naval_supremacy::NavalSupremacy>(),
    register::<leaders::LeaderPortraits>(),
    register::<leaders::career::LeaderCareer>(),
    register::<leaders::career::LeaderChoices>(),
    register::<regional_weather::RegionalWeather>(),
    register::<focus::FocusHourProduct>(),
    register::<focus::FocusOrderPageProduct>(),
    register::<focus::FocusCompletionsProduct>(),
    register::<ideas::IdeaHistory>(),
    register::<party_history::PartyHistory>(),
    // Casualties and peace (P5c).
    register::<casualties::CasualtyPairs>(),
    register::<casualties::CasualtyBreakdown>(),
    register::<casualties::CasualtyLabels>(),
    register::<peace::PeaceDeals>(),
    register::<convoys::ConvoyHealth>(),
    register::<trade_flows::TradeFlows>(),
    register::<production_spending::ProductionSpending>(),
    register::<occupation_reserve::ReserveHistory>(),
    register::<construction::log::ConstructionLogManifest>(),
    register::<construction::log::ConstructionLog>(),
    register::<construction::spending::ConstructionSpending>(),
    register::<division_log::DivisionLog>(),
    register::<division_log::DivisionLogManifest>(),
    register::<land_battles::LandBattlesManifest>(),
    register::<land_battles::LandBattles>(),
    register::<land_battles::LandBattlesRanking>(),
    register::<land_battles::LandBattleCourses>(),
    register::<land_terrain::LandBattleTerrains>(),
    register::<naval::battles::NavalBattlesManifest>(),
    register::<naval::battles::NavalBattles>(),
    register::<naval::battles::NavalBattlesRanking>(),
    register::<naval::battles::NavalBattleDetails>(),
    register::<naval::sunk::ShipsSunkManifest>(),
    register::<naval::sunk::ShipsSunk>(),
    register::<naval::losses::NavalLosses>(),
    register::<research::Research>(),
    register::<research::breakdown::ResearchBreakdown>(),
    register::<feed::EventFeed>(),
    register::<feed::EventChoices>(),
    register::<feed::Decisions>(),
    register::<feed::EventFeedSpan>(),
    // The market (P7c).
    register::<economy::market::history::LendLeaseHistory>(),
    register::<economy::market::ledger::MarketLedger>(),
    register::<economy::fuel::awards::CountryFuelAwards>(),
    register::<economy::fuel::awards::CountryFuelAwardHistory>(),
    register::<economy::fuel::demand::CountryFuelDemand>(),
    register::<economy::fuel::demand::CountryFuelDemandHistory>(),
    register::<stockpile::EquipmentStockpileCatalogue>(),
    register::<stockpile::EquipmentStockpileChart>(),
    register::<faction_state::history::FactionState>(),
    register::<wars::Wars>(),
    register::<wars::WarLosses>(),
    register::<special_projects::SpecialProjects>(),
    register::<air_superiority::AirSuperiorityChoices>(),
    register::<combat_log::CombatLog>(),
    register::<equipment_losses::EquipmentLosses>(),
    // The feed, research, air and weather's second pass (Q7).
    register::<air_statistics::AirStatisticsRegions>(),
    // The scoreboard (P13).
    register::<scoreboard::ScoreboardColumns>(),
    register::<scoreboard::Scoreboard>(),
    // Equipment produced (P8d).
    register::<equipment_produced::EquipmentProduced>(),
    register::<equipment_produced::EquipmentProducedTypes>(),
    register::<equipment_produced::EquipmentProducedCountry>(),
    register::<equipment_produced::EquipmentProducedAt>(),
    // The power ledger (P4c).
    register::<power_ledger::PowerSpending>(),
    // Stability's input reads, for the chart's "as of" marks (Q5).
    register::<stability::reads::StabilityReads>(),
    register::<air_superiority::timeline::AirSuperiorityTimeline>(),
    register::<air_superiority::timeline::AirSuperiorityRanking>(),
    // The espionage types' tokens, apart from their names (L1).
    register::<intelligence::espionage::EspionageTypeTokens>(),
];

const ALL: [Registered; OWN.len() + AT_HOUR.len()] = at_hour::joined(OWN, AT_HOUR);
pub static PRODUCTS: &[Registered] = &ALL;

/// The products answered from the recording's tables through the query layer: these, then
/// [`AT_HOUR`].
const MOVED_OWN: &[&str] = &[
    army::ArmySize::KEY,
    identities::CountryIdentities::KEY,
    styles::CountryFacts::KEY,
    styles::CountryStyles::KEY,
    groups::CountryGroups::KEY,
    land_owners::LandOwnersProduct::KEY,
    logs::LogPage::KEY,
    logs::LogCounts::KEY,
    air_ledger::MilitaryAirLedger::KEY,
    politics::history::PoliticsStateHistory::KEY,
    politics::civil_wars::CivilWars::KEY,
    factions::FactionLines::KEY,
    scalars::ThreatEvents::KEY,
    scalars::FactoryGroups::KEY,
    forces::AirForceSize::KEY,
    forces::NavySize::KEY,
    volunteers::VolunteersExpeditionaries::KEY,
    subjects::Subjects::KEY,
    diplomacy::DiplomaticPacts::KEY,
    intelligence::roster::OperativesDeparted::KEY,
    intelligence::roster::OperativeTenures::KEY,
    intelligence::espionage::EspionageLog::KEY,
    intelligence::espionage::EspionageOperation::KEY,
    intelligence::espionage::EspionageEnded::KEY,
    intelligence::espionage::EspionageTypes::KEY,
    intelligence::intel_details::IntelDetails::KEY,
    raids::RaidLog::KEY,
    economy::resources::ResourceSizes::KEY,
    economy::lend_lease_partners::LendLeasePartners::KEY,
    production::ProductionLines::KEY,
    production::ProductionLineDetail::KEY,
    production::ProductionLineBreakdown::KEY,
    production::EquipmentDesigns::KEY,
    place_names::PlaceNames::KEY,
    minefields::MinefieldHistory::KEY,
    naval_supremacy::NavalSupremacy::KEY,
    leaders::LeaderPortraits::KEY,
    leaders::career::LeaderCareer::KEY,
    leaders::career::LeaderChoices::KEY,
    regional_weather::RegionalWeather::KEY,
    national_focus::NationalFocus::KEY,
    focus::FocusHourProduct::KEY,
    focus::FocusOrderPageProduct::KEY,
    focus::FocusCompletionsProduct::KEY,
    ideas::IdeaHistory::KEY,
    party_history::PartyHistory::KEY,
    casualties::CasualtyPairs::KEY,
    casualties::CasualtyBreakdown::KEY,
    casualties::CasualtyLabels::KEY,
    peace::PeaceDeals::KEY,
    convoys::ConvoyHealth::KEY,
    trade_flows::TradeFlows::KEY,
    production_spending::ProductionSpending::KEY,
    occupation_reserve::ReserveHistory::KEY,
    construction::log::ConstructionLogManifest::KEY,
    construction::log::ConstructionLog::KEY,
    construction::spending::ConstructionSpending::KEY,
    division_log::DivisionLog::KEY,
    division_log::DivisionLogManifest::KEY,
    land_battles::LandBattlesManifest::KEY,
    land_battles::LandBattles::KEY,
    land_battles::LandBattlesRanking::KEY,
    land_battles::LandBattleCourses::KEY,
    land_terrain::LandBattleTerrains::KEY,
    naval::battles::NavalBattlesManifest::KEY,
    naval::battles::NavalBattles::KEY,
    naval::battles::NavalBattlesRanking::KEY,
    naval::battles::NavalBattleDetails::KEY,
    naval::sunk::ShipsSunkManifest::KEY,
    naval::sunk::ShipsSunk::KEY,
    naval::losses::NavalLosses::KEY,
    research::Research::KEY,
    research::breakdown::ResearchBreakdown::KEY,
    feed::EventFeed::KEY,
    feed::EventChoices::KEY,
    feed::Decisions::KEY,
    feed::EventFeedSpan::KEY,
    economy::market::history::LendLeaseHistory::KEY,
    economy::market::ledger::MarketLedger::KEY,
    economy::fuel::awards::CountryFuelAwards::KEY,
    economy::fuel::awards::CountryFuelAwardHistory::KEY,
    economy::fuel::demand::CountryFuelDemand::KEY,
    economy::fuel::demand::CountryFuelDemandHistory::KEY,
    stockpile::EquipmentStockpileCatalogue::KEY,
    stockpile::EquipmentStockpileChart::KEY,
    faction_state::history::FactionState::KEY,
    wars::Wars::KEY,
    wars::WarLosses::KEY,
    special_projects::SpecialProjects::KEY,
    air_superiority::AirSuperiorityChoices::KEY,
    combat_log::CombatLog::KEY,
    equipment_losses::EquipmentLosses::KEY,
    air_statistics::AirStatisticsRegions::KEY,
    scoreboard::ScoreboardColumns::KEY,
    scoreboard::Scoreboard::KEY,
    equipment_produced::EquipmentProduced::KEY,
    equipment_produced::EquipmentProducedTypes::KEY,
    equipment_produced::EquipmentProducedCountry::KEY,
    equipment_produced::EquipmentProducedAt::KEY,
    // The power ledger (P4c).
    power_ledger::PowerSpending::KEY,
    stability::reads::StabilityReads::KEY,
    air_superiority::timeline::AirSuperiorityTimeline::KEY,
    air_superiority::timeline::AirSuperiorityRanking::KEY,
    intelligence::espionage::EspionageTypeTokens::KEY,
];

const MOVED_ALL: [&str; MOVED_OWN.len() + AT_HOUR.len()] = at_hour::keys(MOVED_OWN, AT_HOUR);
pub const MOVED: &[&str] = &MOVED_ALL;

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.iter().find(|product| product.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_product_type_names_do_not_hide_different_shapes() {
        let mut declarations = std::collections::BTreeMap::new();
        for product in PRODUCTS {
            for declaration in (product.types)() {
                let name = declaration.strip_prefix("type ").unwrap().split_whitespace().next().unwrap().to_owned();
                if let Some(previous) = declarations.insert(name.clone(), declaration.clone()) {
                    assert_eq!(previous, declaration, "different exported types named {name}");
                }
            }
        }
    }

    #[test]
    fn tag_validation_preserves_long_recorded_lookup_keys() {
        for tag in ["x".repeat(129), "å›½".repeat(129)] {
            assert_eq!(validate_tag(&tag), Ok(()));
            assert_eq!(
                CountryParams {
                    branch_id: "1".into(),
                    tag,
                }
                .validate(),
                Ok(())
            );
        }
        for tag in ["", "\0", "tag\0suffix"] {
            assert_eq!(validate_tag(tag), Err("Invalid country tag"));
        }
    }

    #[test]
    fn browser_interactive_products_match_service_admission() {
        let contract = crate::typescript_contract();
        let declaration = contract
            .lines()
            .find(|line| line.starts_with("export const INTERACTIVE_PRODUCTS"))
            .expect("browser admission needs the service's product lanes");
        let replays = contract
            .lines()
            .find(|line| line.starts_with("export const REPLAY_PRODUCTS"))
            .expect("browser admission needs the service's replaying pages");
        for product in PRODUCTS {
            assert_eq!(
                declaration.contains(&format!("{:?}", product.key)),
                product.interactive,
                "{} admission differs between browser and service",
                product.key
            );
            assert_eq!(replays.contains(&format!("{:?}", product.key)), product.replays, "{} replays differently in browser and service", product.key);
        }
    }

    #[test]
    fn product_keys_are_unique() {
        let mut keys: Vec<_> = PRODUCTS.iter().map(|product| product.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRODUCTS.len());
        if let Some(path) = std::env::var_os("BATTLE_PLAN_TYPESCRIPT_OUTPUT").or_else(|| std::env::var_os("HOI4_TYPESCRIPT_OUTPUT")) {
            std::fs::write(path, crate::typescript_contract()).unwrap();
        }
    }

    #[test]
    fn every_moved_product_is_registered() {
        for key in MOVED {
            assert!(find(key).is_some(), "{key} is not registered");
        }
    }
}
