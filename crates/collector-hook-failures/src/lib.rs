use collector_primitives::build_gate::{self, Piece};
use engine_executable::availability::{Verified, Withheld};
use replay_data::rows::TableId;
use replay_frame::{frame::FrameInput, frame::TableView};
use serde::Serialize;
#[derive(Clone, Debug)]
pub struct Patch {
    pub family: &'static str,
    pub name: String,
    pub rva: engine_sites::Rva,
    pub len: Option<usize>,
}

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

macro_rules! families {
    ($($family:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub enum Family { $($family),* }
        impl Family {
            pub const ALL: [Self; [$(stringify!($family)),*].len()] = [$(Self::$family),*];
            pub const fn name(self) -> &'static str {
                match self { $(Self::$family => $name),* }
            }
        }
    };
}
families! {
    Hourly => "hourly",
    EquipmentDestruction => "equipment_destruction",
    Land => "land_events",
    Portrait => "portrait_changes",
    Opinion => "opinion_changes",
    Construction => "construction_ic",
    LineOutput => "line_output",
    CombatLogPrune => "combat_log_prune",
    Advisor => "advisor_changes",
    Relation => "relation_changes",
    History => "history_changes",
    Flags => "flag_changes",
    Buildings => "building_changes",
    AdvisorPools => "advisor_pool_changes",
    PartyLeaders => "party_leader_changes",
    Variables => "variable_changes",
    Parties => "party_changes",
    Wings => "wing_changes",
    Roster => "roster_changes",
    PoliticalStatus => "political_status_changes",
    Missions => "mission_changes",
    Rails => "rail_changes",
    Territory => "territory_changes",
    ArmyStats => "army_stat_changes",
    CountryInputs => "country_input_changes",
    Raid => "raid_changes",
    NavalTrees => "naval_tree_changes",
    UnitLeaders => "unit_leader_changes",
    Census => "census_changes",
    OrdersGroups => "orders_group_changes",
    OrderMembership => "order_membership_changes",
    WorldInputs => "world_input_changes",
    AirRecords => "air_record_changes",
    LocalSubunits => "local_subunit_changes",
    Templates => "template_changes",
    ShipModels => "ship_model_changes",
    ProvinceUnits => "province_unit_changes",
    ProvinceCombats => "province_combat_changes",
    CombatEngagementRefreshes => "combat_engagement_refreshes",
    AirBaseCarry => "air_base_carry_changes",
    Experience => "experience_gains",
    PlayerCommands => "player_commands",
    Fuel => "fuel_awards",
    OperationOutcomes => "operation_outcomes",
    Politics => "politics_changes",
    Logistics => "logistics_balance",
    Peace => "peace_settlements",
    NavalResultSources => "naval_result_sources",
    AirLosses => "air_losses",
    BuildingBombing => "building_bombing",
    NuclearStrikes => "nuclear_strikes",
    MapEvents => "map_events",
    EventFeed => "event_feed_observer",
    Propaganda => "propaganda_changes",
    IntelSources => "intel_sources",
    NetworkAuxiliary => "network_auxiliary",
    NavalManagement => "naval_management_changes",
    WarParticipation => "war_participation",
    Designs => "design_changes",
    FactionGoals => "faction_goals",
    FactionCompanions => "faction_companions",
    Market => "market_changes",
    CommandBatch => "command_batch",
    WorldLifetime => "world_lifetime",
}

