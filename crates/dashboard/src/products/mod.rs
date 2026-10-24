//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply that
//! covers a whole branch, so playback in the frontend is a local lookup. The
//! service, transport, host and frontend never name a product; they route by
//! its key.

pub mod aircraft_catalogue;
pub mod country_flags;
pub mod doctrines;
pub mod event_feed;
pub mod focus_artwork;
pub use identity::civil_war;
pub mod land_owners;
pub mod national_focus;
pub mod scoreboard;
pub mod technology_artwork;

// The product contract and the shared products, at their old paths.
pub use dashboard_air_aces::products::air_aces;
pub use dashboard_air_operations::products::{air_missions, air_statistics};
pub use dashboard_air_superiority::products::air_superiority;
pub use dashboard_army::products::{
    division_careers, division_equipment, division_templates, training_queue,
};
pub use dashboard_base::products::*;
pub use dashboard_construction::products::construction_log;
pub use dashboard_construction::products::construction_spending;
pub use dashboard_country_fuel::products::country_fuel;
pub use dashboard_country_nukes::products::country_nukes;
pub use dashboard_diplomacy::products::diplomacy_state;
pub use dashboard_doctrine_mastery::products::doctrine_mastery;
pub use dashboard_economy::products::{
    convoy_health, fuel_branches, lend_lease_balance, market, production_catalogue,
};
pub use dashboard_equipment_losses::products::equipment_losses;
pub use dashboard_equipment_stockpile::products::equipment_stockpile;
pub use dashboard_factions::products::faction_state;
pub use dashboard_forces::products::air_force;
pub use dashboard_forces::products::army_size;
pub use dashboard_forces::products::naval_management;
pub use dashboard_forces::products::navy;
pub use dashboard_forces::products::parts_race_prepared;
pub use dashboard_forces::products::supply_health;
pub use dashboard_forces::products::volunteers_expeditionaries;
pub use dashboard_game_score::products::game_score;
pub use dashboard_game_window::products::game_window;
pub use dashboard_intelligence::products::agency_comparison;
pub use dashboard_intelligence::products::decryption;
pub use dashboard_intelligence::products::espionage_log;
pub use dashboard_intelligence::products::intel_coverage;
pub use dashboard_intelligence::products::intel_details;
pub use dashboard_intelligence::products::intelligence_content;
pub use dashboard_intelligence::products::intelligence_prepared;
pub use dashboard_intelligence::products::operation_description;
pub use dashboard_intelligence::products::operative_content;
pub use dashboard_intelligence::products::operative_placements;
pub use dashboard_intelligence::products::operatives_roster;
pub use dashboard_land_battles::products::army_losses;
pub use dashboard_land_battles::products::combat_log;
pub use dashboard_land_battles::products::division_log;
pub use dashboard_land_battles::products::land_battles;
pub use dashboard_military_air_ledger::products::military_air_ledger;
pub use dashboard_naval::products::naval_battles;
pub use dashboard_naval::products::naval_losses;
pub use dashboard_naval::products::ships_sunk;
pub use dashboard_naval::products::sinkings;
pub use dashboard_naval_regions::products::{minefields, naval_supremacy};
pub use dashboard_naval_yards::products::{naval_refits, naval_repairs};
pub use dashboard_occupation::products::occupation_reserve;
pub use dashboard_occupation::products::{occupation_trend, occupied_states};
pub use dashboard_party_history::products::party_history;
pub use dashboard_peace_deals::products::peace_deals;
pub use dashboard_politics::products::politics_state;
pub use dashboard_politics_ledger::products::politics_ledger;
pub use dashboard_raids::products::raids;
pub use dashboard_resource_balance::products::market_lendlease;
pub use dashboard_resource_balance::products::resource_balance;
pub use dashboard_resource_balance::products::trade_flows;
pub use dashboard_special_projects::products::special_projects;
pub use dashboard_stability::products::{country_stability, stability_parts};
pub use dashboard_subjects::products::{autonomy, subjects};
pub use dashboard_unit_leaders::products::unit_leaders;
pub use dashboard_wars::products::{war_casualties, wargoals, wars};
pub use dashboard_weather::products::regional_weather;
pub use dashboard_world_tension::products::world_tension;

