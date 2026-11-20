//! Capture scope. Single source of truth for which tables each [`CaptureMode`] records.

use crate::rows::TableId;
pub use replay_identity::CaptureMode;

pub const fn records(mode: CaptureMode, table: TableId) -> bool {
    match mode {
        // Owner decision 2026-10-23: Full's DivisionStatus already holds what Map's witness does.
        CaptureMode::Full => !matches!(table, TableId::DivisionModelInputs),
        CaptureMode::Map => map_records(table),
    }
}

/// What a recording in `mode` leaves out, as a reader names it; None for Full.
pub const fn omits(mode: CaptureMode) -> Option<&'static str> {
    match mode {
        CaptureMode::Full => None,
        // Owner decision 2026-10-23: kept views withhold what reads beyond this scope, and say so here.
        CaptureMode::Map => Some(concat!(
            "division names, organisation and strength (counters and battle windows show none), ",
            "land counters' supply, attrition, dig-in and order glyphs, ",
            "division details, fog of war, supply, fronts and plans, raids, ",
            "some hover details, most map modes and every dashboard",
        )),
    }
}

pub fn omitted(mode: CaptureMode) -> impl Iterator<Item = TableId> {
    TableId::ALL
        .into_iter()
        .filter(move |table| !records(mode, *table))
}

