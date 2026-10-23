//! Capture scope. Single source of truth for which tables each [`CaptureMode`] records.

use crate::rows::TableId;
pub use replay_identity::CaptureMode;

pub const fn records(mode: CaptureMode, table: TableId) -> bool {
    match mode {
        CaptureMode::Full => true,
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
            "3D unit figures, division details, fog of war, supply, fronts and plans, raids, ",
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
    fn full_records_every_table_and_map_omits_division_status() {
        assert_eq!(omitted(CaptureMode::Full).count(), 0);
        assert!(!records(CaptureMode::Map, TableId::DivisionStatus));
        for table in [
            TableId::Clock,
            TableId::Countries,
            TableId::Divisions,
            TableId::LandCombats,
            TableId::NavalCombatShips,
            TableId::WeatherRegions,
        ] {
            assert!(records(CaptureMode::Map, table), "{table:?}");
        }
        assert!(omitted(CaptureMode::Map).any(|table| table == TableId::CountryFactories));
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