use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::{
    Source, reply_cache,
    transport::{MAX_REPLY_BYTES, bounded_json},
};

dashboard_base::reply_warnings!(
    country_flags::CountryFlagReply,
    doctrines::DoctrineReply,
    event_feed::EventFeedReply,
    focus_artwork::FocusArtworkReply,
    crate::LandOwners,
    national_focus::FocusReply,
    technology_artwork::TechnologyArtworkReply,
    scoreboard::ScoreboardReply,
);

/// A product erased to its key and JSON entry points.
pub struct Registered {
    pub key: &'static str,
    pub content_dependent: bool,
    pub interactive: bool,
    pub load:
        fn(&Source, &str, serde_json::Value, &LoadContext, &AtomicBool) -> Result<String, String>,
    #[cfg(any(test, feature = "export-types"))]
    pub types: fn() -> Vec<String>,
    /// Answered only by prepared queries: its product route refuses, while its
    /// types, content dependence and lane stay registered.
    pub prepared_only: bool,
}

const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        load: load_json::<P>,
        #[cfg(any(test, feature = "export-types"))]
        types: types::<P>,
        prepared_only: false,
    }
}

/// A product whose replies come only from prepared queries, with no loader.
const fn register_prepared<P: Product>() -> Registered {
    Registered {
        load: refuse::<P>,
        prepared_only: true,
        ..register::<P>()
    }
}

fn refuse<P: Product>(
    _source: &Source,
    _lease_id: &str,
    _params: serde_json::Value,
    _context: &LoadContext,
    _cancelled: &AtomicBool,
) -> Result<String, String> {
    Err(prepared_only(P::KEY))
}

