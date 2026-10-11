//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply that
//! covers a whole branch, so playback in the frontend is a local lookup. The
//! service, transport, host and frontend never name a product; they route by
//! its key.

pub mod agency_comparison;
pub mod air_aces;
pub mod air_force;
pub mod air_superiority;
pub(crate) mod air_wings;
pub mod army_size;
pub mod artwork;
pub mod autonomy;
pub mod character_portraits;
pub mod construction_log;
pub mod construction_spending;
pub mod country_experience;
pub mod country_factories;
pub mod country_flags;
pub mod country_fuel;
pub mod country_groups;
pub(crate) mod country_history;
pub mod country_measure;
mod country_name_cache;
pub mod country_nukes;
pub mod country_stability;
pub mod country_styles;
pub mod decryption;
pub mod division_log;
pub mod equipment;
pub mod equipment_icon;
pub mod equipment_losses;
pub mod equipment_produced;
pub mod equipment_stockpile;
pub mod espionage_log;
pub mod event_feed;
pub mod faction_totals;
pub(crate) mod fixed5_runs;
pub mod focus_artwork;
pub mod game_score;
pub mod idea_format;
pub mod idea_history;
pub mod identity;
pub mod intelligence_capture;
pub mod intelligence_content;
pub mod military_air_ledger;
pub mod operation_description;
pub mod operative_content;
pub mod operative_placements;
pub(crate) mod operative_rows;
pub mod operatives_roster;
pub use identity::civil_war;
pub mod battles;
mod force_census;
pub mod land_battles;
pub mod land_owners;
pub mod national_focus;
pub mod naval_battles;
pub mod naval_losses;
pub mod navy;
pub mod occupation_trend;
pub mod occupied_states;
pub mod party_history;
pub mod peace_deals;
pub mod place_names;
pub mod politics_content;
pub mod politics_ledger;
pub mod production_lines;
pub mod production_spending;
pub mod raids;
pub mod research;
pub mod resource_balance;
pub mod resources;
pub mod scoreboard;
pub mod ships_sunk;
pub mod sinkings;
pub mod special_projects;
pub mod stability_parts;
pub mod subjects;
pub mod supply_health;
pub mod technology_artwork;
pub mod territory;
pub mod territory_breakdown;
pub mod trade_flows;
pub mod unit_leaders;
pub mod war_casualties;
pub mod wars;
pub mod world_tension;

use std::{io, sync::atomic::AtomicBool};

use installed_content::ResolveOptions;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::{
    Source, reply_cache,
    transport::{MAX_REPLY_BYTES, bounded_json},
};

/// Leave room for the lease envelope and escaped product JSON inside the
/// transport's complete-response bound. History products share this reserve.
pub const MAX_PRODUCT_REPLY_BYTES: usize = 24 * 1024 * 1024;

/// Refuse a complete reply instead of dropping recorded entries to fit it.
pub(crate) fn check_reply_budget<T: Serialize>(reply: &T) -> io::Result<()> {
    if crate::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(format!(
            "This product reply exceeds the transport reply budget of {MAX_PRODUCT_REPLY_BYTES} bytes"
        )));
    }
    Ok(())
}

