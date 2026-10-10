//! Capture cadence. Single source of truth.
//!
//! There are three cadences (owner rule): Hourly, Daily and Static.
//! Hourly: map inputs, character identities, unit leaders, the persistent
//! character roles and traits, scientist rosters, operative links and retired
//! records a character-scope trigger reads, and doctrine tactic unlocks. Daily: panel data and slow map
//! inputs such as relations, read once per 24 game-hours. A daily table is
//! walked whole in a fixed slot ([`Cadence::Daily`]) or, when one walk would
//! make its hour spike, spread over the day one owner slice per hour
//! ([`Cadence::Sliced`]): each owner is still read once per day and no hour
//! reads the whole table; SliceReads holds the hour each owner's rows were
//! read at (see [`crate::sliced`]). Static: data that cannot change after a
//! save loads, walked only on a full hour (branch start, load, gap or an
//! explicit refresh request); a table that can change is Daily or Hourly.
//! Off-slot hours record [`SKIPPED_REASON`] and the replay carries the last
//! walk forward; keyframes re-encode the rows already held.

use crate::rows::TableId;

pub const STAGGER_PERIOD: i32 = 24;
pub const SKIPPED_REASON: &str = "staggered off-hours; carry forward";
pub const STAGGER_LIMITATION: &str =
    "refreshed on its scheduled slot; off-slot hours hold the last refresh";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Hourly,
    /// Once per day, the whole table in the given slot.
    Daily(u32),
    /// Daily, spread over the day: one owner slice per hour ([`slice_of`]);
    /// a full hour reads every slice. Each owner's read hour is in
    /// SliceReads; the recorder merges each slice into the previous complete
    /// table.
    Sliced,
    Static,
}

/// The hour slot at which an owner's rows of a [`Cadence::Sliced`] table are
/// refreshed. Owners are native country IDs, so the split follows live data.
pub const fn slice_of(owner: u32) -> u32 {
    owner % STAGGER_PERIOD as u32
}

/// Whether `owner`'s rows of a sliced table are read at this hour.
pub fn slice_active(owner: u32, hour: crate::Hour, force_full: bool) -> bool {
    force_full || slice_of(owner) == hour.rem_euclid(STAGGER_PERIOD) as u32
}