fn load_json<P: Product>(
    source: &Source,
    lease_id: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let request = params;
    let params = P::Params::deserialize(&request)
        .map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    crate::metadata::check_cancel(cancelled).map_err(|error| error.to_string())?;
    let prepared = production_lines::PREPARED_PRODUCTS
        .contains(&P::KEY)
        .then(|| crate::prepared::files(source, context.content.as_ref()))
        .transpose()
        .map_err(|error| error.to_string())?;
    let _scope = prepared
        .as_ref()
        .map(|(store, key)| store.scope(key))
        .transpose()
        .map_err(|error| error.to_string())?;
    // Whole-recording replies persist between sessions; hover answers are quicker to compute.
    let entry = (!P::INTERACTIVE && !production_lines::PREPARED_PRODUCTS.contains(&P::KEY))
        .then(|| reply_cache::Entry::new(source, P::KEY, P::CONTENT_DEPENDENT, &request, context))
        .flatten();
    let mut reply = bounded_json(
        &Envelope {
            source_id: lease_id,
            revision: &source.key().revision,
            product: P::KEY,
            data: (),
        },
        cancelled,
    )?;
    // The data serializes last.
    reply.truncate(reply.len() - "null}".len());
    // Content read outside the recorded coverage is reported on every reply, so
    // no reply is served from or kept on disk while any is.
    let stored = entry
        .as_ref()
        .filter(|_| !source.reads_uncovered())
        .and_then(reply_cache::Entry::get);
    let data = match stored {
        Some(data) => data,
        None => {
            // What this reply drew in the recording's style because the selected visual mods
            // could not supply it, named and counted as the map names its own: the reply's
            // own fallbacks, with those of shared loads it reused.
            let (data, own) =
                crate::catalogue::capture(|| P::load(source, &params, context, cancelled));
            let mut data = data.map_err(|error| error.to_string())?;
            source.append_content_warnings(data.warnings_mut());
            // What the presentation the request was admitted under shows, never the source's now.
            source.append_presentation_warnings(data.warnings_mut(), context.content.as_ref());
            data.warnings_mut().extend(own.fallback_line());
            let data = bounded_json(&data, cancelled)?;
            // Kept under the identity it was drawn with (the recording's own for recorded-only
            // work, whatever is selected), and only while its admission still holds the source's
            // current binding: a reply of a binding the source has left is not kept.
            if let Some(entry) = &entry
                && !source.reads_uncovered()
                && context
                    .content
                    .as_ref()
                    .is_none_or(|content| source.still_admitted(content))
            {
                entry.put(&data);
            }
            data
        }
    };
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

/// Every product the service answers. Add a product here and in the frontend
/// widget registry; nothing else needs to change.
pub static PRODUCTS: &[Registered] = &[
    register_prepared::<diplomacy_state::DiplomacyState>(),
    register::<national_focus::NationalFocus>(),
    register::<focus_artwork::FocusArtwork>(),
    register::<crate::prepared_focus::FocusArt>(),
    register_prepared::<research::Research>(),
    register::<technology_artwork::TechnologyArtwork>(),
    register::<country_flags::CountryFlags>(),
    register::<doctrines::Doctrines>(),
    register::<game_window::GameWindow>(),
    register_prepared::<army_size::ArmySize>(),
    register_prepared::<volunteers_expeditionaries::VolunteersExpeditionaries>(),
    register_prepared::<supply_health::SupplyHealth>(),
    register_prepared::<division_log::DivisionLog>(),
    register_prepared::<division_log::DivisionLogManifest>(),
    register_prepared::<division_log::DivisionLogTotals>(),
    register_prepared::<unit_leaders::UnitLeaders>(),
    register::<unit_leaders::portraits::UnitLeaderPortraits>(),
    register_prepared::<land_owners::LandOwnersProduct>(),
    register_prepared::<land_battles::LandBattles>(),
    register_prepared::<land_battles::LandBattlesManifest>(),
    register_prepared::<land_battles::LandBattlesRanking>(),
    register_prepared::<land_battles::LandBattleCourses>(),
    register::<land_battles::LandBattleTerrains>(),
    register::<combat_log::CombatLog>(),
    register::<army_losses::ArmyLosses>(),
    register::<division_careers::DivisionCareers>(),
    register::<division_careers::DivisionRecord>(),
    register::<division_templates::DivisionTemplates>(),
    register::<division_equipment::DivisionEquipment>(),
    register_prepared::<division_equipment::history::DivisionEquipmentHistory>(),
    register::<training_queue::TrainingQueue>(),
    register::<place_names::PlaceNames>(),
    register::<country_styles::CountryStyles>(),
    register::<country_colours::CountryColours>(),
    register::<faction_details::FactionDetails>(),
    register_prepared::<country_groups::CountryGroups>(),
    // All faction history views share one prepared publication and gate.
    register_prepared::<faction_state::FactionState>(),
    register_prepared::<wars::Wars>(),
    register::<wargoals::Wargoals>(),
    register::<country_stability::CountryStability>(),
    register::<stability_parts::StabilityParts>(),
    register::<country_measure::CountryMeasure>(),
    register::<country_people::CountryPeople>(),
    register_prepared::<country_fuel::CountryFuel>(),
    register::<country_fuel::CountryFuelAwards>(),
    register::<country_fuel::CountryFuelAwardHistory>(),
    register_prepared::<country_fuel::CountryFuelConsumption>(),
    register::<country_fuel::CountryFuelDemand>(),
    register::<country_fuel::CountryFuelDemandHistory>(),
    register::<country_fuel::CountryFuelDemandStack>(),
    register_prepared::<war_casualties::WarCasualties>(),
    register_prepared::<war_casualties::pairs::CasualtyPairs>(),
    register_prepared::<war_casualties::breakdown::CasualtyBreakdown>(),
    register::<country_factories::CountryFactories>(),
    register_prepared::<territory::Territory>(),
    register_prepared::<territory_breakdown::TerritoryBreakdown>(),
    register_prepared::<faction_totals::FactionTotals>(),
    register_prepared::<air_aces::AirAces>(),
    register_prepared::<air_aces::AirAcePortraits>(),
    register::<air_force::AirForceSize>(),
    register::<navy::NavySize>(),
    register::<naval_management::NavalManagement>(),
    register::<game_score::GameScore>(),
    register_prepared::<scoreboard::Scoreboard>(),
    register::<country_nukes::CountryNukes>(),
    register_prepared::<air_superiority::AirSuperiority>(),
    register_prepared::<raids::Raids>(),
    register_prepared::<occupied_states::OccupiedStates>(),
    register_prepared::<occupied_states::OccupiedStatesManifest>(),
    register_prepared::<occupation_trend::OccupationTrend>(),
    register::<country_experience::CountryExperience>(),
    register::<idea_history::IdeaHistory>(),
    register_prepared::<party_history::PartyHistory>(),
    register::<politics_state::PoliticsState>(),
    register_prepared::<politics_state::history::PoliticsStateHistory>(),
    register_prepared::<politics_state::civil_wars::CivilWars>(),
    register_prepared::<politics_ledger::PoliticsLedger>(),
    register_prepared::<fuel_branches::FuelBranches>(),
    register_prepared::<market::MarketHistory>(),
    register_prepared::<market::MarketLedger>(),
    register_prepared::<lend_lease_balance::LendLeaseBalance>(),
    register_prepared::<production_catalogue::ProductionCatalogue>(),
    register_prepared::<convoy_health::ConvoyHealth>(),
    register_prepared::<peace_deals::PeaceDeals>(),
    register_prepared::<resource_balance::ResourceBalance>(),
    register_prepared::<decryption::Decryption>(),
    register::<production_lines::ProductionLines>(),
    register::<equipment::designs::detail::EquipmentDesignDetail>(),
    register_prepared::<production_lines::ProductionLineDetail>(),
    register_prepared::<production_lines::ProductionLineBreakdown>(),
    register_prepared::<equipment_stockpile::EquipmentStockpile>(),
    register::<equipment_stockpile::artwork::StockpileArtwork>(),
    register::<equipment_produced::EquipmentProduced>(),
    register_prepared::<production_spending::ProductionSpending>(),
    register_prepared::<dockyard_spending::DockyardSpending>(),
    register_prepared::<military_factory_spending::MilitaryFactorySpending>(),
    register_prepared::<trade_flows::TradeFlows>(),
    register::<market_lendlease::MarketLendLease>(),
    register_prepared::<special_projects::SpecialProjects>(),
    register_prepared::<subjects::Subjects>(),
    register_prepared::<event_feed::EventFeed>(),
    register_prepared::<event_feed::prepared::script::EventChoices>(),
    register_prepared::<event_feed::prepared::script::Decisions>(),
    register_prepared::<world_tension::WorldTension>(),
    register_prepared::<autonomy::Autonomy>(),
    register_prepared::<construction_log::ConstructionLog>(),
    register_prepared::<construction_log::ConstructionLogManifest>(),
    register_prepared::<equipment_losses::EquipmentLosses>(),
    register_prepared::<military_air_ledger::MilitaryAirLedger>(),
    register::<ships_sunk::ShipsSunk>(),
    register::<ships_sunk::ShipsSunkBattleTitles>(),
    register_prepared::<naval_battles::NavalBattles>(),
    register_prepared::<naval_battles::NavalBattlesManifest>(),
    register_prepared::<naval_battles::NavalBattlesRanking>(),
    register_prepared::<naval_battles::NavalBattleDetails>(),
    register::<naval_losses::NavalLosses>(),
    register_prepared::<construction_spending::ConstructionSpending>(),
    register_prepared::<espionage_log::EspionageLog>(),
    register::<operation_description::OperationDescription>(),
    register::<intel_details::IntelDetails>(),
    register_prepared::<operative_placements::OperativePlacements>(),
    register_prepared::<operatives_roster::OperativesRoster>(),
    register_prepared::<agency_comparison::AgencyComparison>(),
    register::<aircraft_catalogue::AircraftCatalogue>(),
    register_prepared::<air_missions::AirMissions>(),
    register_prepared::<air_statistics::AirStatistics>(),
    register_prepared::<regional_weather::RegionalWeather>(),
    register_prepared::<minefields::Minefields>(),
    register_prepared::<naval_supremacy::NavalSupremacy>(),
    register_prepared::<doctrine_mastery::DoctrineMastery>(),
    register::<artwork::sprites::SpriteArtwork>(),
    register_prepared::<occupation_reserve::OccupationReserve>(),
    register_prepared::<intel_coverage::IntelCoverage>(),
    register_prepared::<naval_repairs::NavalRepairs>(),
    register_prepared::<naval_refits::NavalRefits>(),
];

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.iter().find(|product| product.key == key)
}