/// The fixed64 finite domain excludes the three
/// infinity/undefined sentinels while preserving every signed finite value.
pub(crate) fn finite_fixed64(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Game and user folders chosen in Settings, needed by products that read
/// artwork or localisation.
#[derive(Clone)]
pub struct ContentOptions {
    pub options: ResolveOptions,
    pub allow_unverified: bool,
}

pub struct LoadContext {
    pub content: Option<ContentOptions>,
}

impl LoadContext {
    pub fn content(&self) -> io::Result<&ContentOptions> {
        self.content
            .as_ref()
            .ok_or_else(|| crate::invalid("Choose the game and user folders in Settings first"))
    }
}

/// All product replies carry content diagnostics at the shared serialization boundary.
pub trait ReplyWarnings {
    fn warnings_mut(&mut self) -> &mut Vec<String>;
}

macro_rules! reply_warnings {
    ($($reply:ty),+ $(,)?) => {$(
        impl ReplyWarnings for $reply {
            fn warnings_mut(&mut self) -> &mut Vec<String> {
                &mut self.warnings
            }
        }
    )+};
}

reply_warnings!(
    agency_comparison::AgencyComparisonReply,
    air_aces::AirAcePortraitReply,
    army_size::ArmySizeReply,
    supply_health::SupplyHealthReply,
    unit_leaders::UnitLeadersResponse,
    unit_leaders::portraits::UnitLeaderPortraitsReply,
    air_force::AirForceReply,
    autonomy::AutonomyReply,
    construction_log::ConstructionLogReply,
    construction_log::ConstructionLogManifestReply,
    construction_spending::ConstructionSpendingReply,
    country_experience::ExperienceReply,
    country_factories::CountryFactoriesReply,
    country_flags::CountryFlagReply,
    country_fuel::FuelReply,
    country_fuel::FuelAwardsReply,
    country_fuel::FuelAwardHistoryReply,
    country_fuel::FuelConsumptionReply,
    country_fuel::FuelDemandReply,
    country_fuel::FuelDemandHistoryReply,
    country_fuel::FuelDemandStackReply,
    country_groups::CountryGroupReply,
    country_measure::MeasureReply,
    country_nukes::CountryNukesReply,
    country_stability::CountryStabilityReply,
    country_styles::CountryStyleReply,
    decryption::DecryptionReply,
    equipment_losses::LossesReply,
    military_air_ledger::MilitaryAirReply,
    equipment_produced::EquipmentProducedReply,
    equipment_stockpile::EquipmentStockpileReply,
    equipment_stockpile::artwork::StockpileArtworkReply,
    espionage_log::EspionageLogReply,
    operation_description::OperationDescriptionReply,
    operative_placements::OperativePlacementReply,
    operatives_roster::OperativeRosterReply,
    faction_totals::FactionTotalsReply,
    game_score::GameScoreReply,
    event_feed::EventFeedReply,
    focus_artwork::FocusArtworkReply,
    idea_history::IdeaHistoryReply,
    crate::LandOwners,
    national_focus::FocusReply,
    navy::NavyReply,
    naval_losses::NavalLossesReply,
    occupation_trend::OccupationTrendReply,
    occupied_states::OccupiedStatesReply,
    occupied_states::OccupiedStatesManifestReply,
    party_history::PartyHistoryReply,
    politics_ledger::PoliticsLedgerReply,
    peace_deals::PeaceDealsReply,
    place_names::PlaceNamesReply,
    production_lines::ProductionLinesReply,
    production_lines::ProductionLineDetailReply,
    production_lines::ProductionLineBreakdownReply,
    production_spending::ProductionSpendingReply,
    raids::RaidsReply,
    research::ResearchReply,
    technology_artwork::TechnologyArtworkReply,
    resource_balance::ResourceBalanceReply,
    scoreboard::ScoreboardReply,
    ships_sunk::ShipsSunkReply,
    special_projects::SpecialProjectsReply,
    stability_parts::StabilityPartsReply,
    subjects::SubjectReply,
    territory::TerritoryReply,
    territory_breakdown::TerritoryBreakdownReply,
    trade_flows::TradeFlowsReply,
    war_casualties::CasualtyReply,
    wars::WarsReply,
    war_casualties::pairs::CasualtyPairsReply,
    war_casualties::breakdown::CasualtyBreakdownReply,
    world_tension::TensionReply,
);

pub trait Product: 'static {
    const KEY: &'static str;
    /// Whether any reply field reads localisation, artwork or content definitions.
    const CONTENT_DEPENDENT: bool;
    /// Whether it answers a hover or label at one hour (one block at most): served on its
    /// own threads, so a whole-recording scan never delays it.
    const INTERACTIVE: bool = false;
    type Params: DeserializeOwned + TS + 'static;
    type Reply: Serialize + TS + ReplyWarnings + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    /// TypeScript declarations this product introduces, parameters and reply first.
    fn types() -> Vec<String> {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply>;
}

/// Parameters shared by the per-country products.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryParams {
    pub branch_id: String,
    pub tag: String,
}

