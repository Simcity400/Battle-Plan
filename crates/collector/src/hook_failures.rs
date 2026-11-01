//! Session-scoped hook refusals. No capture callback formats diagnostics.
use replay_data::rows::TableId;
use replay_frame::{frame::FrameInput, frame::TableView};
use serde::Serialize;
use std::{
    io::{self, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) trait RegisteredFamily {
    const FAMILY: Family;
}
macro_rules! runtime_families {
    ($($family:ident => ($name:literal, $ty:ty)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub(crate) enum Family { $($family),* }
        impl Family {
            pub(crate) const ALL: [Self; [$(stringify!($family)),*].len()] = [$(Self::$family),*];
            pub(crate) fn name(self) -> &'static str { match self { $(Self::$family => $name),* } }
            #[cfg(test)]
            pub(crate) fn runtime_type(self) -> &'static str { match self { $(Self::$family => stringify!($ty)),* } }
        }
        $(impl RegisteredFamily for $ty { const FAMILY: Family = Family::$family; })*
    };
}
runtime_families! {
    Hourly => ("hourly", crate::hook::Hook),
    EquipmentDestruction => ("equipment_destruction", crate::equipment_destruction::Hooks),
    Land => ("land_events", crate::land_events::Hooks),
    Portrait => ("portrait_changes", crate::portrait_changes::Hooks),
    Opinion => ("opinion_changes", crate::opinion_changes::Hooks),
    Construction => ("construction_ic", crate::construction_ic::Hooks),
    LineOutput => ("line_output", crate::line_output::Hooks),
    CombatLogPrune => ("combat_log_prune", crate::combat_log_prune::Hooks),
    Advisor => ("advisor_changes", crate::advisor_changes::Hooks),
    Relation => ("relation_changes", crate::relation_changes::Hooks),
    History => ("history_changes", crate::history_changes::Hooks),
    Flags => ("flag_changes", crate::flag_changes::Hooks),
    Buildings => ("building_changes", crate::building_changes::Hooks),
    AdvisorPools => ("advisor_pool_changes", crate::advisor_pool_changes::Hooks),
    PartyLeaders => ("party_leader_changes", crate::party_leader_changes::Hooks),
    Variables => ("variable_changes", crate::variable_changes::Hooks),
    Parties => ("party_changes", crate::party_changes::Hooks),
    Wings => ("wing_changes", crate::wing_changes::Hooks),
    Roster => ("roster_changes", crate::roster_changes::Hooks),
    PoliticalStatus => ("political_status_changes", crate::political_status_changes::Hooks),
    Missions => ("mission_changes", crate::mission_changes::Hooks),
    Rails => ("rail_changes", crate::rail_changes::Hooks),
    Territory => ("territory_changes", crate::territory_changes::Hooks),
    ArmyStats => ("army_stat_changes", crate::army_stat_changes::Hooks),
    CountryInputs => ("country_input_changes", crate::country_input_changes::Hooks),
    Raid => ("raid_changes", crate::raid_changes::Hooks),
    NavalTrees => ("naval_tree_changes", crate::naval_tree_changes::Hooks),
    UnitLeaders => ("unit_leader_changes", crate::unit_leader_changes::Hooks),
    Census => ("census_changes", crate::census_changes::Hooks),
    OrdersGroups => ("orders_group_changes", crate::orders_group_changes::Hooks),
    WorldInputs => ("world_input_changes", crate::world_input_changes::Hooks),
    AirRecords => ("air_record_changes", crate::air_record_changes::Hooks),
    LocalSubunits => ("local_subunit_changes", crate::local_subunit_changes::Hooks),
    Templates => ("template_changes", crate::template_changes::Hooks),
    ShipModels => ("ship_model_changes", crate::ship_model_changes::Hooks),
    ProvinceUnits => ("province_unit_changes", crate::province_unit_changes::Hooks),
    ProvinceCombats => ("province_combat_changes", crate::province_combat_changes::Hooks),
    AirBaseCarry => ("air_base_carry_changes", crate::air_base_carry_changes::Hooks),
    Experience => ("experience_gains", crate::experience_gains::Hooks),
    PlayerCommands => ("player_commands", crate::player_commands::Hooks),
    Fuel => ("fuel_awards", crate::fuel_awards::Hooks),
    OperationOutcomes => ("operation_outcomes", crate::operation_outcomes::Hooks),
    Politics => ("politics_changes", crate::politics_changes::Hooks),
    Logistics => ("logistics_balance", crate::logistics_balance::Hooks),
    Peace => ("peace_settlements", crate::peace_settlements::Hooks),
    NavalResultSources => ("naval_result_sources", crate::naval_result_sources::Hooks),
    AirLosses => ("air_losses", crate::air_losses::Hooks),
    BuildingBombing => ("building_bombing", crate::building_bombing::Hooks),
    NuclearStrikes => ("nuclear_strikes", crate::nuclear_strikes::Hooks),
    MapEvents => ("map_events", crate::map_events::hooks::Hooks),
    EventFeed => ("event_feed_observer", crate::event_feed_observer::Hooks),
    Propaganda => ("propaganda_changes", crate::propaganda_changes::Hooks),
    IntelSources => ("intel_sources", crate::intel_source_changes::Hooks),
    NetworkAuxiliary => ("network_auxiliary", crate::network_auxiliary::Hooks),
    NavalManagement => ("naval_management_changes", crate::naval_management_changes::Hooks),
    WarParticipation => ("war_participation", crate::war_participation::Hooks),
    Designs => ("design_changes", crate::design_changes::Hooks),
    FactionGoals => ("faction_goals", crate::faction_goal_changes::Hooks),
    FactionCompanions => ("faction_companions", crate::faction_companion_changes::Hooks),
    Market => ("market_changes", crate::market_changes::Hooks),
    // World invalidation stays published until every other family restores.
    WorldLifetime => ("world_lifetime", crate::world_lifetime::Hooks),
}

impl Family {
    pub(crate) fn owns_patch_family(self, group: &str) -> bool {
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
    pub(crate) fn essential(self) -> Option<&'static str> {
        match self {
            Self::Hourly => Some("supplies every frame's settled hourly boundary"),
            Self::WorldLifetime => {
                Some("invalidates every retained native observer across world loads")
            }
            _ => None,
        }
    }
    pub(crate) fn tables(self) -> &'static [TableId] {
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
            Self::Hourly | Self::WorldLifetime => &[],
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
    // These captures already refuse missing observation or read current sources.
    fn preserves_capture(self, table: TableId) -> bool {
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
pub(crate) fn workers_started() {
    WORKERS_STARTED.store(true, Ordering::Release);
}
static FAILURES: Mutex<Vec<Failure>> = Mutex::new(Vec::new());
struct AttachContext {
    directory: PathBuf,
    session: u64,
    process_start: u64,
}
static CONTEXT: OnceLock<AttachContext> = OnceLock::new();
pub(crate) fn configure(directory: &Path, session: u64, process_start: u64) {
    let _ = CONTEXT.set(AttachContext {
        directory: directory.to_path_buf(),
        session,
        process_start,
    });
}
fn publish_attach_status(family: Family) -> io::Result<()> {
    // Once workers exist, the spool's coalesced status writer is the sole
    // publisher. It serializes this same registry; never race its temp file.
    if WORKERS_STARTED.load(Ordering::Acquire) {
        return Ok(());
    }
    if let Some(context) = CONTEXT.get() {
        let status = crate::status::status!({
            "state": if family.essential().is_some() { "failed" } else { "attaching" },
            "session": context.session,
            "process_start_time": context.process_start,
            "failed_hook_families": status(),
            "installation_error": family.essential(),
        });
        crate::runtime::write_status(&context.directory.join("status.json"), &status)?;
    }
    Ok(())
}

pub(crate) fn log(directory: &Path, message: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("recorder.log"))?;
    writeln!(file, "{message}")
}
pub(crate) fn failed(family: Family) -> bool {
    FAILED[family as usize].load(Ordering::Acquire)
}
pub(crate) fn refuse(family: Family, stage: &'static str, cause: &str) -> Result<(), &'static str> {
    if failed(family) {
        return if family.essential().is_some() {
            Err("essential hook family not installed")
        } else {
            Ok(())
        };
    }
    refuse_at(family, None, stage, cause)
}
fn refuse_at(
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
        let site = crate::hook_inventory::patches()
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
            refuse(dependent, "forwarder", &cause)?;
        }
    }
    Ok(())
}
pub(crate) fn status() -> impl Serialize {
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
pub(crate) fn apply(source: &mut FrameInput) -> io::Result<()> {
    if let Some(error) = fatal_installation_error() {
        return Err(io::Error::other(error));
    }
    if !ANY.load(Ordering::Acquire) {
        return Ok(());
    }
    let failures = FAILURES.lock().unwrap_or_else(PoisonError::into_inner);
    // Template observation admits reuse; its absence selects fresh acquisition.
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
struct Diagnostic {
    kind: crate::hook::installation::Kind,
    site: String,
    cause: String,
}
static PENDING: Mutex<[Option<Diagnostic>; Family::ALL.len()]> =
    Mutex::new([const { None }; Family::ALL.len()]);
static TERMINAL: Mutex<Option<&'static str>> = Mutex::new(None);
pub(crate) fn fatal_installation_error() -> Option<&'static str> {
    *TERMINAL.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn shared_rollback_failed(
    name: Option<&str>,
    failure: &crate::hook::installation::Failure,
) -> Result<(), &'static str> {
    let family = Family::ALL
        .into_iter()
        .find(|family| Some(family.name()) == name)
        .ok_or("shared rollback has no registered runtime family")?;
    // Shared publication reports an atomic family outcome. Name that family's
    // complete registered site set, as for other group publication errors.
    let diagnostic = audit_report(
        family,
        0,
        crate::hook::installation::Report {
            failure: Some(crate::hook::installation::Failure {
                site: None,
                ..failure.clone()
            }),
            ..Default::default()
        },
    )?
    .ok_or("shared rollback has no failure diagnostic")?;
    refuse_diagnostic(family, diagnostic)
}
fn audit_report(
    family: Family,
    base: usize,
    mut report: crate::hook::installation::Report,
) -> Result<Option<Diagnostic>, &'static str> {
    use crate::hook::installation::{Failure as ObservedFailure, Kind};
    let patches = crate::hook_inventory::patches();
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
        !declared
            .iter()
            .any(|patch| base.checked_add(patch.rva) == Some(*site))
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
        patches
            .iter()
            .find(|patch| base.checked_add(patch.rva) == Some(at))
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
fn remember_failure(family: Family, diagnostic: Diagnostic) {
    if diagnostic.kind.fatal() {
        let mut terminal = TERMINAL.lock().unwrap_or_else(PoisonError::into_inner);
        terminal.get_or_insert_with(|| {
            crate::runtime::installation_message(format!(
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
fn refuse_diagnostic(family: Family, diagnostic: Diagnostic) -> Result<(), &'static str> {
    remember_failure(family, diagnostic.clone());
    refuse_at(
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
pub(crate) fn install_failed(family: Family, cause: &'static str) -> Result<(), &'static str> {
    let diagnostic = PENDING.lock().unwrap_or_else(PoisonError::into_inner)[family as usize].take();
    if let Some(diagnostic) = diagnostic {
        return refuse_diagnostic(family, diagnostic);
    }
    if let Some(error) = fatal_installation_error() {
        return Err(error);
    }
    if failed(family) {
        return Err(cause);
    }
    refuse(family, "publication", cause)
}

pub(crate) struct FamilyHooks<T> {
    pub(crate) inner: Option<T>,
    family: Family,
    base: usize,
}
impl<T: RegisteredFamily> FamilyHooks<T> {
    pub(crate) fn construct(
        base: usize,
        construct: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<Self, &'static str> {
        let family = T::FAMILY;
        let (result, report) = crate::hook::installation::capture(|| {
            let result = construct();
            if let Err(cause) = &result {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    cause,
                );
            }
            result
        });
        let diagnostic = audit_report(family, base, report)?;
        let hooks = Self {
            inner: result.ok(),
            family,
            base,
        };
        if let Some(diagnostic) = diagnostic {
            refuse_diagnostic(family, diagnostic)?;
        } else if hooks.inner.is_none() {
            // Every returned Err must have been observed above, even before
            // reaching a native constructor.
            return Err("hook constructor failed without an installation diagnostic");
        }
        Ok(hooks)
    }
}
impl<T> FamilyHooks<T> {
    #[cfg(test)]
    pub(crate) fn new(
        family: Family,
        hooks: Result<T, &'static str>,
    ) -> Result<Self, &'static str> {
        let inner = match hooks {
            Ok(hooks) => Some(hooks),
            Err(cause) => {
                refuse(family, "construction", cause)?;
                None
            }
        };
        Ok(Self {
            inner,
            family,
            base: 0,
        })
    }
    fn toggle(
        &mut self,
        enabled: bool,
        mut toggle: impl FnMut(&mut T, bool) -> Result<bool, &'static str>,
    ) -> Result<bool, &'static str> {
        let requested = enabled;
        let enabled = enabled && !failed(self.family);
        let Some(hooks) = self.inner.as_mut() else {
            return Ok(true);
        };
        if !requested {
            return toggle(hooks, false);
        }
        let (result, report) = crate::hook::installation::capture(|| {
            let result = toggle(hooks, enabled);
            if requested && let Err(cause) = &result {
                crate::hook::installation::refused(
                    if enabled {
                        crate::hook::installation::Kind::Publication
                    } else {
                        crate::hook::installation::Kind::Rollback
                    },
                    None,
                    cause,
                );
            }
            result
        });
        if report.failure.is_none() && report.sites.is_empty() {
            return result;
        }
        if let Some(diagnostic) = audit_report(self.family, self.base, report)? {
            let reason = crate::runtime::installation_message(diagnostic.cause.clone());
            remember_failure(self.family, diagnostic);
            return Err(reason);
        }
        result
    }
}
macro_rules! family_hooks {
    ($($module:ident),* $(,)?) => {$ (
        impl FamilyHooks<crate::$module::Hooks> {
            pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
                self.toggle(enabled, |hooks, enabled| hooks.set_enabled(enabled).map_err(|cause|
                    if enabled {
                        crate::runtime::installation_message(cause)
                    } else {
                        "hook restoration failed; collector retained"
                    }))
            }
            pub(crate) fn executable_ranges(&self) -> impl Iterator<Item=Range<usize>> + '_ {
                self.inner.iter().flat_map(|hooks| hooks.executable_ranges().into_iter())
            }
        }
    )*};
}
family_hooks!(
    propaganda_changes,
    intel_source_changes,
    network_auxiliary,
    naval_management_changes,
    war_participation,
    design_changes,
    faction_goal_changes,
    faction_companion_changes,
    market_changes,
    equipment_destruction,
    land_events,
    portrait_changes,
    opinion_changes,
    construction_ic,
    line_output,
    combat_log_prune,
    advisor_changes,
    relation_changes,
    history_changes,
    flag_changes,
    building_changes,
    advisor_pool_changes,
    party_leader_changes,
    variable_changes,
    party_changes,
    wing_changes,
    roster_changes,
    political_status_changes,
    mission_changes,
    rail_changes,
    territory_changes,
    army_stat_changes,
    country_input_changes,
    raid_changes,
    naval_tree_changes,
    unit_leader_changes,
    census_changes,
    orders_group_changes,
    world_input_changes,
    air_record_changes,
    local_subunit_changes,
    template_changes,
    ship_model_changes,
    province_unit_changes,
    province_combat_changes,
    air_base_carry_changes,
    experience_gains,
    player_commands,
    fuel_awards,
    operation_outcomes,
    politics_changes,
    logistics_balance,
    peace_settlements,
    naval_result_sources,
    air_losses,
    building_bombing,
    nuclear_strikes,
    event_feed_observer,
    world_lifetime
);
impl FamilyHooks<crate::map_events::hooks::Hooks> {
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        self.toggle(enabled, |hooks, enabled| hooks.set_enabled(enabled))
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.inner
            .iter()
            .flat_map(|hooks| hooks.executable_ranges())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::{FrameInput, Metrics, TABLE_COUNT};

    #[test]
    fn every_runtime_family_contains_failures_and_restores_safely() {
        const CHILD: &str = "BATTLE_PLAN_HOOK_CONTAINMENT_TEST";
        const TEST: &str =
            "hook_failures::tests::every_runtime_family_contains_failures_and_restores_safely";
        // The production registry is session-scoped. Isolate its mutations.
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("hoi4-hooks-contained-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        configure(&directory, 1, 1000);
        for family in Family::ALL {
            for flag in &FAILED {
                flag.store(false, Ordering::Release);
            }
            ANY.store(false, Ordering::Release);
            FAILURES.lock().unwrap().clear();
            *PENDING.lock().unwrap() = std::array::from_fn(|_| None);
            *TERMINAL.lock().unwrap() = None;
            if family.essential().is_some() {
                assert!(
                    FamilyHooks::<bool>::new(family, Err("synthetic sentinel differs")).is_err()
                );
                assert!(refuse(family, "publication", "synthetic publication failure").is_err());
                continue;
            }
            assert!(
                !family.tables().is_empty(),
                "{} needs a refusal contract",
                family.name()
            );
            let mut broken = FamilyHooks::new(family, Ok(false)).unwrap();
            assert_eq!(
                broken.toggle(true, |_, _| Ok(false)),
                Ok(false),
                "busy is retryable"
            );
            let mut other = FamilyHooks::new(Family::Hourly, Ok(false)).unwrap();
            let progress = crate::progress::Progress::open(&directory);
            let stage = crate::progress::Stage::new(&progress, family.name());
            let mut attempts = 0;
            loop {
                attempts += 1;
                assert!(attempts <= 2, "failed family retried");
                let result = crate::runtime::install_attach_batch(
                    &stage,
                    &mut [
                        (family.name(), &mut || {
                            broken.toggle(true, |installed, enabled| {
                                *installed = enabled;
                                if enabled {
                                    Err("synthetic publication failure")
                                } else {
                                    Ok(true)
                                }
                            })
                        }),
                        ("hourly", &mut || {
                            other.toggle(true, |installed, enabled| {
                                *installed = enabled;
                                Ok(true)
                            })
                        }),
                    ],
                );
                match result {
                    Ok(true) => break,
                    Err(cause) => {
                        assert_eq!(stage.get(), family.name());
                        install_failed(family, cause).unwrap();
                    }
                    Ok(false) => panic!("synthetic families are not busy"),
                }
            }
            assert_eq!(attempts, 2);
            assert_eq!(
                broken.inner,
                Some(false),
                "partial publication must roll back"
            );
            assert_eq!(
                other.inner,
                Some(true),
                "independent family was not installed"
            );
            let mut source = FrameInput {
                branch: 1,
                hour: 10,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| {
                    TableView::Complete(replay_data::rows::TableRows::empty(TableId::ALL[index]))
                        .into()
                }),
            };
            apply(&mut source).unwrap();
            for table in super::contract_tests::expected_tables(family) {
                if family == Family::Templates {
                    assert!(matches!(
                        &source.tables[table.index()],
                        replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
                    ));
                    continue;
                }
                assert!(
                    matches!(&source.tables[table.index()], replay_frame::frame::TableInput::Sampled(TableView::Unavailable { reason })
                    if reason.starts_with("hook family ") && reason.contains("not installed:") && reason.contains("synthetic publication failure")),
                    "{} {table:?}",
                    family.name()
                );
            }
            for table in family.tables() {
                if !super::contract_tests::expected_tables(family).contains(table) {
                    assert!(
                        matches!(
                            &source.tables[table.index()],
                            replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
                        ),
                        "{} {table:?} discarded fresh rows",
                        family.name()
                    );
                }
            }
            assert!(matches!(
                &source.tables[TableId::Clock.index()],
                replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
            ));
            let persisted: serde_json::Value =
                serde_json::from_slice(&std::fs::read(directory.join("status.json")).unwrap())
                    .unwrap();
            assert!(
                persisted["failed_hook_families"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["family"] == family.name())
            );
            let status = serde_json::to_value(status()).unwrap();
            let failure = status
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["family"] == family.name())
                .unwrap();
            assert_eq!(failure["stage"], "publication");
            assert!(!failure["site"].as_str().unwrap().is_empty());
            // A partially published family is retained for restoration, even
            // though it supplies no recorded table. Never discard live relays.
            let mut restored = false;
            broken
                .toggle(false, |_, enabled| {
                    assert!(!enabled);
                    restored = true;
                    Ok(true)
                })
                .unwrap();
            assert!(restored);
            other
                .toggle(false, |installed, enabled| {
                    *installed = enabled;
                    Ok(true)
                })
                .unwrap();
            assert_eq!(other.inner, Some(false));
            let mut absent = FamilyHooks::<bool>::new(family, Err("constructor failed")).unwrap();
            absent
                .toggle(false, |_, _| panic!("unconstructed family restored"))
                .unwrap();
            assert_eq!(source.tables.len(), TABLE_COUNT);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

impl FamilyHooks<crate::faction_goal_changes::Hooks> {
    pub(crate) fn busy_sites(&mut self) -> Vec<&'static str> {
        self.inner
            .as_mut()
            .map_or_else(Vec::new, |hooks| hooks.busy_sites())
    }
}

impl FamilyHooks<crate::faction_companion_changes::Hooks> {
    pub(crate) fn busy_sites(&mut self) -> Vec<&'static str> {
        self.inner
            .as_mut()
            .map_or_else(Vec::new, |hooks| hooks.busy_sites())
    }
}

#[cfg(test)]
#[path = "hook_contract_tests.rs"]
pub(crate) mod contract_tests;

#[cfg(test)]
#[path = "hook_fresh_reads_tests.rs"]
mod fresh_reads_tests;

#[cfg(test)]
#[path = "hook_fresh_producer_tests.rs"]
mod fresh_producer_tests;

#[cfg(test)]
#[path = "hook_constructor_gate.rs"]
mod constructor_gate;
