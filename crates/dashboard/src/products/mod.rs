//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply that
//! covers a whole branch, so playback in the frontend is a local lookup. The
//! service, transport, host and frontend never name a product; they route by
//! its key.

pub mod agency_comparison;
pub mod air_force;
pub mod army_size;
pub mod autonomy;
pub mod country_flags;
pub mod country_fuel;
mod country_name_cache;
pub mod country_nukes;
pub mod country_stability;
pub mod decryption;
pub mod division_log;
pub mod espionage_log;
pub mod event_feed;
pub mod focus_artwork;
pub mod game_score;
pub mod intelligence_capture;
pub mod intelligence_content;
pub(crate) mod intelligence_prepared;
pub mod operation_description;
pub mod operative_content;
pub mod operative_placements;
pub(crate) mod operative_rows;
pub mod operatives_roster;
pub use identity::civil_war;
pub mod land_battles;
pub mod land_owners;
pub mod national_focus;
pub mod naval_battles;
pub mod naval_losses;
pub mod navy;
pub mod occupation_trend;
pub mod occupied_states;
pub(crate) mod parts_race_prepared;
pub mod party_history;
pub mod peace_deals;
pub mod scoreboard;
pub mod ships_sunk;
pub mod sinkings;
pub mod special_projects;
pub mod stability_parts;
pub mod subjects;
pub mod supply_health;
pub mod technology_artwork;
pub mod war_casualties;
pub mod wars;
pub mod world_tension;

// The product contract and the shared products, at their old paths.
pub use dashboard_air_aces::products::air_aces;
pub use dashboard_air_superiority::products::air_superiority;
pub use dashboard_base::products::*;
pub use dashboard_construction::products::construction_log;
pub use dashboard_construction::products::construction_spending;
pub use dashboard_equipment_losses::products::equipment_losses;
pub use dashboard_equipment_stockpile::products::equipment_stockpile;
pub use dashboard_military_air_ledger::products::military_air_ledger;
pub use dashboard_politics_ledger::products::politics_ledger;
pub use dashboard_raids::products::raids;
pub use dashboard_resource_balance::products::resource_balance;
pub use dashboard_resource_balance::products::trade_flows;
pub use dashboard_unit_leaders::products::unit_leaders;

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::{
    Source, reply_cache,
    transport::{MAX_REPLY_BYTES, bounded_json},
};

dashboard_base::reply_warnings!(
    agency_comparison::AgencyComparisonReply,
    army_size::ArmySizeReply,
    supply_health::SupplyHealthReply,
    air_force::AirForceReply,
    autonomy::AutonomyReply,
    country_flags::CountryFlagReply,
    country_fuel::FuelReply,
    country_fuel::FuelAwardsReply,
    country_fuel::FuelAwardHistoryReply,
    country_fuel::FuelConsumptionReply,
    country_fuel::FuelDemandReply,
    country_fuel::FuelDemandHistoryReply,
    country_fuel::FuelDemandStackReply,
    country_nukes::CountryNukesReply,
    country_stability::CountryStabilityReply,
    decryption::DecryptionReply,
    espionage_log::EspionageLogReply,
    operation_description::OperationDescriptionReply,
    operative_placements::OperativePlacementReply,
    operatives_roster::OperativeRosterReply,
    game_score::GameScoreReply,
    event_feed::EventFeedReply,
    focus_artwork::FocusArtworkReply,
    crate::LandOwners,
    national_focus::FocusReply,
    navy::NavyReply,
    naval_losses::NavalLossesReply,
    occupation_trend::OccupationTrendReply,
    occupied_states::OccupiedStatesReply,
    occupied_states::OccupiedStatesManifestReply,
    party_history::PartyHistoryReply,
    peace_deals::PeaceDealsReply,
    technology_artwork::TechnologyArtworkReply,
    scoreboard::ScoreboardReply,
    ships_sunk::ShipsSunkReply,
    special_projects::SpecialProjectsReply,
    stability_parts::StabilityPartsReply,
    subjects::SubjectReply,
    war_casualties::CasualtyReply,
    wars::WarsReply,
    war_casualties::pairs::CasualtyPairsReply,
    war_casualties::breakdown::CasualtyBreakdownReply,
    world_tension::TensionReply,
);

/// Serialised with the lease identity the frontend used, so a reply can never
/// be attributed to another recording.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T: Serialize> {
    source_id: &'a str,
    revision: &'a str,
    product: &'static str,
    data: T,
}

/// A product erased to its key and JSON entry points.
pub struct Registered {
    pub key: &'static str,
    pub content_dependent: bool,
    pub interactive: bool,
    pub load:
        fn(&Source, &str, serde_json::Value, &LoadContext, &AtomicBool) -> Result<String, String>,
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
    // Whole-recording replies persist between sessions; hover answers are quicker to compute.
    let entry = (!P::INTERACTIVE)
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
            let mut data =
                P::load(source, &params, context, cancelled).map_err(|error| error.to_string())?;
            source.append_content_warnings(data.warnings_mut());
            let data = bounded_json(&data, cancelled)?;
            if let Some(entry) = &entry
                && !source.reads_uncovered()
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

fn types<P: Product>() -> Vec<String> {
    P::types()
}

/// Every product the service answers. Add a product here and in the frontend
/// widget registry; nothing else needs to change.
pub static PRODUCTS: &[Registered] = &[
    register::<national_focus::NationalFocus>(),
    register::<focus_artwork::FocusArtwork>(),
    register_prepared::<research::Research>(),
    register::<technology_artwork::TechnologyArtwork>(),
    register::<country_flags::CountryFlags>(),
    register_prepared::<army_size::ArmySize>(),
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
    register::<place_names::PlaceNames>(),
    register::<country_styles::CountryStyles>(),
    register_prepared::<country_groups::CountryGroups>(),
    register_prepared::<wars::Wars>(),
    register::<country_stability::CountryStability>(),
    register::<stability_parts::StabilityParts>(),
    register::<country_measure::CountryMeasure>(),
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
    register_prepared::<politics_ledger::PoliticsLedger>(),
    register_prepared::<peace_deals::PeaceDeals>(),
    register_prepared::<resource_balance::ResourceBalance>(),
    register_prepared::<decryption::Decryption>(),
    register::<production_lines::ProductionLines>(),
    register::<production_lines::ProductionLineDetail>(),
    register::<production_lines::ProductionLineBreakdown>(),
    register_prepared::<equipment_stockpile::EquipmentStockpile>(),
    register::<equipment_stockpile::artwork::StockpileArtwork>(),
    register::<equipment_produced::EquipmentProduced>(),
    register_prepared::<production_spending::ProductionSpending>(),
    register_prepared::<trade_flows::TradeFlows>(),
    register_prepared::<special_projects::SpecialProjects>(),
    register_prepared::<subjects::Subjects>(),
    register_prepared::<event_feed::EventFeed>(),
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
    register_prepared::<operative_placements::OperativePlacements>(),
    register_prepared::<operatives_roster::OperativesRoster>(),
    register_prepared::<agency_comparison::AgencyComparison>(),
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
mod source_warning_tests;

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