impl CountryParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_tag(&self.tag)
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// Parameters of a product about the whole world of one branch.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchParams {
    pub branch_id: String,
}

impl BranchParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// Selection checks shared by products drawing several countries.
pub fn validate_countries(branch_id: &str, tags: &[String]) -> Result<(), &'static str> {
    if tags.is_empty() {
        return Err("Invalid country selection");
    }
    for tag in tags {
        validate_tag(tag)?;
    }
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    Ok(())
}

/// The one check for a requested country tag, shared by products and the map.
/// Tags are lookup keys compared with recorded tags, never paths. Definition
/// validity belongs to the content loader; requests refuse only emptiness and NUL.
pub fn validate_tag(tag: &str) -> Result<(), &'static str> {
    if tag.is_empty() || tag.contains('\0') {
        return Err("Invalid country tag");
    }
    Ok(())
}

/// A census API has no warnings field, so an unrepresentable key refuses its map.
fn require_displayable_tags(
    refused: &std::collections::BTreeMap<Vec<u8>, usize>,
) -> io::Result<()> {
    if refused.is_empty() {
        return Ok(());
    }
    Err(crate::invalid(format!(
        "Country tags are not UTF-8 (bytes: observation count): {refused:?}; the census cannot be represented"
    )))
}

/// The rows a table holds this hour, complete or truncated; an unavailable table
/// holds none. For products that use whatever rows were recorded.
pub(crate) fn recorded_rows<T: replay::rows::TableSchema>(
    frame: &replay::frame::DecodedFrame,
) -> io::Result<&[T::Row]> {
    use replay::frame::TableView;
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().ok_or_else(|| {
                crate::invalid(format!(
                    "hour {} {:?} contains another table's rows",
                    frame.hour,
                    T::ID
                ))
            })
        }
        TableView::Unavailable { .. } => Ok(&[]),
    }
}

/// A table-state reason that can be shared across dated history runs. Read
/// dates and changing truncation counts belong to coverage, not reason text.
pub(crate) fn table_missing_reason(
    frame: &replay::frame::DecodedFrame,
    table: replay::rows::TableId,
) -> Option<String> {
    use replay::frame::TableView;
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) if rows.table() == table => None,
        TableView::Complete(_) => Some(format!("{table:?} contains another table's rows")),
        TableView::Truncated { .. } => Some(format!("{table:?} is truncated")),
        TableView::Unavailable { reason } => {
            if reason.starts_with("missing table baseline") {
                return Some(format!("{table:?} baseline missing"));
            }
            // FrameEncoder withholds partial rows at keyframes, retaining the
            // truncation count in its named unavailable marker.
            let truncated = reason
                .strip_prefix("truncated at keyframe: ")
                .and_then(|count| count.strip_suffix(" rows not read"))
                .is_some_and(|count| count.parse::<u32>().is_ok_and(|count| count != 0));
            Some(if truncated {
                format!("{table:?} is truncated")
            } else {
                reason.clone()
            })
        }
    }
}

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
}

