use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub enum Role {
    Characters,
    CountryLeaderRoles,
    AdvisorRoles,
    CharacterTraits,
    Relations,
    AirWing,
    AirStats,
    AirBases,
    Naval,
    Supremacy,
    Sunk,
    Capitulation,
    Players,
    Projects,
    Lexical,
    Session,
    Weather,
    Radar,
    MapCombats,
    ObserverBoundary,
    Divisions,
    DivisionInputParts,
    World,
    Economy,
    Governance,
    Variants,
    Research,
    Intel,
    Presentation,
    FinalInputs,
    IntelResidual,
    SupplyTrains,
    CharacterPortraits,
    ProvinceUnitOrder,
    Ordinary,
    OrdinaryCombat,
    OrdinaryNaval,
    OrdinaryMap,
    OrdinaryTaskForces,
    OrdinaryNavies,
    Slow,
    Stability,
    WarOverview,
    IndustrialOrganisations,
    ProductionDaily,
    ProductionModifierParts,
    RaidState,
    RaidLaunch,
    RaidTargets,
    AirRelations,
    AirMissions,
    AirHistories,
    Late,
    ScoresRails,
    ArmyIcons,
    Orders,
    OrdersPart,
    OrdersStitch,
    ChildFronts,
    Fronts,
    Gated,
    ShipModels,
    ShipModelDetails,
    Provinces,
    Factions,
    Countries,
    Wars,
    States,
    Templates,
    CharacterInputs,
    PresentationMovement,
    DivisionInputs,
    OrderRoots,
    PresentationSpeedTerms,
    PresentationPrepare,
    PresentationStatus,
    PresentationPreparation,
    PresentationSettle,
    DivisionModelInputs,
    GatedProjects,
    ProjectsRestPrimary,
    ProjectsRestAux,
    ProjectsRestPaths,
    Buildings,
    RaidStateInputs,
    Rails,
    IntelResidualAux,
    AirSupply,
    ScriptVariables,
}

impl Role {
    pub const fn worker_only(self) -> bool {
        true
    }
    pub const fn callback_help(self) -> bool {
        matches!(
            self,
            Role::Characters
                | Role::CountryLeaderRoles
                | Role::AdvisorRoles
                | Role::CharacterTraits
                | Role::Relations
                | Role::AirWing
                | Role::AirStats
                | Role::AirBases
                | Role::Capitulation
                | Role::Players
                | Role::Projects
                | Role::Session
                | Role::Weather
                | Role::Radar
                | Role::OrderRoots
                | Role::DivisionInputs
                | Role::DivisionInputParts
                | Role::Divisions
                | Role::PresentationPreparation
                | Role::PresentationStatus
                | Role::World
                | Role::Economy
                | Role::CharacterPortraits
                | Role::ProvinceUnitOrder
                | Role::AirRelations
                | Role::AirHistories
                | Role::ArmyIcons
                | Role::Orders
                | Role::OrdersPart
                | Role::Fronts
                | Role::ShipModels
                | Role::Countries
                | Role::Wars
                | Role::States
                | Role::AirSupply
        )
    }