pub const fn cadence(table: TableId) -> Cadence {
    use Cadence::{Daily, Hourly, Sliced, Static};
    match table {
        // Native fill/merge events drain at the next hourly boundary.
        TableId::NavalResultSources => Hourly,
        // Native applications run during the transition to midnight.
        TableId::CountryPowerDaily | TableId::CountryPowerDailyCosts => Daily(0),
        TableId::CountryPowerChanges | TableId::GovernmentChanges
        | TableId::PoliticsCaptureCoverage => Hourly,
        // The stored factory totals change within any hour: controller, idea and
        // decision changes recalculate them immediately. The hourly update applies
        // pending `refresh` and `dirty` recalculations and reassigns constructions.
        // The factory widget ranks hourly beside States and StateBuildings. One
        // scatter per country; unchanged rows encode as no change.
        TableId::AgencyUpgradeLevels | TableId::OperationCompletions => Hourly,
        TableId::CountryFactories => Cadence::Hourly,
        // Live end-screen getters read these operands without a periodic cache.
        TableId::CountryGameScoreInputs
        | TableId::AirWingGameScoreStats
        | TableId::GameScoreMultipliers => Hourly,
        // Available-advisor search membership changes at any hour.
        TableId::AvailableAdvisors => Hourly,
        TableId::OwnedWargoals => Hourly,
        // Accepted native awards belong to the preceding hourly interval.
        TableId::CountryExperienceGains => Hourly,
        TableId::EventFeedHistory => Hourly,
        TableId::PeaceSettlements | TableId::PeaceBids => Cadence::Hourly,
        // Ownership and orders-group membership follow the hourly division read.
        TableId::DivisionCensus => Cadence::Hourly,
        // Mission registration appends; removal keeps surviving occurrences in order.
        // Hourly updates, type changes and home-base changes remove and re-append
        // registrations, so their order must follow hourly TaskForceMissions.
        TableId::NavalMissionLedgers | TableId::NavalMissionTaskForces => Hourly,
        // Research inputs change at least daily for every researching country;
        // a whole-table walk is the family's largest cost.
        TableId::ResearchInputs | TableId::ResearchAppliedBonuses | TableId::ResearchModifiers => {
            Sliced
        }
        // Raid inputs: a daily read per owner, one owner slice per hour, with
        // no change fingerprint (owner rule: no harsher than the game, whose
        // raid offers are re-checked in a rolling sweep); an owner new to the
        // census, or whose read was not published, is read at the next hour.
        TableId::RaidCountryInputs
        | TableId::StockpileArchetypes
        | TableId::RaidAirSites
        | TableId::AceRangeInputs => Sliced,
        // Commanders' retreat bans, per roster owner's divisions.
        TableId::DivisionRetreatBans => Sliced,
        // State cores, with the raid state family.
        TableId::StateCores => Sliced,
        TableId::StateResourceInputs
        | TableId::StateResources
        | TableId::StateDynamicModifiers
        | TableId::CountryBuildingResources => Sliced,
        // Technology rosters: daily progress for every country. Definitions
        // are the templates the hour's rosters reach, merged by handle; the
        // console command reloadtechnologies can rewrite them after a load.
        TableId::ResearchTechnologies | TableId::TechnologyDefinitions => Sliced,
        // Held by the collector and certified by its native-writer hooks: a
        // slice keeps each hour's copy small and a change is emitted at once.
        TableId::AirCountryRelations | TableId::AirNavalPatrolHistories => Sliced,
        // Stability and war-support breakdowns: one country slice per hour
        // plus every country whose change fingerprint moved (bases, stored
        // penalty and coastal terms, ruling party popularity, the war counter,
        // the tree's child count and the tracked modifier totals), so a
        // recalculated tree or a weekly base change is read within the hour
        // without walking every country's tree and sources at one hour. The
        // war flags and world tension join a read but are not fingerprinted:
        // a change of only those reaches the rows at the country's slice.
        TableId::CountryStabilityInputs | TableId::CountryStabilityLines => Sliced,
        // Naval mission icon country and commander modifiers (owner decision 2026-09-25:
        // daily slices, up to a day old). A country new to the family, or whose task forces
        // name another leader or headquarters commander, is read within the hour.
        TableId::NavalMissionCountries | TableId::NavalMissionCommanders => Sliced,
        // Cannot change after the save loads (the DLC set included).
        TableId::LexicalTokens | TableId::AirSortieModifierMetadata => Static,
        // Native grouping and naming can change at any hour, including faction
        // changes, goals, major flags and controlled core factory levels.
        TableId::WarOwnedRelations | TableId::WarRelationGoals | TableId::WarOverviewCountries | TableId::WarNameRules => Cadence::Hourly,
        // Modifier definitions are registered only by database loaders; the
        // collector also re-reads them whenever the table's data pointer or
        // count changes, as it does the lexer on a new maximum token.
        TableId::ModifierDefinitions => Static,
        // Relations feed the perspective. Daily (owner decision): an opinion
        // or relation change made by an event can show up to a game day late.
        // One walk of every owner would spike its hour at thousands of
        // countries, so each owner's relations are read in its slice.
        TableId::RelationOpinions | TableId::RelationTuples => Sliced,
        // The Combat Log grows at any hour but only its arrays' last two
        // month entries can change after a read in this or the previous
        // calendar month, so each country's slice reads those (every month
        // at a full hour, or when its last complete read is older).
        // An observer logs every country: one walk of all of them would spike
        // its hour. Countries holding the log of a deleted orders group are
        // also read at the last hour before the daily prune deletes it.
        TableId::CombatLogEquipment => Sliced,
        // Province names are live overrides: set_province_name and
        // reset_province_name effects can change them after a save loads,
        // so they are daily, not static; a rename can show up a day late.
        TableId::ProvinceNames => Daily(1),
        // Daily slots: whole shared-source families stay together; a
        // definition table shares the slot of the census that reaches it.
        // Intel lanes change only in the daily update, which runs inside the
        // hourly update at hour 23; slot 0 is the first sample after it.
        TableId::CountryIntel => Daily(0),
        // The logistics balance and stock are written by the same daily update,
        // so slot 0 reads the value the day computed; within the day the record
        // changes only when equipment leaves or enters the stockpile outside it.
        TableId::LogisticsEquipment => Daily(0),
        TableId::ProjectPrograms
        | TableId::Projects
        | TableId::ProjectHistory
        | TableId::ProjectBreakthroughs => Daily(4),
        // The 3D model's equipment inputs. The largest eligible variant can
        // change mid-day (reinforcement, upgrade deliveries, captures) and the
        // game re-selects the entity on each graphical-map update; the record
        // catches up at the next slot. A division deployed after the slot is in
        // the hourly NewDivisionEquipment until then. Local sub-unit clones
        // change only when research or a doctrine unlocks one.
        TableId::DivisionEquipment
        | TableId::CountryLocalSubunits
        | TableId::ArmyUpgradeGroups
        | TableId::ArmyUpgradeRequests
        | TableId::ArmyUpgradeDeliveries
        | TableId::ArmyUpgradeProduced => Daily(2),
        // Each construction line's output totals are published with the queues'
        // census, which decides which lines ended and which were created after
        // observation began.
        TableId::StateBaseResources
        | TableId::BuildingConstruction
        | TableId::RailwayConstruction
        | TableId::ConstructionLineOutput
        | TableId::OccupationSettings
        | TableId::StateOccupationLaws
        | TableId::StateGarrisons
        | TableId::StateAssignedManpower
        | TableId::CountryOccupationReserveManpower
        | TableId::StateResistance
        | TableId::CountryCollaboration => Daily(3),
        TableId::AirStatistics => Daily(5),
        TableId::DecisionCounts | TableId::Decisions | TableId::DecisionRandomItems => Daily(6),
        TableId::AirRegionHistory | TableId::AirRegionDamage => Daily(7),
        // Organisations share the lines' slot: a line names its organisation,
        // and the history's units tile with the lines' daily reads. Each
        // line's output totals are published with the census that decides
        // which lines ended and which were created after observation began.
        TableId::ProductionLines
        | TableId::CountryProductionModifiers
        | TableId::ProductionLineModifiers
        | TableId::IndustrialOrganisations
        | TableId::IndustrialOrganisationHistory
        | TableId::ProductionLineOutput
        | TableId::RocketProductionOutput
        | TableId::Stockpiles
        | TableId::ProductionLicenses
        | TableId::DeploymentConveyors
        | TableId::DeploymentLines
        | TableId::DeploymentManpower
        | TableId::DeploymentEquipment
        | TableId::ProductionLicenseRecords => Daily(8),
        TableId::DivisionTemplates => Daily(9),
        // CountryResources is hourly: a tactic's resource-count trigger reads its
        // produced and adjustment pools. Resource effects, country release and
        // building level changes rewrite them at any hour, as does the daily update;
        // a peace conference recomputes them in full.
        // ResourceExchanges is hourly: player and AI trade commands and import
        // effects create, resize and cancel deals. Civilian-factory consumption can
        // remove them. The hourly trade update rewrites deliveries and terminates
        // deals, as resource recalculations do after the changes above.
        // The collector walks the resource census with both every hour.
        // ResourceDefinitions stays daily: its rows are content, set only as it loads.
        TableId::ResourceDefinitions => Daily(10),
        // The raid army search's strategic-region access modes, which the
        // player's UI and the AI's daily step set.
        TableId::ResearchSlots | TableId::NavalRegionAccess => Daily(11),
        // Theatre details share one daily slot. LandTheatres itself (id,
        // owner, roster ordinal) falls to the hourly arm below: hourly
        // OrdersGroups rows name their theatre and battle plans draw in its
        // roster order, so the roster must come from the same hour.
        TableId::LandTheatreGroups
        | TableId::LandTheatreMembers
        | TableId::AirTheatres
        | TableId::AirTheatreGroups
        | TableId::AirTheatreMembers => Daily(13),
        // Graph strengths are applied inside the hour-23 daily tick. Slot 0,
        // like CountryIntel, observes their first completed state.
        TableId::IntelNetworks => Daily(0),
        TableId::DynamicModifiers => Daily(15),
        TableId::OperativeMemberships
        | TableId::CapturedOperativeReferences
        | TableId::WingCumulative => Daily(16),
        // Script effects can change the selected balance and its presentation
        // in any tick, and scripted text reads them in the hour.
        TableId::PowerBalances
        | TableId::PowerBalanceMembers
        | TableId::PowerBalanceRanges
        | TableId::PowerBalanceOverrides => Cadence::Hourly,
        // Scientists and ScientistSkills, one roster walk, are hourly: recruitment
        // adds scientists to the roster and trait effects append traits between
        // daily ticks.
        // The fuel status and its consumers come from one walk. CountryFuel's
        // ring sums cover the last 24 hourly fuel updates, so reads on this
        // daily slot tile time exactly.
        TableId::CountryFuel
        | TableId::CountryFuelConsumers
        | TableId::CountryExperience
        | TableId::HubMotorization
        | TableId::DisruptedSupply
        | TableId::ForeignHomebases
        | TableId::PurchaseContracts => Daily(17),
        TableId::PreferredTactics
        | TableId::WorldTension
        | TableId::ThreatSources
        | TableId::ThreatCountryAggregates
        | TableId::TheatreAreas => Daily(18),
        // Raids stay hourly: RaidTargets is the same walk's map-icon projection.
        TableId::SunkShips | TableId::Focus | TableId::CompletedFocuses => Daily(19),
        TableId::OperationTokens => Daily(20),
        // Agency creation and renaming can run in any tick, and the agency display
        // reads the stored created byte and name in the hour.
        TableId::IntelligenceAgencies => Cadence::Hourly,
        // The persistent role and trait tables are hourly: their writers run
        // between daily ticks, and a tactic's character-scope `has_trait` reads
        // unit-leader traits. Learning and officer commands, trait effects and
        // experience thresholds add traits; removal effects erase them.
        // Country-leader and advisor role effects insert and erase persistent
        // roles whenever an event option is chosen.
        // UnitLeaderTraitExperience comes from the same trait walk, so it is
        // hourly with UnitLeaderTraits. HiredAdvisors, its own walk, keeps
        // this slot: no hourly consumer reads it, and the hourly AdvisorRoles
        // is read in the same hour, so every appointed advisor's role (traits,
        // idea, slot) comes from that refresh. An appointment made after the
        // slot appears at the next day's slot.
        TableId::AirAces | TableId::PeaceConferences | TableId::HiredAdvisors => Daily(21),
        TableId::IncomingDiplomaticActions
        | TableId::TechSharingGroups
        | TableId::TechSharingMembers
        // Variants and the rosters that name them share a slot, so a roster
        // never names a variant the same refresh lacks. A new variant reaches a
        // division or ship only through days of production.
        | TableId::VariantDefinitions
        | TableId::VariantModules
        | TableId::VariantStats
        | TableId::VariantUpgrades
        | TableId::AvailableEquipmentVariants => Daily(22),
        // Lend-lease totals grow at deliveries and sinkings; the exchange is
        // deleted, with its totals, when the lend-lease ends.
        TableId::NavalSupremacy
        | TableId::Operations
        | TableId::OperationAssignments
        | TableId::SunkConvoys
        | TableId::LendLeaseEquipment => Daily(23),
        // RadarCoverage is hourly because a controller change edits radar
        // groups at any hour. Cost over 25 hours on a vanilla map:
        // 0.11 ms p50 / 0.16 ms p95 read on the game thread, a 0.12 ms/h raw
        // copy on the decode worker, 104 B/h encoded.
        // The DLC mask is fixed for the session, but `human_ai` is a debug
        // toggle the console can flip at any hour; both are single reads.
        TableId::SimulationFeatures => Cadence::Hourly,
        // A tag switch or observer command can run between any two ticks, and
        // each tick's combat manager tests the viewing choice once; one eight-byte read.
        TableId::LocalPlayer => Cadence::Hourly,
        // Cryptology is hourly: activation and scripted progress can change the
        // raid target-intel predicate between daily ticks.
        TableId::Cryptology => Cadence::Hourly,
        // ScriptFlags, ScriptVariables and ActiveIdeas are hourly: the triggers
        // that read them see an effect's write in the same hour, and effects run
        // whenever an event option, decision or focus fires. Only effects, the
        // console, a load and resets write persistent variables; no engine pass
        // writes them periodically. Country and character flag effects set, clear
        // and modify flags; idea effects add, remove and swap ideas.
        // Saving and clearing event targets can write at any hour. Readers see
        // those writes immediately, so daily reads are stale. The viewer evaluates
        // raid `visible` and `show_target` every hour, including event-target and
        // tag operands.
        TableId::GlobalSavedEventTargets => Cadence::Hourly,
        TableId::EquipmentDestruction => Cadence::Hourly,
        TableId::AirLossEvents | TableId::BuildingBombingDamage | TableId::AirDailyRoster => Cadence::Hourly,
        // Owned simulation operations and coverage are drained at the hourly cut.
        TableId::ReserveModifierBreakdowns
        | TableId::PowerBalanceModifierBreakdowns
        | TableId::NavalModifierBreakdowns
        | TableId::AttritionModifierBreakdowns => Cadence::Hourly,
        TableId::ScriptFlags
        | TableId::ScriptVariables
        | TableId::ActiveIdeas
        | TableId::TimedIdeas => {
            Cadence::Hourly
        }
        _ => Cadence::Hourly,
    }
}