/// Prepared-only families keep their whole-recording loaders as test oracles.
/// Tests of the product route's own machinery (envelopes, reply cache, content
/// warnings) reach those loaders here; the service never does.
#[cfg(test)]
static ORACLE_ROUTES: &[Registered] = &[
    register::<world_tension::WorldTension>(),
    register::<war_casualties::WarCasualties>(),
    register::<war_casualties::pairs::CasualtyPairs>(),
    register::<war_casualties::breakdown::CasualtyBreakdown>(),
    register::<country_fuel::CountryFuel>(),
];

#[cfg(test)]
pub(crate) fn oracle_route(key: &str) -> Option<&'static Registered> {
    ORACLE_ROUTES.iter().find(|product| product.key == key)
}

#[cfg(test)]
mod recording_tests;

#[cfg(test)]
mod casualty_route_tests;
#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod source_warning_tests;
#[cfg(test)]
mod tension_query_tests;

#[cfg(test)]
mod supply_health_registry_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn exported_product_type_names_do_not_hide_different_shapes() {
        let mut declarations = std::collections::BTreeMap::new();
        for product in PRODUCTS {
            for declaration in (product.types)() {
                let name = declaration
                    .strip_prefix("type ")
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_owned();
                if let Some(previous) = declarations.insert(name.clone(), declaration.clone()) {
                    assert_eq!(
                        previous, declaration,
                        "different exported types named {name}"
                    );
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
    fn prepared_only_products_refuse_their_route_and_keep_their_contract() {
        struct Prepared;
        impl Product for Prepared {
            const KEY: &'static str = "prepared-only-fixture";
            const CONTENT_DEPENDENT: bool = true;
            type Params = BranchParams;
            type Reply = wars::WarsReply;
        }
        let registered = register_prepared::<Prepared>();
        assert!(registered.prepared_only && registered.content_dependent);
        assert!(!register::<Prepared>().prepared_only);
        assert_eq!((registered.types)(), Prepared::types());
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "prepared-only".into(),
                revision: "1".into(),
            },
            "prepared-only.battleplan".into(),
            std::sync::Arc::new(
                replay::container::Replay::from_records(
                    crate::test_fixtures::CAMPAIGN.into(),
                    &[crate::test_fixtures::records(
                        1,
                        vec![crate::test_fixtures::source(1, 0, None)],
                    )],
                )
                .unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap();
        let context = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);
        let refusal = "prepared-only-fixture is served by prepared queries";
        assert_eq!(
            (registered.load)(
                &source,
                "lease",
                serde_json::json!({"branchId": "1"}),
                &context,
                &cancelled
            ),
            Err(refusal.to_owned())
        );
        let params = BranchParams {
            branch_id: "1".into(),
        };
        let error = Prepared::load(&source, &params, &context, &cancelled)
            .err()
            .unwrap();
        assert_eq!(
            (error.kind(), error.to_string()),
            (io::ErrorKind::Unsupported, refusal.to_owned())
        );
    }

    #[test]
    fn faction_totals_are_registered_under_their_key() {
        assert_eq!(
            find("faction-totals").unwrap().key,
            faction_totals::FactionTotals::KEY
        );
    }

    #[test]
    fn peace_action_labels_participate_in_the_content_cache_contract() {
        assert!(
            find(peace_deals::PeaceDeals::KEY)
                .unwrap()
                .content_dependent
        );
    }

    #[test]
    fn recording_only_products_do_not_depend_on_content() {
        assert!(
            !find(country_measure::CountryMeasure::KEY)
                .unwrap()
                .content_dependent
        );
        assert!(
            find(country_fuel::CountryFuel::KEY)
                .unwrap()
                .content_dependent
        );
        // This product's values are recorded, but its building names/definitions
        // also belong to the reply and must recover when content becomes available.
        assert!(
            find(country_factories::CountryFactories::KEY)
                .unwrap()
                .content_dependent
        );
        let contract = crate::typescript_contract();
        for product in PRODUCTS.iter().filter(|product| product.content_dependent) {
            let declaration = contract
                .lines()
                .find(|line| line.starts_with("export const CONTENT_PRODUCTS"))
                .unwrap();
            assert!(declaration.contains(&format!("{:?}", product.key)));
        }
    }

    #[test]
    fn browser_interactive_products_match_service_admission() {
        let contract = crate::typescript_contract();
        let declaration = contract
            .lines()
            .find(|line| line.starts_with("export const INTERACTIVE_PRODUCTS"))
            .expect("browser admission needs the service's product lanes");
        for product in PRODUCTS {
            assert_eq!(
                declaration.contains(&format!("{:?}", product.key)),
                product.interactive,
                "{} admission differs between browser and service",
                product.key
            );
        }
    }

    #[test]
    fn product_keys_are_unique_and_exported_types_are_named_once() {
        let mut keys: Vec<_> = PRODUCTS.iter().map(|product| product.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRODUCTS.len());
        let contract = crate::typescript_contract();
        // A targeted registry test can also export the exact checked contract
        // for frontend consumers without building a separate generator binary.
        if let Some(path) = std::env::var_os("BATTLE_PLAN_TYPESCRIPT_OUTPUT").or_else(|| std::env::var_os("HOI4_TYPESCRIPT_OUTPUT")) {
            std::fs::write(path, &contract).unwrap();
        }
        let names: Vec<_> = contract
            .lines()
            .filter_map(|line| line.strip_prefix("export type "))
            .map(|rest| rest.split_whitespace().next().unwrap_or_default())
            .collect();
        for name in [
            "CountryParams",
            "CountryFlagSize",
            "CountryFlagImage",
            "LandOwners",
            "FocusReply",
            "ResearchReply",
            "CountryStyleReply",
            "CountryGroupReply",
            "CountryStabilityReply",
            "StabilityPartsReply",
            "FixedRun",
            "MeasureReply",
            "FuelReply",
            "CountryFactoriesReply",
            "CasualtyPairsParams",
            "CasualtyPairRun",
            "CasualtyPairSeries",
            "CasualtyPairsReply",
            "CasualtyBreakdownParams",
            "CasualtyBreakdownRun",
            "CasualtyBreakdownSeries",
            "CasualtyBreakdownCountryRun",
            "CasualtyWarNameRun",
            "CasualtyBreakdownWar",
            "CasualtyBreakdownReply",
            "AirForceReply",
            "NavyReply",
            "ExperienceReply",
            "IdeaHistoryReply",
            "PartyHistoryReply",
            "PeaceDealsReply",
            "ResourceBalanceReply",
            "ProductionLinesReply",
            "ProductionLineDetailReply",
            "DesignDetailReply",
            "DesignDetailParams",
            "DesignSection",
            "DesignField",
            "DesignChoice",
            "LineRun",
            "EquipmentStockpileReply",
            "EquipmentProducedReply",
            "AmountRun",
            "ResourceInfo",
            "TradeFlowsReply",
            "SubjectReply",
            "BranchParams",
            "TensionReply",
            "AutonomyReply",
            "ConstructionLogReply",
            "LossesReply",
            "ConstructionSpendingReply",
        ] {
            assert_eq!(
                names.iter().filter(|n| **n == name).count(),
                1,
                "{name} must be declared exactly once"
            );
        }
    }
}