const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        load: load_json::<P>,
        types: types::<P>,
    }
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
    register::<research::Research>(),
    register::<technology_artwork::TechnologyArtwork>(),
    register::<country_flags::CountryFlags>(),
    register::<army_size::ArmySize>(),
    register::<supply_health::SupplyHealth>(),
    register::<division_log::DivisionLog>(),
    register::<division_log::DivisionLogManifest>(),
    register::<division_log::DivisionLogTotals>(),
    register::<unit_leaders::UnitLeaders>(),
    register::<unit_leaders::portraits::UnitLeaderPortraits>(),
    register::<land_owners::LandOwnersProduct>(),
    register::<land_battles::LandBattles>(),
    register::<land_battles::LandBattlesManifest>(),
    register::<land_battles::LandBattlesRanking>(),
    register::<place_names::PlaceNames>(),
    register::<country_styles::CountryStyles>(),
    register::<country_groups::CountryGroups>(),
    register::<wars::Wars>(),
    register::<country_stability::CountryStability>(),
    register::<stability_parts::StabilityParts>(),
    register::<country_measure::CountryMeasure>(),
    register::<country_fuel::CountryFuel>(),
    register::<country_fuel::CountryFuelAwards>(),
    register::<country_fuel::CountryFuelAwardHistory>(),
    register::<country_fuel::CountryFuelConsumption>(),
    register::<country_fuel::CountryFuelDemand>(),
    register::<country_fuel::CountryFuelDemandHistory>(),
    register::<country_fuel::CountryFuelDemandStack>(),
    register::<war_casualties::WarCasualties>(),
    register::<war_casualties::pairs::CasualtyPairs>(),
    register::<war_casualties::breakdown::CasualtyBreakdown>(),
    register::<country_factories::CountryFactories>(),
    register::<territory::Territory>(),
    register::<territory_breakdown::TerritoryBreakdown>(),
    register::<faction_totals::FactionTotals>(),
    register::<air_aces::AirAces>(),
    register::<air_aces::AirAcePortraits>(),
    register::<air_force::AirForceSize>(),
    register::<navy::NavySize>(),
    register::<game_score::GameScore>(),
    register::<scoreboard::Scoreboard>(),
    register::<country_nukes::CountryNukes>(),
    register::<air_superiority::AirSuperiority>(),
    register::<raids::Raids>(),
    register::<occupied_states::OccupiedStates>(),
    register::<occupied_states::OccupiedStatesManifest>(),
    register::<occupation_trend::OccupationTrend>(),
    register::<country_experience::CountryExperience>(),
    register::<idea_history::IdeaHistory>(),
    register::<party_history::PartyHistory>(),
    register::<politics_ledger::PoliticsLedger>(),
    register::<peace_deals::PeaceDeals>(),
    register::<resource_balance::ResourceBalance>(),
    register::<decryption::Decryption>(),
    register::<production_lines::ProductionLines>(),
    register::<production_lines::ProductionLineDetail>(),
    register::<production_lines::ProductionLineBreakdown>(),
    register::<equipment_stockpile::EquipmentStockpile>(),
    register::<equipment_stockpile::artwork::StockpileArtwork>(),
    register::<equipment_produced::EquipmentProduced>(),
    register::<production_spending::ProductionSpending>(),
    register::<trade_flows::TradeFlows>(),
    register::<special_projects::SpecialProjects>(),
    register::<subjects::Subjects>(),
    register::<event_feed::EventFeed>(),
    register::<world_tension::WorldTension>(),
    register::<autonomy::Autonomy>(),
    register::<construction_log::ConstructionLog>(),
    register::<construction_log::ConstructionLogManifest>(),
    register::<equipment_losses::EquipmentLosses>(),
    register::<military_air_ledger::MilitaryAirLedger>(),
    register::<ships_sunk::ShipsSunk>(),
    register::<ships_sunk::ShipsSunkBattleTitles>(),
    register::<naval_battles::NavalBattles>(),
    register::<naval_battles::NavalBattlesManifest>(),
    register::<naval_battles::NavalBattlesRanking>(),
    register::<naval_battles::NavalBattleDetails>(),
    register::<naval_losses::NavalLosses>(),
    register::<construction_spending::ConstructionSpending>(),
    register::<espionage_log::EspionageLog>(),
    register::<operation_description::OperationDescription>(),
    register::<operative_placements::OperativePlacements>(),
    register::<operatives_roster::OperativesRoster>(),
    register::<agency_comparison::AgencyComparison>(),
];

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