    pub fn read_anchor(self) -> replay_data::rows::TableId {
        use replay_data::rows::TableId as T;
        match self {
            Self::Characters => T::Characters,
            Self::CountryLeaderRoles => T::CountryLeaderRoles,
            Self::AdvisorRoles => T::AdvisorRoles,
            Self::CharacterTraits => T::UnitLeaderTraits,
            Self::Relations => T::RelationTuples,
            Self::AirWing => T::AirWings,
            Self::AirStats => T::AirStatistics,
            Self::AirBases => T::AirBases,
            Self::Naval => T::OrdersGroups,
            Self::Supremacy => T::NavalSupremacy,
            Self::Sunk => T::SunkShips,
            Self::Capitulation => T::Capitulation,
            Self::Players => T::Players,
            Self::Projects => T::ProjectPrograms,
            Self::Lexical => T::LexicalTokens,
            Self::Session => T::WorldTension,
            Self::Weather => T::WeatherRegions,
            Self::Radar => T::RadarCoverage,
            Self::MapCombats => T::LandCombats,
            Self::ObserverBoundary => T::LandEventCoverage,
            Self::Divisions | Self::DivisionInputs | Self::DivisionInputParts => T::Divisions,
            Self::World | Self::Economy => T::ProvinceSupply,
            Self::Governance => T::Politics,
            Self::Buildings => T::ProvinceBuildings,
            Self::Variants => T::VariantDefinitions,
            Self::Research => T::ResearchInputs,
            Self::Intel => T::UnitIntel,
            Self::Presentation
            | Self::PresentationPrepare
            | Self::PresentationSettle
            | Self::PresentationStatus
            | Self::PresentationPreparation
            | Self::FinalInputs => T::DivisionStatus,
            Self::IntelResidual => T::Capitulation,
            Self::IntelResidualAux => T::Subjects,
            Self::ScriptVariables => T::ScriptVariables,
            Self::SupplyTrains => T::SupplyNodes,
            Self::CharacterPortraits => T::CharacterPortraits,
            Self::ProvinceUnitOrder => T::ProvinceUnitOrder,
            Self::Ordinary
            | Self::OrdinaryCombat
            | Self::OrdinaryNaval
            | Self::OrdinaryMap
            | Self::OrdinaryTaskForces
            | Self::OrdinaryNavies => T::LandCombatDetails,
            Self::Slow => T::ProvinceNames,
            Self::Stability => T::CountryStabilityInputs,
            Self::WarOverview => T::WarOwnedRelations,
            Self::IndustrialOrganisations => T::IndustrialOrganisations,
            Self::ProductionDaily | Self::ProductionModifierParts => T::CountryProductionModifiers,
            Self::RaidState | Self::RaidStateInputs => T::StateResourceInputs,
            Self::RaidLaunch => T::RaidCountryInputs,
            Self::RaidTargets => T::RaidTargetStatuses,
            Self::AirRelations => T::AirCountryRelations,
            Self::AirMissions => T::AirRegionalMissions,
            Self::AirHistories => T::AirNavalPatrolHistories,
            Self::Late => T::WingEquipment,
            Self::ScoresRails => T::RaidAirScores,
            Self::Rails => T::RailCooldowns,
            Self::AirSupply => T::AirGroundCrews,
            Self::ArmyIcons => T::ArmyIconInputs,
            Self::Orders | Self::OrderRoots => T::ArmyOrders,
            Self::OrdersPart => T::ArmyOrders,
            Self::OrdersStitch => T::ArmyOrderPlans,
            Self::ChildFronts => T::ArmyOrderPlans,
            Self::Fronts => T::Fronts,
            Self::Gated => T::VolunteerHosts,
            Self::GatedProjects => T::Projects,
            Self::ProjectsRestPrimary => T::Raids,
            Self::ProjectsRestAux => T::RaidIntelInputs,
            Self::ProjectsRestPaths => T::Scientists,
            Self::ShipModels => T::Ships,
            Self::ShipModelDetails => T::ShipModels,
            Self::Provinces => T::Provinces,
            Self::Factions => T::Factions,
            Self::Countries => T::Countries,
            Self::Wars => T::WarRelations,
            Self::States => T::States,
            Self::Templates => T::DivisionTemplateIcons,
            Self::CharacterInputs => T::CharacterRoleTraits,
            Self::PresentationMovement => T::DivisionMovement,
            Self::PresentationSpeedTerms => T::MovementStateTerms,
            Self::DivisionModelInputs => T::DivisionModelInputs,
        }
    }
}