/// The daily slot, for slot balancing and status.
pub const fn refresh_slot(table: TableId) -> Option<u32> {
    match cadence(table) {
        Cadence::Daily(slot) => Some(slot),
        Cadence::Hourly | Cadence::Sliced | Cadence::Static => None,
    }
}

/// Refreshed one owner slice per hour and merged into the previous table.
pub const fn is_sliced(table: TableId) -> bool {
    matches!(cadence(table), Cadence::Sliced)
}

/// Not walked every hour; off-hours carry the last walk forward.
pub const fn is_staggered(table: TableId) -> bool {
    matches!(cadence(table), Cadence::Daily(_) | Cadence::Static)
}

pub fn is_refresh(table: TableId, hour: crate::Hour) -> bool {
    cadence(table).is_refresh(hour)
}

impl Cadence {
    pub fn is_refresh(self, hour: crate::Hour) -> bool {
        match self {
            // Every hour reads one slice.
            Cadence::Hourly | Cadence::Sliced => true,
            Cadence::Daily(slot) => hour.rem_euclid(STAGGER_PERIOD) == slot as i32,
            Cadence::Static => false,
        }
    }
}

pub fn should_walk(table: TableId, hour: crate::Hour, force_full: bool) -> bool {
    force_full || is_refresh(table, hour)
}

