
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
pub use dashboard_intelligence::products::intelligence;
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
pub use dashboard_research::products::research;
pub mod stockpile;
pub mod styles;
pub mod subjects;
pub mod technology_artwork;
pub mod technology_effects;
pub mod entity_stats;
pub mod game_text;
pub mod territory;
pub mod factions;
pub mod experience;
pub mod forces;
pub mod training_queue;
pub mod volunteers;
pub mod divisions;
pub mod production_spending;
#[cfg(test)]
mod war_fixtures;
pub mod wargoals;
pub mod stability;
pub mod convoys;
pub mod trade_flows;
pub mod division_log;
pub mod land_battles;
pub mod combat_log;
pub mod land_terrain;
pub mod naval;
pub mod feed;
pub mod air_missions;
pub mod faction_state;
pub mod wars;
pub mod special_projects;
pub mod air_superiority;
pub mod army_losses;
pub mod equipment_losses;
pub mod scoreboard;
pub mod equipment_produced;
pub mod power_ledger;
pub mod windows;

pub use dashboard_base::products::*;
#[cfg(any(test, feature = "export-types"))]
use dashboard_base::products::register_for_export as register_local;
#[cfg(not(any(test, feature = "export-types")))]
use dashboard_base::products::register as register_local;




dashboard_base::reply_warnings!(
    country_flags::CountryFlagReply,
    doctrines::DoctrineReply,
    focus_artwork::FocusArtworkReply,
    national_focus::FocusReply,
    technology_artwork::TechnologyArtworkReply,
    player_apm::ApmReply,
    campaign_clock::CampaignClockReply,
);

pub const AT_HOUR: &[at_hour::Entry] = &[
    at_hour::local_entry::<army::TemplateRoster>(),
    at_hour::local_entry::<air_statistics::AirStatisticsAt>(),
    at_hour::local_entry::<politics::state::PoliticsState>(),
    at_hour::local_entry::<country_people::CountryPeople>(),
    at_hour::local_entry::<territory::TerritoryShare>(),
    at_hour::local_entry::<territory::TerritoryBreakdown>(),
    at_hour::local_entry::<factions::FactionRosters>(),
    at_hour::local_entry::<scalars::GameScore>(),
    at_hour::local_entry::<scalars::NuclearArsenal>(),
    at_hour::local_entry::<scalars::WorldTension>(),
    at_hour::local_entry::<scalars::FactoryParts>(),
    at_hour::local_entry::<experience::CountryExperience>(),
    at_hour::local_entry::<training_queue::TrainingQueue>(),
    at_hour::local_entry::<diplomacy::DiplomaticRelations>(),
    at_hour::local_entry::<wargoals::Wargoals>(),
    dashboard_intelligence::registrations::INTELLIGENCE_DECRYPTION_DECRYPTION,
    dashboard_intelligence::registrations::INTELLIGENCE_PLACEMENTS_OPERATIVEPLACEMENTS,
    dashboard_intelligence::registrations::INTELLIGENCE_AGENCIES_AGENCYCOMPARISON,
    dashboard_intelligence::registrations::INTELLIGENCE_ROSTER_OPERATIVESROSTER,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEOPERATIONS,
    at_hour::local_entry::<occupation_reserve::OccupationReserve>(),
    at_hour::local_entry::<intel_coverage::IntelNetworkCoverage>(),
    at_hour::local_entry::<economy::fuel_branches::FuelBranches>(),
    at_hour::local_entry::<economy::resources::ResourceBalance>(),
    at_hour::local_entry::<economy::lend_lease_balance::LendLeaseBalance>(),
    at_hour::local_entry::<production::ProductionLinesAt>(),
    at_hour::local_entry::<production::ProductionLicences>(),
    at_hour::local_entry::<production::IndustrialOrganisations>(),
    at_hour::local_entry::<doctrine_mastery::DoctrineMastery>(),
    at_hour::local_entry::<doctrine_mastery::rewards::DoctrineRewards>(),
    at_hour::local_entry::<minefields::Minefields>(),
    at_hour::local_entry::<minefields::MinefieldReports>(),
    at_hour::local_entry::<naval_yards::NavalRepairs>(),
    at_hour::local_entry::<naval_yards::NavalRefits>(),
    at_hour::local_entry::<leaders::UnitLeaders>(),
    at_hour::local_entry::<leaders::career::LeaderHour>(),
    at_hour::local_entry::<naval_management::NavalManagement>(),
    at_hour::local_entry::<air_aces::AirAcesAt>(),
    at_hour::local_entry::<air_aces::AirAcePortraits>(),
    at_hour::local_entry::<regional_weather::WeatherProvinces>(),
    at_hour::local_entry::<focus::FocusUnavailableAt>(),
    at_hour::local_entry::<stability::CountryStabilityBreakdown>(),
    at_hour::local_entry::<stability::StabilityPartNames>(),
    at_hour::local_entry::<divisions::templates::DivisionTemplates>(),
    at_hour::local_entry::<divisions::careers::DivisionCareers>(),
    at_hour::local_entry::<divisions::careers::DivisionRecord>(),
    at_hour::local_entry::<divisions::equipment::DivisionEquipment>(),
    dashboard_research::registrations::RESEARCH_SLOTS_RESEARCHSLOTS,
    at_hour::local_entry::<air_missions::AirMissionsAt>(),
    at_hour::local_entry::<air_missions::AirMissionWingsAt>(),
    at_hour::local_entry::<occupation::states::OccupiedStates>(),
    at_hour::local_entry::<occupation::detail::OccupationDetail>(),
    at_hour::local_entry::<economy::market::MarketLendLease>(),
    at_hour::local_entry::<economy::fuel::CountryFuel>(),
    at_hour::local_entry::<stockpile::EquipmentStockpileAt>(),
    at_hour::local_entry::<stockpile::StockpileArtwork>(),
    at_hour::local_entry::<faction_state::details::FactionDetails>(),
    at_hour::local_entry::<wars::contributions::WarContributions>(),
    at_hour::local_entry::<special_projects::details::SpecialProjectDetailsAt>(),
    at_hour::local_entry::<special_projects::details::SpecialProjectEffectsAt>(),
    at_hour::local_entry::<special_projects::details::SpecialProjectIterationAt>(),
    at_hour::local_entry::<technology_effects::TechnologyEffectsAt>(),
    at_hour::local_entry::<entity_stats::EntityStatsAt>(),
    at_hour::local_entry::<air_superiority::AirSuperiorityAt>(),
    at_hour::local_entry::<equipment_losses::EquipmentLossHead>(),
    at_hour::local_entry::<army_losses::ArmyLosses>(),
    at_hour::local_entry::<power_ledger::day::PowerDay>(),
    at_hour::local_entry::<faction_state::rewards::FactionGoalRewards>(),
];