impl Role {
    pub fn priority(self) -> u8 {
        match self {
            Self::DivisionInputs
            | Self::DivisionInputParts
            | Self::Divisions
            | Self::PresentationPrepare
            | Self::PresentationPreparation
            | Self::PresentationSettle
            | Self::PresentationStatus
            | Self::Presentation
            | Self::OrderRoots
            | Self::Orders
            | Self::OrdersPart
            | Self::OrdersStitch
            | Self::Naval
            | Self::ChildFronts
            | Self::ArmyIcons
            | Self::AirWing
            | Self::AirMissions
            | Self::Characters
            | Self::ShipModels
            | Self::Late
            | Self::Variants
            | Self::FinalInputs => 0,
            Self::RaidState
            | Self::RaidStateInputs
            | Self::PresentationSpeedTerms
            | Self::GatedProjects
            | Self::ProjectsRestPrimary
            | Self::ProjectsRestAux
            | Self::ProjectsRestPaths
            | Self::AirBases
            | Self::ShipModelDetails
            | Self::OrdinaryNavies
            | Self::ScoresRails
            | Self::Rails
            | Self::Templates
            | Self::IntelResidual
            | Self::IntelResidualAux
            | Self::Countries
            | Self::OrdinaryCombat
            | Self::OrdinaryTaskForces
            | Self::Governance
            | Self::Buildings
            | Self::OrdinaryMap
            | Self::MapCombats
            | Self::Capitulation
            | Self::Relations
            | Self::AdvisorRoles
            | Self::CountryLeaderRoles
            | Self::CharacterTraits
            | Self::CharacterPortraits => 1,
            Self::ObserverBoundary
            | Self::Intel
            | Self::World
            | Self::Economy
            | Self::PresentationMovement
            | Self::Radar
            | Self::Players
            | Self::AirStats
            | Self::Supremacy
            | Self::Sunk
            | Self::AirSupply
            | Self::ScriptVariables
            | Self::Slow
            | Self::Stability
            | Self::WarOverview
            | Self::IndustrialOrganisations
            | Self::ProductionDaily
            | Self::ProductionModifierParts
            | Self::Gated
            | Self::Projects
            | Self::CharacterInputs
            | Self::Ordinary
            | Self::OrdinaryNaval
            | Self::Provinces
            | Self::Research
            | Self::DivisionModelInputs
            | Self::Session => 2,
            Self::States
            | Self::Wars
            | Self::ProvinceUnitOrder
            | Self::SupplyTrains
            | Self::Fronts
            | Self::Weather => 3,
            Self::Factions
            | Self::Lexical
            | Self::RaidLaunch
            | Self::RaidTargets
            | Self::AirRelations
            | Self::AirHistories => 4,
        }
    }
}

pub const ROLE_COUNT: usize = 89;
pub const ROLES: [Role; ROLE_COUNT] = [
    Role::Characters,
    Role::CountryLeaderRoles,
    Role::AdvisorRoles,
    Role::CharacterTraits,
    Role::Relations,
    Role::AirWing,
    Role::AirStats,
    Role::AirBases,
    Role::Naval,
    Role::Supremacy,
    Role::Sunk,
    Role::Capitulation,
    Role::Players,
    Role::Projects,
    Role::Lexical,
    Role::Session,
    Role::Weather,
    Role::Radar,
    Role::MapCombats,
    Role::ObserverBoundary,
    Role::Divisions,
    Role::DivisionInputParts,
    Role::World,
    Role::Economy,
    Role::Governance,
    Role::Variants,
    Role::Research,
    Role::Intel,
    Role::Presentation,
    Role::FinalInputs,
    Role::IntelResidual,
    Role::SupplyTrains,
    Role::CharacterPortraits,
    Role::ProvinceUnitOrder,
    Role::Ordinary,
    Role::OrdinaryCombat,
    Role::OrdinaryNaval,
    Role::OrdinaryMap,
    Role::OrdinaryTaskForces,
    Role::OrdinaryNavies,
    Role::Slow,
    Role::Stability,
    Role::WarOverview,
    Role::IndustrialOrganisations,
    Role::ProductionDaily,
    Role::ProductionModifierParts,
    Role::RaidState,
    Role::RaidLaunch,
    Role::RaidTargets,
    Role::AirRelations,
    Role::AirMissions,
    Role::AirHistories,
    Role::Late,
    Role::ScoresRails,
    Role::ArmyIcons,
    Role::Orders,
    Role::OrdersPart,
    Role::OrdersStitch,
    Role::ChildFronts,
    Role::Fronts,
    Role::Gated,
    Role::ShipModels,
    Role::ShipModelDetails,
    Role::Provinces,
    Role::Factions,
    Role::Countries,
    Role::Wars,
    Role::States,
    Role::Templates,
    Role::CharacterInputs,
    Role::PresentationMovement,
    Role::DivisionInputs,
    Role::OrderRoots,
    Role::PresentationSpeedTerms,
    Role::PresentationPrepare,
    Role::PresentationStatus,
    Role::PresentationPreparation,
    Role::PresentationSettle,
    Role::DivisionModelInputs,
    Role::GatedProjects,
    Role::ProjectsRestPrimary,
    Role::ProjectsRestAux,
    Role::ProjectsRestPaths,
    Role::Buildings,
    Role::RaidStateInputs,
    Role::Rails,
    Role::IntelResidualAux,
    Role::AirSupply,
    Role::ScriptVariables,
];
pub const MAX_JOBS: usize = 235;