impl Family {
    pub fn owns_patch_family(self, group: &str) -> bool {
        self.name() == group
            || matches!(
                (self, group),
                (
                    Self::Missions,
                    "radar_symbols"
                        | "ground_crews"
                        | "air_detection_overrides"
                        | "naval_map_changes"
                        | "fleet_region_changes"
                ) | (Self::ArmyStats, "division_panel_changes")
                    | (
                        Self::Designs,
                        "design_source_changes"
                            | "design_organisation_liveness"
                            | "design_template_changes"
                    )
                    | (Self::Wings, "wing_preferences")
                    | (Self::CountryInputs, "wargoal_changes")
                    | (Self::NavalTrees, "occupation_changes")
                    | (Self::CombatLogPrune, "combat_log_history")
            )
    }
    fn piece(self) -> Piece {
        match self {
            Self::Hourly => Piece::new("hourly sampling"),
            _ => Piece::new(self.name()),
        }
    }
    fn verified(self) -> Verified<()> {
        self.piece().verified()?;
        if self == Self::PlayerCommands {
            match engine_executable::executable_tables::current() {
                Some(tables) => tables
                    .commands
                    .as_ref()
                    .map(drop)
                    .map_err(|withheld| withheld.requires(self.name()))?,
                None => {
                    return Err(Withheld::new(
                        self.name(),
                        "executable tables are not loaded",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn essential(self) -> Option<&'static str> {
        match self {
            Self::Hourly => Some("supplies every frame's settled hourly boundary"),
            Self::WorldLifetime => {
                Some("invalidates every retained native observer across world loads")
            }
            _ => None,
        }
    }
    pub fn tables(self) -> &'static [TableId] {
        use TableId::*;
        match self {
            Self::EquipmentDestruction => &[
                EquipmentDestruction,
                DivisionEquipmentDeletions,
                VariantDefinitions,
                VariantModules,
                VariantStats,
                VariantUpgrades,
                AvailableEquipmentVariants,
            ],
            Self::Land => &[LandCombatResults, DestroyedDivisions, LandEventCoverage],
            Self::Portrait => &[CharacterPortraits, LeaderPortraits, LeaderPortraitPaths],
            Self::Opinion => &[
                RelationOpinions,
                RelationTuples,
                WarOwnedRelations,
                WarRelationGoals,
                WarOverviewCountries,
                WarNameRules,
            ],
            Self::Construction => &[ConstructionLineOutput],
            Self::LineOutput => &[ProductionLineOutput, RocketProductionOutput],
            Self::CombatLogPrune => &[
                CombatLogEquipment,
                CombatLogState,
                CombatLogGroups,
                CombatLogMonths,
                CombatLogManpower,
                CombatLogReferences,
                CombatLogBattles,
                CombatLogBattleSides,
                CombatLogBattleCountries,
                CombatLogBattleEquipment,
                CombatLogBattleModifiers,
                CombatLogTemplates,
                CombatLogEnemyEquipment,
            ],
            Self::Advisor => &[
                Characters,
                CountryLeaderRoles,
                AdvisorRoles,
                UnitLeaderTraits,
                CharacterRoleTraits,
                CharacterOperatives,
                AvailableAdvisors,
                PartyLeaders,
            ],
            Self::Relation => &[AirCountryRelations, WarRelations, RelationLinks],
            Self::History => &[AirNavalPatrolHistories],
            Self::Flags => &[ScriptFlags],
            Self::Buildings => &[StateBuildings, ProvinceBuildings],
            Self::AdvisorPools => &[AvailableAdvisors, HiredAdvisors, AdvisorTokens],
            Self::PartyLeaders => &[PartyLeaders],
            Self::Variables => &[ScriptVariables, FactionVariables, Projects, ProjectPrograms],
            Self::Parties => &[PoliticalParties, ActiveIdeas],
            Self::Wings => &[WingEquipment],
            Self::Roster => &[
                CountryCharacters,
                CountryRetiredCharacters,
                Scientists,
                ScientistSkills,
                Characters,
                CountryLeaderRoles,
                AdvisorRoles,
                UnitLeaderTraits,
                CharacterRoleTraits,
                CharacterOperatives,
                ScientistDaily,
            ],
            Self::PoliticalStatus => &[Politics],
            Self::Missions => &[
                AirRegionalMissions,
                AirTransferInputs,
                AirGroundCrews,
                RadarCoverage,
                NavalRegionalMines,
                NavalAccidentReports,
                FleetCounterInputs,
            ],
            Self::Rails => &[RailCooldowns, RailEdges],
            Self::Territory => &[States, Provinces],
            Self::ArmyStats => &[
                DivisionLandInputs,
                DivisionStatus,
                DivisionPanelSources,
                DivisionPanelStats,
                DivisionHistory,
            ],
            Self::CountryInputs => &[
                CountryStabilityInputs,
                CountryStabilityLines,
                ResearchInputs,
                ResearchAppliedBonuses,
                ResearchModifiers,
                ResearchTechnologies,
                TechnologyDefinitions,
                Focus,
                CompletedFocuses,
                Projects,
                ProjectHistory,
                CountryAppearance,
                CountryCounterInputs,
                CountryVisibility,
                CountryWargoals,
                WargoalTargets,
                WargoalJustifications,
                StateResistance,
                CountryUnitSpeedEntries,
                FactionResearch,
            ],
            Self::Raid => &[Raids, RaidSpeedInputs, RaidAirScores],
            Self::NavalTrees => &[
                NavalModifierTreeSamples,
                NavalModifierTrees,
                NavalMissionCountries,
                NavalMissionCommanders,
                DivisionRetreatBans,
                UnitLeaderTraits,
                ShipModels,
                StateResistance,
            ],
            Self::UnitLeaders => &[UnitLeaders, UnitLeaderTraitExperience],
            Self::Census => &[
                Characters,
                CountryCharacters,
                Scientists,
                ScientistSkills,
                UnitLeaders,
                CharacterRoleTraits,
            ],
            Self::OrdersGroups => &[DivisionStatus, DivisionPanelSources, DivisionRetreatBans],
            Self::OrderMembership => &[],
            Self::WorldInputs => &[
                ProvinceNames,
                NavalRegionAccess,
                WorldTension,
                ThreatSources,
                ThreatCountryAggregates,
                DynamicModifiers,
                IndustrialOrganisations,
                IndustrialOrganisationDetails,
                IndustrialOrganisationPolicies,
                IndustrialOrganisationHistory,
                DoctrineFolders,
                DoctrineTracks,
                DoctrineLeaderMastery,
                DoctrineCostReductions,
                DoctrineMasteryGrants,
                DoctrineEquipmentBonuses,
                DoctrineCountries,
            ],
            Self::AirRecords => &[AirRegionCombat],
            Self::LocalSubunits => &[CountryLocalSubunits],
            Self::Templates => &[
                DivisionTemplates,
                DivisionTemplateIcons,
                CountryTemplateState,
                OrdersGroupCounterInputs,
            ],
            Self::ShipModels => &[ShipModels],
            Self::ProvinceUnits => &[ProvinceUnitOrder],
            Self::ProvinceCombats => &[NavalProvinceCombats],
            Self::CombatEngagementRefreshes => &[CombatEngagementRefreshes],
            Self::AirBaseCarry => &[AirBases, AirBaseRegistrations],
            Self::Experience => &[CountryExperienceGains],
            Self::PlayerCommands => &[],
            Self::Fuel => &[CountryFuel],
            Self::OperationOutcomes => &[OperationCompletions],
            Self::Politics => &[
                CountryPowerDaily,
                CountryPowerChanges,
                CountryPowerDailyCosts,
                GovernmentChanges,
                PoliticsCaptureCoverage,
            ],
            Self::Logistics => &[LogisticsEquipment],
            Self::Peace => &[PeaceSettlements],
            Self::NavalResultSources => &[NavalResultSources],
            Self::AirLosses => &[AirLossEvents, AirDailyRoster],
            Self::BuildingBombing => &[BuildingBombingDamage],
            Self::NuclearStrikes => &[NuclearStrikes],
            Self::MapEvents => &[MapEvents],
            Self::EventFeed => &[EventFeedHistory],
            Self::Propaganda => &[PropagandaEfforts],
            Self::IntelSources => &[IntelContributions, IntelSources],
            Self::NetworkAuxiliary => &[IntelNetworkAuxiliaries],
            Self::NavalManagement => &[
                NavalRepairCountries,
                NavalRepairFleets,
                NavalRepairTaskForces,
                NavalTaskForceComposition,
                NavalRepairBases,
                NavalRepairQueue,
                NavalRefitShips,
                NavalRefitProgress,
                NavalRepairChildren,
            ],
            Self::WarParticipation => &[
                WarParticipation,
                WarCapturedProvinces,
                WarLendLeaseHistories,
            ],
            Self::Designs => &[
                VariantDefinitions,
                VariantModules,
                VariantStats,
                VariantUpgrades,
                VariantChanges,
                VariantBonusSnapshots,
                DesignCountryModifiers,
                DesignCountryTemplates,
                DesignObserverContext,
                DesignOrganisationLiveness,
            ],
            Self::FactionGoals => &[FactionGoalStatus, FactionGoalEntries],
            Self::FactionCompanions => &[
                FactionRules,
                FactionPrograms,
                FactionManpower,
                FactionUnlocks,
                FactionAccounts,
                FactionProjection,
                FactionResources,
                FactionAdvisorModifiers,
                FactionMinimumUpgrades,
                FactionDoctrine,
            ],
            Self::Market => &[
                CountryMarket,
                MarketContractTerms,
                LendLeaseAgreements,
                MarketEvents,
            ],
            Self::Hourly | Self::CommandBatch | Self::WorldLifetime => &[],
        }
    }
    fn depends_on(self, source: Self) -> bool {
        use Family::*;
        matches!(
            (self, source),
            (
                CountryInputs,
                Advisor
                    | Relation
                    | PartyLeaders
                    | Roster
                    | PoliticalStatus
                    | Territory
                    | Raid
                    | Opinion
                    | AdvisorPools
                    | Politics
            ) | (Census, Advisor | Roster)
                | (Roster, Advisor | AdvisorPools)
                | (Opinion, CountryInputs)
                | (Wings, EquipmentDestruction)
                | (Territory, Buildings)
                | (PoliticalStatus, Politics)
                | (Rails | Raid, Buildings)
                | (NavalTrees, CountryInputs | Politics)
                | (UnitLeaders, NavalTrees)
                | (OrdersGroups, Templates)
                | (AdvisorPools, CountryInputs)
                | (AirBaseCarry, Raid | ShipModels | Buildings)
                | (
                    NuclearStrikes,
                    EquipmentDestruction | Buildings | CountryInputs
                )
                | (EventFeed, Politics | Territory)
                | (Designs, WorldInputs | CountryInputs | Buildings)
                | (
                    ArmyStats,
                    Designs | UnitLeaders | CountryInputs | LocalSubunits
                )
                | (Market, CountryInputs | Variables)
                | (WarParticipation, CountryInputs | Opinion | Relation)
                | (Missions, WorldInputs | Raid)
        )
    }
    pub fn preserves_capture(self, table: TableId) -> bool {
        if matches!(
            self,
            Self::Portrait
                | Self::CombatLogPrune
                | Self::Opinion
                | Self::Advisor
                | Self::Relation
                | Self::History
                | Self::Flags
                | Self::Buildings
                | Self::AdvisorPools
                | Self::PartyLeaders
                | Self::Variables
                | Self::Parties
                | Self::Wings
                | Self::Roster
                | Self::PoliticalStatus
                | Self::Rails
                | Self::Territory
                | Self::ArmyStats
                | Self::CountryInputs
                | Self::Missions
                | Self::Raid
                | Self::NavalTrees
                | Self::UnitLeaders
                | Self::Census
                | Self::OrdersGroups
                | Self::OrderMembership
                | Self::AirRecords
                | Self::LocalSubunits
                | Self::ShipModels
                | Self::ProvinceUnits
                | Self::ProvinceCombats
                | Self::AirBaseCarry
                | Self::Fuel
                | Self::Propaganda
        ) {
            return true;
        }
        match self {
            Self::Land => table == TableId::LandEventCoverage,
            Self::Politics => table == TableId::PoliticsCaptureCoverage,
            Self::Designs => matches!(
                table,
                TableId::DesignObserverContext
                    | TableId::VariantDefinitions
                    | TableId::VariantModules
                    | TableId::VariantStats
                    | TableId::VariantUpgrades
                    | TableId::VariantBonusSnapshots
            ),
            Self::EquipmentDestruction => matches!(
                table,
                TableId::VariantDefinitions
                    | TableId::VariantModules
                    | TableId::VariantStats
                    | TableId::VariantUpgrades
                    | TableId::AvailableEquipmentVariants
            ),
            Self::AirLosses => table == TableId::AirDailyRoster,
            Self::Market => matches!(
                table,
                TableId::CountryMarket
                    | TableId::MarketContractTerms
                    | TableId::LendLeaseAgreements
            ),
            Self::WorldInputs => matches!(
                table,
                TableId::ProvinceNames
                    | TableId::NavalRegionAccess
                    | TableId::WorldTension
                    | TableId::ThreatSources
                    | TableId::ThreatCountryAggregates
                    | TableId::DynamicModifiers
                    | TableId::IndustrialOrganisations
                    | TableId::IndustrialOrganisationDetails
                    | TableId::IndustrialOrganisationPolicies
                    | TableId::IndustrialOrganisationHistory
            ),
            _ => false,
        }
    }
}

#[derive(Serialize)]
struct Failure {
    family: &'static str,
    site: String,
    stage: &'static str,
    cause: String,
    refusal: String,
}
static FAILED: [AtomicBool; Family::ALL.len()] =
    [const { AtomicBool::new(false) }; Family::ALL.len()];
static ANY: AtomicBool = AtomicBool::new(false);
static WORKERS_STARTED: AtomicBool = AtomicBool::new(false);
pub fn workers_started() {
    WORKERS_STARTED.store(true, Ordering::Release);
}
static FAILURES: Mutex<Vec<Failure>> = Mutex::new(Vec::new());
struct AttachContext {
    write_status: fn(&Path, &[u8]) -> io::Result<()>,
    directory: PathBuf,
    session: u64,
    process_start: u64,
}
static CONTEXT: OnceLock<AttachContext> = OnceLock::new();
pub fn configure(
    directory: &Path,
    session: u64,
    process_start: u64,
    write_status: fn(&Path, &[u8]) -> io::Result<()>,
) {
    let _ = CONTEXT.set(AttachContext {
        write_status,
        directory: directory.to_path_buf(),
        session,
        process_start,
    });
}
fn publish_attach_status(family: Family) -> io::Result<()> {
    if WORKERS_STARTED.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(context) = CONTEXT.get() else {
        return Ok(());
    };
    let status = collector_status::status::status!({
        "state": if family.essential().is_some() { "failed" } else { "attaching" },
        "session": context.session,
        "process_start_time": context.process_start,
        "failed_hook_families": status(),
        "build": build_gate::status(),
        "installation_error": family.essential(),
    });
    let published = collector_status::status::serialize(&status)
        .and_then(|bytes| (context.write_status)(&context.directory.join("status.json"), &bytes));
    if let Err(error) = published {
        log(
            &context.directory,
            format_args!("hook failure status not published: {error}"),
        )?;
    }
    Ok(())
}

pub fn log(directory: &Path, message: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("recorder.log"))?;
    writeln!(file, "{message}")
}
pub fn failed(family: Family) -> bool {
    FAILED[family as usize].load(Ordering::Acquire)
}
pub fn refuse(
    patches: fn() -> Vec<Patch>,
    family: Family,
    stage: &'static str,
    cause: &str,
) -> Result<(), &'static str> {
    if failed(family) {
        return if family.essential().is_some() {
            Err("essential hook family not installed")
        } else {
            Ok(())
        };
    }
    refuse_at(patches, family, None, stage, cause)
}
pub fn admit(
    patches: fn() -> Vec<Patch>,
    family: Family,
    installation_message: fn(String) -> &'static str,
) -> Result<bool, &'static str> {
    let Err(withheld) = family.verified() else {
        return Ok(true);
    };
    build_gate::report(&withheld).map_err(|_| "withheld piece could not be recorded")?;
    let cause = withheld.to_string();
    match refuse(patches, family, "build", &cause) {
        Ok(()) => Ok(false),
        Err(_) if family.essential().is_some() => Err(installation_message(cause)),
        Err(error) => Err(error),
    }
}
pub fn admit_essential(
    patches: fn() -> Vec<Patch>,
    installation_message: fn(String) -> &'static str,
) -> Result<(), &'static str> {
    for family in Family::ALL
        .into_iter()
        .filter(|family| family.essential().is_some())
    {
        admit(patches, family, installation_message)?;
    }
    Ok(())
}
fn refuse_at(
    patches: fn() -> Vec<Patch>,
    family: Family,
    exact_site: Option<&str>,
    stage: &'static str,
    cause: &str,
) -> Result<(), &'static str> {
    if failed(family) {
        return if family.essential().is_some() {
            Err("essential hook family not installed")
        } else {
            Ok(())
        };
    }
    let result = (|| -> io::Result<()> {
        let site = patches()
            .into_iter()
            .filter(|patch| family.owns_patch_family(patch.family))
            .map(|patch| patch.name)
            .collect::<Vec<_>>()
            .join(", ");
        if site.is_empty() {
            return Err(io::Error::other(
                "runtime hook family has no registered native sites",
            ));
        }
        let site = exact_site.map_or(site, str::to_owned);
        let refusal = replay_data::codec::try_format(format_args!(
            "hook family {} not installed: {cause}",
            family.name()
        ))?;
        if let Some(context) = CONTEXT.get() {
            log(
                &context.directory,
                format_args!("{refusal}; site={site}; stage={stage}"),
            )?;
        }
        let mut failures = FAILURES.lock().unwrap_or_else(PoisonError::into_inner);
        engine_allocation::reserve(&mut failures, 1)?;
        failures.push(Failure {
            family: family.name(),
            site,
            stage,
            cause: engine_allocation::string(cause)?,
            refusal,
        });
        FAILED[family as usize].store(true, Ordering::Release);
        ANY.store(true, Ordering::Release);
        Ok(())
    })();
    result.map_err(|_| "hook failure diagnostic could not be recorded")?;
    publish_attach_status(family).map_err(|_| "hook failure status could not be recorded")?;
    if family.essential().is_some() {
        return Err("essential hook family not installed");
    }
    for dependent in Family::ALL {
        if dependent.depends_on(family) && !failed(dependent) {
            let cause = format!("forwarder {} not installed: {cause}", family.name());
            refuse(patches, dependent, "forwarder", &cause)?;
        }
    }
    Ok(())
}
pub fn status() -> impl Serialize {
    struct Status;
    impl Serialize for Status {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            FAILURES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .serialize(serializer)
        }
    }
    Status
}
pub fn apply(source: &mut FrameInput) -> io::Result<()> {
    if let Some(error) = fatal_installation_error() {
        return Err(io::Error::other(error));
    }
    if !ANY.load(Ordering::Acquire) {
        return Ok(());
    }
    let failures = FAILURES.lock().unwrap_or_else(PoisonError::into_inner);
    for family in Family::ALL
        .into_iter()
        .filter(|family| failed(*family) && *family != Family::Templates)
    {
        let failure = failures
            .iter()
            .find(|failure| failure.family == family.name())
            .unwrap();
        for table in family.tables() {
            if family.preserves_capture(*table) {
                continue;
            }
            source.tables[table.index()] = TableView::Unavailable {
                reason: engine_allocation::string(&failure.refusal)?,
            }
            .into();
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: collector_hooks::hook::installation::Kind,
    site: String,
    cause: String,
}
static PENDING: Mutex<[Option<Diagnostic>; Family::ALL.len()]> =
    Mutex::new([const { None }; Family::ALL.len()]);
static TERMINAL: Mutex<Option<&'static str>> = Mutex::new(None);
pub fn fatal_installation_error() -> Option<&'static str> {
    *TERMINAL.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn shared_rollback_failed(
    patches: fn() -> Vec<Patch>,
    name: Option<&str>,
    failure: &collector_hooks::hook::installation::Failure,
    installation_message: fn(String) -> &'static str,
) -> Result<(), &'static str> {
    let family = Family::ALL
        .into_iter()
        .find(|family| Some(family.name()) == name)
        .ok_or("shared rollback has no registered runtime family")?;
    let diagnostic = audit_report(
        patches,
        family,
        0,
        collector_hooks::hook::installation::Report {
            failure: Some(collector_hooks::hook::installation::Failure {
                site: None,
                ..failure.clone()
            }),
            ..Default::default()
        },
    )?
    .ok_or("shared rollback has no failure diagnostic")?;
    refuse_diagnostic(patches, family, diagnostic, installation_message)
}
pub fn audit_report(
    patches: fn() -> Vec<Patch>,
    family: Family,
    base: usize,
    mut report: collector_hooks::hook::installation::Report,
) -> Result<Option<Diagnostic>, &'static str> {
    use collector_hooks::hook::installation::{Failure as ObservedFailure, Kind};
    let patches = patches();
    let declared: Vec<_> = patches
        .iter()
        .filter(|patch| family.owns_patch_family(patch.family))
        .collect();
    if declared.is_empty() {
        return Ok(Some(Diagnostic {
            kind: Kind::Registration,
            site: family.name().into(),
            cause: "runtime family has no native site registration".into(),
        }));
    }
    if report.sites.iter().any(|site| {
        let key = engine_sites::reference(base, *site);
        !declared.iter().any(|patch| key == Some(patch.rva))
    }) {
        report.failure = Some(ObservedFailure {
            kind: Kind::Registration,
            site: None,
            cause: "runtime attempted a hook site absent from its family inventory".into(),
        });
    }
    let Some(failure) = report.failure else {
        return Ok(None);
    };
    let site = if let Some(at) = failure.site {
        let key = engine_sites::reference(base, at);
        patches
            .iter()
            .find(|patch| key == Some(patch.rva))
            .map(|patch| patch.name.clone())
            .ok_or("installation failure site is not registered")?
    } else {
        declared
            .iter()
            .map(|patch| patch.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(Some(Diagnostic {
        kind: failure.kind,
        site,
        cause: failure.cause.into_owned(),
    }))
}
pub fn remember_failure(
    family: Family,
    diagnostic: Diagnostic,
    installation_message: fn(String) -> &'static str,
) {
    if diagnostic.kind.fatal() {
        let mut terminal = TERMINAL.lock().unwrap_or_else(PoisonError::into_inner);
        terminal.get_or_insert_with(|| {
            installation_message(format!(
                "{} {} at {}: {}",
                family.name(),
                diagnostic.kind.name(),
                diagnostic.site,
                diagnostic.cause
            ))
        });
    }
    let mut pending = PENDING.lock().unwrap_or_else(PoisonError::into_inner);
    if !pending[family as usize]
        .as_ref()
        .is_some_and(|old| old.kind.fatal())
    {
        pending[family as usize] = Some(diagnostic);
    }
}
pub fn refuse_diagnostic(
    patches: fn() -> Vec<Patch>,
    family: Family,
    diagnostic: Diagnostic,
    installation_message: fn(String) -> &'static str,
) -> Result<(), &'static str> {
    remember_failure(family, diagnostic.clone(), installation_message);
    refuse_at(
        patches,
        family,
        Some(&diagnostic.site),
        diagnostic.kind.name(),
        &diagnostic.cause,
    )?;
    if let Some(error) = fatal_installation_error() {
        return Err(error);
    }
    Ok(())
}
pub fn install_failed(
    patches: fn() -> Vec<Patch>,
    family: Family,
    cause: &'static str,
    installation_message: fn(String) -> &'static str,
) -> Result<(), &'static str> {
    let diagnostic = PENDING.lock().unwrap_or_else(PoisonError::into_inner)[family as usize].take();
    if let Some(diagnostic) = diagnostic {
        return refuse_diagnostic(patches, family, diagnostic, installation_message);
    }
    if let Some(error) = fatal_installation_error() {
        return Err(error);
    }
    if failed(family) {
        return Err(cause);
    }
    refuse(patches, family, "publication", cause)
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

#[cfg(feature = "test-support")]
pub fn reset_failures_for_test() {
    for flag in &FAILED {
        flag.store(false, Ordering::Release);
    }
    ANY.store(false, Ordering::Release);
    FAILURES.lock().unwrap().clear();
    *PENDING.lock().unwrap() = std::array::from_fn(|_| None);
    *TERMINAL.lock().unwrap() = None;
}
