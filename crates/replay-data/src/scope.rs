
use crate::rows::TableId;
pub use replay_identity::CaptureMode;

pub const fn records(mode: CaptureMode, table: TableId) -> bool {
    match mode {
        CaptureMode::Full => !matches!(table, TableId::DivisionModelInputs),
        CaptureMode::Map => map_records(table),
    }
}

pub const fn omits(mode: CaptureMode) -> Option<&'static str> {
    match mode {
        CaptureMode::Full => None,
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

const fn map_records(table: TableId) -> bool {
    use TableId::*;
    matches!(
        table,
        Campaign
            | Clock
            | Players
            | LocalPlayer
            | LandUnitOrder
            | HumanControllers
            | SimulationFeatures
            | LexicalTokens
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
            | DivisionModelInputs
            | DivisionEquipment
            | NewDivisionEquipment
            | CountryLocalSubunits
            | VariantUpgrades
            | AvailableEquipmentVariants
            | Factions
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
            | FactionMembers
            | FactionTheatres
            | RelationLinks
            | WarRelations
            | CountryWars
            | VolunteerHosts
            | SentVolunteers
            | CountryCivilWars
            | LandCombats
            | LandCombatSides
            | CombatEngagementRefreshes
            | NavalCombats
            | NavalResults
            | NavalResultMembers
            | NavalResultCountries
            | NavalCombatResultDetails
            | AirBases
            | AirBaseRegistrations
            | AirWings
            | WingEquipment
            | VariantDefinitions
            | StateBuildings
            | ProvinceBuildings
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
            | AirDailyRoster
            | NuclearStrikes
            | MapEvents
            | AirNavalPatrolCombats
            | AirNavalPatrolHistories
            | AirTransferInputs
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
            | SliceReads
            | CountryGameScoreInputs
            | WeatherProvinces
            | WeatherRegions
            | NavalProvinceCombats
            | NavalCombatTransfers
            | NavalTransferUnits
            | LandCombatDetails
            | LandCombatDivisions
            | LandCombatModifiers
            | LandCombatDivisionModifiers
            | ReserveModifierTrees
            | CountryTacticUnlocks
            | OrdersGroups
            | RelationTuples
            | NavalCombatDetails
            | NavalCombatShips
            | NavalCombatParticipants
            | NavalCombatAirBaseWingHandles
            | NavalModifierTrees
            | CountryCounterInputs
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