const OWN: &[Registered] = &[
    register_local::<campaign_clock::CampaignClock>(),
    register_local::<player_apm::PlayerApm>(),
    register_local::<national_focus::NationalFocus>(),
    register_local::<focus_artwork::FocusArtwork>(),
    register_local::<technology_artwork::TechnologyArtwork>(),
    register_local::<country_flags::CountryFlags>(),
    register_local::<doctrines::Doctrines>(),
    register_local::<aircraft_catalogue::AircraftCatalogue>(),
    register_local::<artwork::sprites::SpriteArtwork>(),
    register_local::<army::ArmySize>(),
    register_local::<identities::CountryIdentities>(),
    register_local::<styles::CountryFacts>(),
    register_local::<styles::CountryStyles>(),
    register_local::<groups::CountryGroups>(),
    register_local::<land_owners::LandOwnersProduct>(),
    register_local::<logs::LogPage>(),
    register_local::<logs::LogCounts>(),
    register_local::<air_ledger::MilitaryAirLedger>(),
    register_local::<politics::history::PoliticsStateHistory>(),
    register_local::<politics::civil_wars::CivilWars>(),
    register_local::<factions::FactionLines>(),
    register_local::<scalars::ThreatEvents>(),
    register_local::<scalars::FactoryGroups>(),
    register_local::<forces::AirForceSize>(),
    register_local::<forces::NavySize>(),
    register_local::<volunteers::VolunteersExpeditionaries>(),
    register_local::<subjects::Subjects>(),
    register_local::<diplomacy::DiplomaticPacts>(),
    dashboard_intelligence::registrations::INTELLIGENCE_ROSTER_OPERATIVESDEPARTED,
    dashboard_intelligence::registrations::INTELLIGENCE_ROSTER_OPERATIVETENURES,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGELOG,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEOPERATION,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEENDED,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPES,
    dashboard_intelligence::registrations::INTELLIGENCE_INTEL_DETAILS_INTELDETAILS,
    register_local::<raids::RaidLog>(),
    register_local::<economy::resources::ResourceSizes>(),
    register_local::<economy::lend_lease_partners::LendLeasePartners>(),
    register_local::<production::ProductionLines>(),
    register_local::<production::ProductionLineDetail>(),
    register_local::<production::ProductionLineBreakdown>(),
    register_local::<production::EquipmentDesigns>(),
    register_local::<place_names::PlaceNames>(),
    register_local::<minefields::MinefieldHistory>(),
    register_local::<naval_supremacy::NavalSupremacy>(),
    register_local::<leaders::LeaderPortraits>(),
    register_local::<leaders::career::LeaderCareer>(),
    register_local::<leaders::career::LeaderChoices>(),
    register_local::<regional_weather::RegionalWeather>(),
    register_local::<focus::FocusHourProduct>(),
    register_local::<focus::FocusOrderPageProduct>(),
    register_local::<focus::FocusCompletionsProduct>(),
    register_local::<ideas::IdeaHistory>(),
    register_local::<party_history::PartyHistory>(),
    register_local::<casualties::CasualtyPairs>(),
    register_local::<casualties::CasualtyBreakdown>(),
    register_local::<casualties::CasualtyLabels>(),
    register_local::<peace::PeaceDeals>(),
    register_local::<convoys::ConvoyHealth>(),
    register_local::<trade_flows::TradeFlows>(),
    register_local::<production_spending::ProductionSpending>(),
    register_local::<occupation_reserve::ReserveHistory>(),
    register_local::<construction::log::ConstructionLogManifest>(),
    register_local::<construction::log::ConstructionLog>(),
    register_local::<construction::spending::ConstructionSpending>(),
    register_local::<division_log::DivisionLog>(),
    register_local::<division_log::DivisionLogManifest>(),
    register_local::<land_battles::LandBattlesManifest>(),
    register_local::<land_battles::LandBattles>(),
    register_local::<land_battles::LandBattlesRanking>(),
    register_local::<land_battles::LandBattleCourses>(),
    register_local::<land_terrain::LandBattleTerrains>(),
    register_local::<naval::battles::NavalBattlesManifest>(),
    register_local::<naval::battles::NavalBattles>(),
    register_local::<naval::battles::NavalBattlesRanking>(),
    register_local::<naval::battles::NavalBattleDetails>(),
    register_local::<naval::sunk::ShipsSunkManifest>(),
    register_local::<naval::sunk::ShipsSunk>(),
    register_local::<naval::losses::NavalLosses>(),
    dashboard_research::registrations::RESEARCH_RESEARCH,
    dashboard_research::registrations::RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN,
    register_local::<feed::EventFeed>(),
    register_local::<feed::EventChoices>(),
    register_local::<feed::Decisions>(),
    register_local::<feed::EventFeedSpan>(),
    register_local::<economy::market::history::LendLeaseHistory>(),
    register_local::<economy::market::ledger::MarketLedger>(),
    register_local::<economy::fuel::awards::CountryFuelAwards>(),
    register_local::<economy::fuel::awards::CountryFuelAwardHistory>(),
    register_local::<economy::fuel::demand::CountryFuelDemand>(),
    register_local::<economy::fuel::demand::CountryFuelDemandHistory>(),
    register_local::<stockpile::EquipmentStockpileCatalogue>(),
    register_local::<stockpile::EquipmentStockpileChart>(),
    register_local::<faction_state::history::FactionState>(),
    register_local::<wars::Wars>(),
    register_local::<wars::WarLosses>(),
    register_local::<special_projects::SpecialProjects>(),
    register_local::<air_superiority::AirSuperiorityChoices>(),
    register_local::<combat_log::CombatLog>(),
    register_local::<equipment_losses::EquipmentLosses>(),
    register_local::<air_statistics::AirStatisticsRegions>(),
    register_local::<scoreboard::ScoreboardColumns>(),
    register_local::<scoreboard::Scoreboard>(),
    register_local::<equipment_produced::EquipmentProduced>(),
    register_local::<equipment_produced::EquipmentProducedTypes>(),
    register_local::<equipment_produced::EquipmentProducedCountry>(),
    register_local::<equipment_produced::EquipmentProducedAt>(),
    register_local::<power_ledger::PowerSpending>(),
    register_local::<stability::reads::StabilityReads>(),
    register_local::<air_superiority::timeline::AirSuperiorityTimeline>(),
    register_local::<air_superiority::timeline::AirSuperiorityRanking>(),
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPETOKENS,
];

const ALL: [Registered; OWN.len() + AT_HOUR.len()] = at_hour::joined(OWN, AT_HOUR);
pub static PRODUCTS: &[Registered] = &ALL;

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
            for declaration in product.types.expect("every exported product registers its types")() {
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
                    country: tag.into(),
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
        if let Some(path) = std::env::var_os("BATTLEPLAN_TYPESCRIPT_OUTPUT").or_else(|| std::env::var_os("HOI4_TYPESCRIPT_OUTPUT")) {
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