pub fn refresh_text(table: TableId) -> &'static str {
    cadence(table).refresh_text()
}

impl Cadence {
    pub fn refresh_text(self) -> &'static str {
        const DAILY: [&str; 24] = [
            "H%24==0", "H%24==1", "H%24==2", "H%24==3", "H%24==4", "H%24==5", "H%24==6", "H%24==7",
            "H%24==8", "H%24==9", "H%24==10", "H%24==11", "H%24==12", "H%24==13", "H%24==14",
            "H%24==15", "H%24==16", "H%24==17", "H%24==18", "H%24==19", "H%24==20", "H%24==21",
            "H%24==22", "H%24==23",
        ];
        match self {
            Cadence::Hourly => "hourly",
            Cadence::Sliced => "one owner slice per hour, merged into the held table",
            Cadence::Daily(slot) => DAILY[slot as usize],
            Cadence::Static => "static",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cadence_refreshes_its_count_per_day_and_full_hours_walk_everything_recorded() {
        for base in [0, 1, 60_820_531, i32::MAX - 24] {
            for table in TableId::ALL {
                let hits = (base..base + 24).filter(|h| is_refresh(table, *h)).count();
                let expected = match cadence(table) {
                    Cadence::Hourly | Cadence::Sliced => 24,
                    Cadence::Daily(_) => 1,
                    Cadence::Static => 0,
                };
                assert_eq!(hits, expected, "{table:?} near {base}");
                assert!(should_walk(table, base, true));
            }
        }
    }

    #[test]
    fn map_tables_are_hourly_and_families_share_a_slot() {
        for table in [
            TableId::Clock,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::OrdersGroupCounterInputs,
            TableId::Provinces,
            TableId::States,
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            // Equipment of divisions deployed since the daily walk.
            TableId::NewDivisionEquipment,
            TableId::DivisionEtaInputs,
            TableId::LandTheatres,
            TableId::FactionTheatres,
            TableId::RadarCoverage,
            TableId::RelationLinks,
            // A template edit changes its counters' icons at any hour.
            TableId::DivisionTemplateIcons,
            TableId::CountryVisibility,
            TableId::StateVisibility,
            TableId::ShipModels,
            TableId::SupplyNodes,
            TableId::WeatherProvinces,
            TableId::LandCombatDivisions,
            TableId::LandCombatSides,
            TableId::NavalModifierBreakdowns,
            TableId::AirBaseRegistrations,
            // The land counter's ability grid: abilities start and change phase at any hour.
            TableId::ActiveAbilities,
            // Bombing changes a building's mesh and fire within the hour.
            TableId::StateBuildings,
            TableId::ProvinceBuildings,
            TableId::RaidTargetStatuses,
            // Roles, traits and scientist rosters whose writers run between
            // daily ticks; each trait and roster walk moves whole.
            TableId::CountryLeaderRoles,
            TableId::AdvisorRoles,
            TableId::UnitLeaderTraits,
            TableId::UnitLeaderTraitExperience,
            TableId::Scientists,
            TableId::ScientistSkills,
            // Their other character-scope inputs: every role's traits, the
            // operative link and the retired records.
            TableId::CharacterRoleTraits,
            TableId::CharacterOperatives,
            TableId::CountryRetiredCharacters,
            // Script-effect state the triggers read.
            TableId::ScriptFlags,
            TableId::ScriptVariables,
            TableId::ActiveIdeas,
            // Scripted text reads the balance and the agency of the hour.
            TableId::PowerBalances,
            TableId::PowerBalanceMembers,
            TableId::PowerBalanceRanges,
            TableId::PowerBalanceOverrides,
            TableId::IntelligenceAgencies,
            // The game's factory totals, recalculated within any hour.
            TableId::CountryFactories,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert_eq!(refresh_text(table), "hourly");
        }
        // The appointed-advisor list keeps its daily slot; the advisor roles
        // it names are read every hour, so also at that slot.
        assert_eq!(refresh_slot(TableId::HiredAdvisors), Some(21));
        for family in [
            &[
                TableId::ProjectPrograms,
                TableId::Projects,
                TableId::ProjectHistory,
                TableId::ProjectBreakthroughs,
            ][..],
            &[TableId::RelationOpinions, TableId::RelationTuples][..],
            &[
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantStats,
                TableId::AvailableEquipmentVariants,
            ],
            &[TableId::DivisionEquipment, TableId::CountryLocalSubunits],
            &[
                TableId::ProductionLines,
                TableId::CountryProductionModifiers,
                TableId::ProductionLineModifiers,
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationHistory,
                TableId::ProductionLineOutput,
                TableId::RocketProductionOutput,
            ],
            &[
                TableId::BuildingConstruction,
                TableId::RailwayConstruction,
                TableId::ConstructionLineOutput,
            ],
        ] {
            let shared = cadence(family[0]);
            assert_ne!(shared, Cadence::Hourly);
            assert!(family.iter().all(|t| cadence(*t) == shared), "{family:?}");
        }
        assert_eq!(cadence(TableId::RelationTuples), Cadence::Sliced);
        assert_eq!(refresh_slot(TableId::RelationTuples), None);
        assert_eq!(refresh_slot(TableId::Projects), Some(4));
        assert_eq!(cadence(TableId::LexicalTokens), Cadence::Static);
        assert_eq!(refresh_text(TableId::LexicalTokens), "static");
        assert!(is_staggered(TableId::LexicalTokens));
        assert_eq!(refresh_slot(TableId::LexicalTokens), None);
        assert_eq!(cadence(TableId::ModifierDefinitions), Cadence::Static);
    }

    #[test]
    fn sliced_tables_read_every_owner_once_per_day_and_everything_on_full_hours() {
        for table in [
            TableId::ResearchInputs,
            TableId::RaidCountryInputs,
            TableId::RaidAirSites,
            TableId::AceRangeInputs,
            TableId::StateResourceInputs,
            TableId::StateResources,
            TableId::StateDynamicModifiers,
            TableId::CountryBuildingResources,
        ] {
            assert!(is_sliced(table), "{table:?}");
            assert!(!is_staggered(table), "{table:?}");
        }
        assert_eq!(refresh_slot(TableId::ResearchModifiers), None);
        assert!(is_sliced(TableId::CountryStabilityInputs));
        assert!(is_sliced(TableId::CountryStabilityLines));
        for owner in [0, 1, 23, 24, 439, 5_000, u32::MAX] {
            for base in [0, 60_827_795, -48] {
                let hits = (base..base + 24)
                    .filter(|hour| slice_active(owner, *hour, false))
                    .count();
                assert_eq!(hits, 1, "owner {owner} near {base}");
                assert!(slice_active(owner, base, true));
            }
        }
    }

    #[test]
    fn province_name_changes_are_recaptured_within_a_day() {
        assert_eq!(cadence(TableId::ProvinceNames), Cadence::Daily(1));
        for changed_at in -24..24 {
            let next_refresh = (changed_at + 1..=changed_at + 24)
                .find(|hour| should_walk(TableId::ProvinceNames, *hour, false))
                .expect("a changed override must be revisited within a game day");
            assert_eq!(next_refresh.rem_euclid(24), 1);
            assert!(should_walk(TableId::ProvinceNames, changed_at, true));
        }
    }

    #[test]
    fn relations_are_read_once_a_day_one_owner_slice_per_hour() {
        for table in [
            TableId::RelationOpinions,
            TableId::RelationTuples,
            TableId::CombatLogEquipment,
        ] {
            assert!(is_sliced(table), "{table:?}");
            assert!(!is_staggered(table), "{table:?}");
        }
    }
    #[test]
    fn air_ledger_event_tables_drain_each_hour() {
        for table in [
            TableId::AirLossEvents,
            TableId::BuildingBombingDamage,
            TableId::AirDailyRoster,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            assert_eq!(refresh_slot(table), None);
            assert!((-48..48).all(|hour| should_walk(table, hour, false)));
        }
    }
}