/// Owner decision 2026-10-23: the map, its counters and battle windows, without DivisionStatus.
const fn map_records(table: TableId) -> bool {
    use TableId::*;
    matches!(
        table,
        // Session, clock and the readers' lexicon.
        Campaign
            | Clock
            | Players
            | LocalPlayer
            | LandUnitOrder
            | HumanControllers
            | SimulationFeatures
            | LexicalTokens
            // Political map, subject colours and land counters.
            | Countries
            | CountryLinks
            | Provinces
            | States
            | StateFlags
            | CountryAppearance
            | ReservedCountryNaming
            | BorderConflicts
            | Divisions
            | DivisionCensus
            | ProvinceUnitOrder
            | DivisionMovement
            | ArmyIconInputs
            | Subjects
            | DivisionTemplateIcons
            | DivisionTemplates
            | CountryTemplateState
            // Owner decision 2026-10-23: 3D land figures, their model inputs and model choice.
            | DivisionModelInputs
            | DivisionEquipment
            | NewDivisionEquipment
            | CountryLocalSubunits
            | VariantUpgrades
            | AvailableEquipmentVariants
            | Factions
            // Counter colours, order icons, abilities, portraits and railway guns.
            | ArmyOrders
            | ArmyOrderPlans
            | OrdersGroupPlans
            | OrdersGroupCounterInputs
            | ActiveAbilities
            | LeaderPortraits
            | LeaderPortraitPaths
            | CharacterPortraits
            | RailwayGunCounterInputs
            | RailCooldowns
            // Allegiance and shared vision.
            | FactionMembers
            | FactionTheatres
            | RelationLinks
            | WarRelations
            | CountryWars
            | VolunteerHosts
            | SentVolunteers
            | CountryCivilWars
            // Land and naval battle markers.
            | LandCombats
            | LandCombatSides
            | NavalCombats
            | NavalResults
            | NavalResultMembers
            | NavalResultCountries
            | NavalCombatResultDetails
            // Air bases, wings and mission regions.
            | AirBases
            | AirBaseRegistrations
            | AirWings
            | WingEquipment
            | VariantDefinitions
            | StateBuildings
            | ProvinceBuildings
            // The state view's shared-slot and factory-energy operands, read daily.
            | StateBuildingInputs
            | AirRegionalMissions
            | AirReferenceWitnesses
            | AirMissionInputs
            | AirRegionCombat
            | AirGroundCrews
            | AirSortieBaseCountries
            | AirSortieModifiers
            | AirSortieSpecialBases
            | AirSortieModifierMetadata
            // Strategic Air, the map mode that shows wing grids and mission regions, with a
            // selected wing's range and assignment and its regions' mine hovers.
            | RaidAirScores
            | RaidCountryInputs
            | AceRangeInputs
            | CountryCommandPower
            | RaidAirSites
            | NavalRegionalMines
            | AirCountryRelations
            | NavalAccidentReports
            | Capitulation
            | CountryIntel
            // Which recorded air combats survive the daily prune, for the province hover.
            | AirDailyRoster
            // Owner decision: nuclear strike effects, floating map text and bombardment explosions.
            | NuclearStrikes
            | MapEvents
            // Owner decision: every animation. Opposed and naval-patrol aircraft with the transports
            // that follow them, and air transfer arrows.
            | AirNavalPatrolCombats
            | AirNavalPatrolHistories
            | AirTransferInputs
            // Trade-route lines and their convoys, drawn in the navy map mode.
            | TradeRoutes
            | ConvoyClients
            | RegionalConvoyEfficiency
            | StrategicRegionWeights
            | ResourceExchanges
            | ResourceOrigins
            | NavalSupremacy
            | FacilityMapFlags
            | StateCores
            | RadarCoverage
            // Task forces and their missions.
            | TaskForceMapMotion
            | TaskForces
            | Ships
            | ShipModels
            | Fleets
            | TaskForceCounterInputs
            | FleetCounterInputs
            | NavalBases
            | NavalRepairBases
            | NavalRepairQueue
            | TaskForceMissions
            | NavalMissionLedgers
            | NavalMissionTaskForces
            | NavalMissionCountries
            | NavalMissionCommanders
            // The witness each commander reference needs.
            | SliceReads
            // Choosing a fleet reads its owner's score inputs.
            | CountryGameScoreInputs
            // Weather.
            | WeatherProvinces
            | WeatherRegions
            // Live naval battle markers place transferring units.
            | NavalProvinceCombats
            | NavalCombatTransfers
            | NavalTransferUnits
            // Land battle window.
            | LandCombatDetails
            | LandCombatDivisions
            | LandCombatModifiers
            | LandCombatDivisionModifiers
            | ReserveModifierTrees
            | CountryTacticUnlocks
            | OrdersGroups
            | RelationTuples
            // Naval battle window.
            | NavalCombatDetails
            | NavalCombatShips
            | NavalCombatParticipants
            | NavalCombatAirBaseWingHandles
            | NavalModifierTrees
            | CountryCounterInputs
            // Both windows: flags, names, commanders and modifier text.
            | Politics
            | PoliticalParties
            | Autonomy
            | ProvinceNames
            | UnitLeaders
            | UnitLeaderTraits
            | Characters
            | CharacterRoleTraits
            | ModifierDefinitions
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_records_every_table_but_the_model_witness_and_map_omits_division_status() {
        assert_eq!(
            omitted(CaptureMode::Full).collect::<Vec<_>>(),
            [TableId::DivisionModelInputs]
        );
        assert!(omits(CaptureMode::Full).is_none());
        assert!(!records(CaptureMode::Map, TableId::DivisionStatus));
        for table in [
            TableId::Clock,
            TableId::Countries,
            TableId::Divisions,
            TableId::LandUnitOrder,
            TableId::StateBuildingInputs,
            TableId::LandCombats,
            TableId::NavalCombatShips,
            TableId::WeatherRegions,
        ] {
            assert!(records(CaptureMode::Map, table), "{table:?}");
        }
        assert!(omitted(CaptureMode::Map).any(|table| table == TableId::CountryFactories));
    }

    #[test]
    fn map_records_the_land_model_witness_and_model_choice() {
        for table in [
            TableId::DivisionModelInputs,
            TableId::DivisionEquipment,
            TableId::NewDivisionEquipment,
            TableId::CountryLocalSubunits,
            TableId::VariantUpgrades,
            TableId::AvailableEquipmentVariants,
            TableId::Factions,
            TableId::DivisionTemplateIcons,
            TableId::VariantDefinitions,
        ] {
            assert!(records(CaptureMode::Map, table), "{table:?}");
        }
        let omits = omits(CaptureMode::Map).unwrap();
        assert!(!omits.contains("3D unit figures"), "{omits}");
        // The witness's exercise predicate draws the land counters' training glyph.
        assert!(
            omits.contains("land counters' supply, attrition, dig-in and order glyphs")
                && !omits.contains("training"),
            "{omits}"
        );
    }

    #[test]
    fn map_records_only_tables_its_format_records() {
        let format = crate::FORMAT_VERSION;
        for table in TableId::ALL {
            if records(CaptureMode::Map, table) {
                assert!(table.last_change().present(format), "{table:?}");
            }
        }
    }
}
