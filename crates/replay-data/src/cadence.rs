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
//!
//! The crate's `cadence-audit.md` lists, for every Hourly table,
//! the native writers that keep it hourly: an hourly-tick writer, an any-hour
//! writer (effects, commands, actions, console) or a hook log. A table moves
//! off Hourly only when every writer of every field it records runs on the
//! slower cadence, or when an owner decision accepts a day's staleness for a
//! table only panels read (noted beside its cadence). The 2026-10-21 owner
//! exception names UnitLeaders.experience, CombatLogObservers.a/b and
//! DivisionStatus.supply/experience/officer_experience; their other columns
//! and row membership remain hourly.

use crate::rows::TableId;

/// Owner decision 2026-10-21 accepts daily samples for UnitLeaderTraitExperience,
/// UnitLeaders.experience, CombatLogObservers.a/b and the DivisionStatus supply
/// footer/stored XP/officer XP only. Other fields keep their existing cadence
/// and writer-certificate requirements. The division's supply ratio, exact
/// ratio, grace and native exercise decision remain hourly. New occurrences
/// read fresh; daily/full boundaries revoke column carry before dispatch.
pub const COMMANDER_PROGRESS_SLOT: u32 = 12;
pub const COMBAT_LOG_NUMERIC_SLOT: u32 = 12;
pub const DIVISION_STATUS_DAILY_SLOT: u32 = 12;

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
    /// table. A family whose hooked writers mark the owners they change also
    /// reads those within the hour; one whose every owner changes in the
    /// daily tick (Focus) reads every owner at the first hour after it
    /// instead of a slice.
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
        // Any-hour faction commands/effects and lifecycle writers certify the
        // retained state. This availability cadence adds no hourly native scan;
        // the existing faction worker reads only changed owner domains.
        TableId::FactionGoalStatus | TableId::FactionGoalEntries
        | TableId::FactionRules | TableId::FactionPrograms
        | TableId::FactionManpower | TableId::FactionUnlocks | TableId::FactionVariables | TableId::FactionAccounts | TableId::FactionResearch | TableId::FactionMinimumUpgrades | TableId::FactionDoctrine | TableId::FactionProjection | TableId::FactionResources | TableId::FactionAdvisorModifiers => Hourly,
        // Commands and last-mission removal write the flag. The existing
        // mission worker carries unchanged demanded regions under writer hooks.
        TableId::AirGroundCrews => Hourly,
        // The existing mission worker carries direct registration order and
        // reads progress only for incoming wings admitted by the native gate.
        TableId::AirTransferInputs => Hourly,
        TableId::NavalAccidentReports => Hourly,
        TableId::NavalRegionalMines => Hourly,
        TableId::NavalDominanceTaskForceRanges => Hourly,
        TableId::NavalPatrolMissionInputs => Hourly,
        // Native fill/merge events drain at the next hourly boundary.
        TableId::NavalResultSources => Hourly,
        // The military-staff selector and country lock can be commanded at any hour.
        TableId::CountryTemplateState => Hourly,
        // Native applications run during the transition to midnight.
        TableId::CountryPowerDaily | TableId::CountryPowerDailyCosts => Daily(0),
        TableId::CountryPowerChanges | TableId::GovernmentChanges
        | TableId::PoliticsCaptureCoverage => Hourly,
        // The stored factory totals change within any hour: controller, idea and
        // decision changes recalculate them immediately. The hourly update applies
        // pending `refresh` and `dirty` recalculations and reassigns constructions.
        // The factory widget ranks hourly beside States and StateBuildings. One
        // scatter per country; unchanged rows encode as no change.
        TableId::AgencyOperationBuffers | TableId::AgencyUpgradeLevels | TableId::OperationCompletions => Hourly,
        TableId::CountryFactories => Cadence::Hourly,
        // Live end-screen getters read these operands without a periodic cache.
        TableId::CountryGameScoreInputs
        | TableId::AirWingGameScoreStats
        | TableId::GameScoreMultipliers => Hourly,
        // Available-advisor search membership changes at any hour,
        // but only panels read the pools: the advisor-idea lookup of effect
        // descriptions and the oracle. Owner decision 2026-10-11:
        // panel-only values refresh once a game day, up to a day stale, so each
        // country's pool is read at its owner slice. AdvisorTokens, which
        // has_idea triggers read, stays hourly.
        TableId::AvailableAdvisors => Sliced,
        // The decorative supply trains (viewer map/trains.rs) and
        // the panel-only stored vehicle operands get_supply_vehicles_temp
        // describes. The hourly supply update rebuilds every node and
        // resets the train and truck demand each hour; owner
        // decision 2026-10-11: they refresh once a game day, up to a day stale,
        // so each country's companion is read at its owner slice. Nodes and
        // inventory share the slice, so a country's node rows always match its
        // node_count. ProvinceSupply, which the supply map mode and railways
        // draw, stays hourly.
        TableId::SupplyNodes | TableId::TrainInventory => Sliced,
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
        // Research inputs: one country slice per hour plus every country a hooked writer
        // (assignment, levels, bonuses, sharing, the modifier tree, ideas, scientists) marked since
        // the previous hour, so a change is read within the hour and a quiet country costs no read.
        // Daily progress and banked idle days drift only for a country owning a state: the
        // country's daily update returns before the technology update when its owned-state count is
        // below 1. They reach the rows at the country's slice; a stateless country's slice reads
        // nothing, unless it gained or lost its last state since its last slice.
        TableId::ResearchInputs | TableId::ResearchAppliedBonuses | TableId::ResearchModifiers => {
            Sliced
        }
        // RaidCountryInputs and AceRangeInputs fall to the hourly arm (owner
        // decision 2026-10-11: raid inputs equal the game's at every hour,
        // whose offer cache re-checks every offer at least hourly); both share
        // CountryCommandPower's hourly country pass, which already copies
        // every country's accumulator and roster headers. Convoy reservations
        // change at any hour: supply distribution, trade, task-force
        // maintenance and naval transfers request convoys. An ace's wing and
        // alive byte change with assignment (assign and delete-wing commands,
        // deployment and the hourly air-base tick) and death in hourly air
        // combat. Its range terms are copied from its type at creation,
        // including effect, generated and parsed aces, and only read after.
        //
        // StockpileArchetypes falls to the hourly arm too (owner decision
        // 2026-10-11): stock follows production and reinforcement through
        // the equipment pool's quantity additions, too hot to hook,
        // so every country is read each hour, keeping only the archetypes a
        // raid type lists as essential, in whole units capped at the largest
        // amount any raid type requires; the requirement check's value then
        // changes only while a country is short.
        //
        // RaidAirSites, with no owner slice: an owner new to the census, or
        // whose read was not published, is read at the next hour, and each
        // hour reads every owner a hooked site list writer marked
        // (raid_changes: append, air-base access refresh, withdrawal,
        // transfer, clear, construction, destruction, initialization and
        // load, and a pool-capacity setter when it changes a capacity), so it
        // equals the game's at every hour; every other owner's rows carry.
        TableId::RaidAirSites => Sliced,
        // DivisionRetreatBans falls to the hourly arm: the selected-route
        // search reads the commander's keys live as the combat-disengagement
        // check does. A commander's blocks change at any hour after modifier
        // recalculation from traits, skills and daily dynamic modifiers, or
        // after HQ-scaled modifier updates when a group's HQ moves or its
        // membership changes. Every division is read each hour; a commander
        // is read again only after one of those writers ran
        // (retreat_ban_changes, forwarded by naval_tree_changes).
        // The raid state inputs and the building-resource overrides (owner
        // decision 2026-10-11: raid inputs equal the game's at every hour,
        // whose offer cache re-checks every offer at least hourly).
        // Each hour reads every state or country a hooked writer marked since
        // the previous hour (the collector's raid_changes: resource modifier
        // gate refresh, building dynamic modifier addition and removal,
        // resource addition, timed-resource expiry, core insertion and
        // removal, resource-origin construction and destruction, building
        // resource modifier addition, override clearing and the forwarded
        // building writers) and every new or unpublished owner; an hour whose
        // marks are not certified reads every owner. Only
        // StateResourceInputs.supply_route, which the supply update rewrites
        // every hour, keeps a daily slice: a state holding a resource origin
        // is read at its slice, and the flag is zero without one.
        // Trade routes, by the country holding them, with no owner slice: each hour reads an
        // owner whose set of referenced routes changed and every owner one of whose routes a
        // hooked route writer touched (the collector's raid_changes: route update, which is the
        // only caller of both path builders, route construction, the save loader and a country's
        // delivery-route construction); an hour whose marks are not certified reads every
        // owner.
        TableId::TradeRoutes => Sliced,
        TableId::StateCores => Sliced,
        TableId::StateResourceInputs
        | TableId::StateResources
        | TableId::StateDynamicModifiers
        | TableId::CountryBuildingResources => Sliced,
        // Technology rosters: one country slice per hour plus every country
        // a hooked writer of a technology's level, classifier, researched
        // membership or applied bonuses marked since the previous hour
        // (technology assignment and level setting, slot completion, research
        // completion and reset, bonus claim and release, roster rebuilds,
        // the research console, country reset and fresh initialization of a
        // reused dynamic country), so a has_tech answer changes within the
        // hour; an uncertified hour reads every country. Only progress, which
        // the daily research step advances once a day for each researching
        // technology of a country owning a state, reaches the rows at the
        // country's slice (a stateless country's slice reads nothing unless
        // its drift changed since): no trigger reads it (the research panel
        // and its tooltips show it), and panel-only numbers may be a day stale
        // (owner decision 2026-10-11).
        // Definitions are the templates the hour's rosters reach, merged by
        // handle. Templates change after a load only through the console's
        // reloadtechnologies, which re-parses and postprocesses definitions,
        // then rebuilds every roster. Its whole call marks every country,
        // so the next hour reads every roster and replaces them.
        TableId::ResearchTechnologies | TableId::TechnologyDefinitions => Sliced,
        // Held by the collector and certified by its native-writer hooks: a
        // slice keeps each hour's copy small and a change is emitted at once.
        TableId::AirCountryRelations | TableId::AirNavalPatrolHistories => Sliced,
        // Stability and war-support breakdowns: one country slice per hour
        // plus every country a hooked writer (bases, the weekly penalties,
        // the modifier tree and its sources, wars, exiles, popularity and
        // interception) marked since the previous hour, so a change is read
        // within the hour and a quiet country costs no read. The daily drift
        // (ruling party popularity, coastal protection, the interception
        // window and world tension) reaches the rows at the country's slice.
        TableId::CountryStabilityInputs | TableId::CountryStabilityLines => Sliced,
        // Naval mission icon country and commander modifiers: one country slice per hour,
        // and every country whose certificate moved is read within the hour (the collector's
        // naval_mission_inputs): its task forces' leader and commander references, the naval
        // tree family's certificates of its accumulator (modifier recalculation and appearance
        // reset) and its fleet leaders' (modifier and HQ-scaled modifier rebuilds, leader
        // construction and destruction), its navy's stored hubs and its headquarters
        // commanders' skill objects (skill setters).
        TableId::NavalMissionCountries | TableId::NavalMissionCommanders => Sliced,
        // Cannot change after the save loads (the DLC set included).
        TableId::LexicalTokens | TableId::AirSortieModifierMetadata => Static,
        // Native grouping and naming can change at any hour, including faction
        // changes, goals, major flags and controlled core factory levels.
        TableId::WarParticipation | TableId::WarCapturedProvinces | TableId::WarLendLeaseHistories | TableId::WarOwnedRelations | TableId::WarRelationGoals | TableId::WarOverviewCountries | TableId::WarNameRules
        | TableId::CountryWargoals | TableId::WargoalTargets | TableId::WargoalJustifications => Cadence::Hourly,
        // Modifier definitions are registered only by database loaders; the
        // collector also re-reads them whenever the table's data pointer or
        // count changes, as it does the lexer on a new maximum token.
        TableId::ModifierDefinitions => Static,
        // Relations feed the perspective, opinion predicates and the naval map's diplomatic
        // gradients. One walk of every owner would spike its hour at thousands of countries, so
        // each owner's relations are read in its slice, and each hour also reads every owner a
        // drained mark names (the collector's opinion_changes): opinion recomputation, the only
        // run-time opinion store, marks a status whose opinion changed. Reset and the sixteen
        // relation slot replacements, including the war body the stability family forwards, are the
        // only run-time writers of the serializable relation vector and mark theirs on every call.
        // Loading and construction end the observation. A changed opinion or relation is read the
        // hour after it changed; an hour whose marks are not all handed over reads every owner.
        TableId::RelationOpinions | TableId::RelationTuples => Sliced,
        // The Combat Log grows at any hour but only its arrays' last two
        // month entries can change after a read in this or the previous
        // calendar month, so each country's slice reads those (every month
        // at a full hour, or when its last complete read is older).
        // An observer logs every country: one walk of all of them would spike
        // its hour. Countries holding the log of a deleted orders group are
        // also read at the last hour before the daily prune deletes it.
        TableId::CombatLogEquipment => Sliced,
        // Owner decision 2026-10-21: training progress and runtime factors may
        // be up to a day stale despite their any-hour writers. Residual XP uses
        // the same slot, but the rest of UnitLeaders remains hourly.
        TableId::UnitLeaderTraitExperience => Daily(COMMANDER_PROGRESS_SLOT),
        // Daily slots: whole shared-source families stay together; a
        // definition table shares the slot of the census that reaches it.
        // Intel lanes change only in the daily update, which runs inside the
        // hourly update at hour 23; slot 0 is the first sample after it.
        TableId::CountryIntel => Daily(0),
        // The logistics balance and stock are written by the same daily update,
        // so slot 0 reads the value the day computed; within the day the record
        // changes only when equipment leaves or enters the stockpile outside it.
        TableId::LogisticsEquipment => Daily(0),
        // The recorded rows are each strategic region's sorted dominance cache (its entries and
        // count). Its sole writer runs in the daily naval step and in the post-read callback the
        // persistent reader calls when a save loads. The daily step runs inside the update entered
        // at hour 23, so slot 0 is the first sample of each cache epoch and certifies every hour of
        // its day; the political naval FX (viewer naval_dominance.rs) refuses any sample from an
        // earlier epoch. A load or full refresh reads the cache at its own hour, which is that
        // epoch's cache until the next midnight.
        TableId::NavalSupremacy => Daily(0),
        // Special projects and their reward histories, which project triggers in scripted map
        // offers read: the daily pool pass advances every project inside the hour entered at 23 for
        // every country owning a state (the country's daily update returns earlier when its
        // owned-state count is below 1), so each of those is read at the next hour. Other writers
        // mark their project's owner, which is read within the hour: start, stop, cancel (including
        // capture), complete (including completion effects), progress-ratio addition (ratio
        // effects, capture and supportive scientists), prototype reward-option application, forced
        // update (dismantling), database resync and the forwarded country appearance reset.
        TableId::Projects | TableId::ProjectHistory => Sliced,
        // The panel-only facility programs and breakthrough points keep their
        // slot: their walk is separate from the pools'.
        TableId::ProjectPrograms | TableId::ProjectBreakthroughs | TableId::ScientistDaily => Daily(4),
        // The 3D model's equipment inputs. The largest eligible variant can
        // change mid-day (reinforcement, upgrade deliveries, captures) and the
        // game re-selects the entity on each graphical-map update; the record
        // catches up at the next slot. A division deployed after the slot is in
        // the hourly NewDivisionEquipment until then.
        // CountryLocalSubunits falls to the hourly arm: the selectors read a
        // country's local sub-unit clones, which research completion,
        // doctrine and unlock effects and army addition create and rebuild
        // at any hour; the collector carries every country no hooked writer
        // touched (local_subunit_changes).
        TableId::DivisionEquipment
        | TableId::ArmyUpgradeGroups
        | TableId::ArmyUpgradeRequests
        | TableId::ArmyUpgradeDeliveries
        | TableId::ArmyUpgradeProduced => Daily(2),
        // Each construction line's output totals are published with the queues'
        // census, which decides which lines ended and which were created after
        // observation began. StateBaseResources is content: the state
        // templates' base resources are written only as a game state loads
        // (resource loading and entry construction) and by the state
        // nudger's reload, which the collector observes as a world
        // invalidation, so every hour holds the game's value; add_resource
        // writes the live state pools, not the template.
        TableId::StateBaseResources
        | TableId::BuildingConstruction
        | TableId::RailwayConstruction
        | TableId::ConstructionLineOutput
        | TableId::OccupationSettings
        | TableId::StateOccupationLaws
        | TableId::StateGarrisons
        | TableId::StateAssignedManpower
        | TableId::CountryOccupationReserveManpower
        | TableId::CountryCollaboration => Daily(3),
        TableId::AirStatistics => Daily(5),
        TableId::DecisionCounts | TableId::Decisions | TableId::DecisionRandomItems => Daily(6),
        // Organisations share the lines' slot: a line names its organisation,
        // and the history's units tile with the lines' daily reads. Each
        // line's output totals are published with the census that decides
        // which lines ended and which were created after observation began.
        TableId::ProductionLines
        | TableId::CountryProductionModifiers
        | TableId::ProductionLineModifiers
        | TableId::IndustrialOrganisations
        | TableId::IndustrialOrganisationHistory
        | TableId::IndustrialOrganisationDetails
        | TableId::IndustrialOrganisationPolicies
        | TableId::ProductionLineOutput
        | TableId::RocketProductionOutput
        | TableId::Stockpiles
        | TableId::ProductionLicenses
        | TableId::DeploymentConveyors
        | TableId::DeploymentLines
        | TableId::DeploymentManpower
        | TableId::DeploymentEquipment
        | TableId::ProductionLicenseRecords => Daily(8),
        // DivisionTemplates falls to the hourly arm: counter tooltips, template triggers and the
        // model selectors read a template's names and category, which the update command, template
        // effects and the daily AI category chooser change at any hour. The collector derives it
        // from the hourly icon walk, carrying templates no hooked writer touched
        // (template_changes). CountryResources is hourly: a tactic's resource-count trigger reads
        // its produced and adjustment pools. Resource addition, building-resource modification,
        // country release and building level changes rewrite them at any hour, as does the daily
        // update; a peace conference recomputes them in full. ResourceExchanges is hourly: player
        // trade UI commands, the AI's daily trade commands and import effects create, resize and
        // cancel deals (amount 0). Civilian-factory consumption can remove them. The hourly trade
        // task rewrites deliveries and terminates deals, as resource recalculations do after the
        // changes above. The collector walks the resource census with both every hour, so
        // ResourceDefinitions, content set only as it loads (construction, persistent reading and
        // cost-token loading), is published with them at no further read.
        TableId::ResourceDefinitions => Hourly,
        TableId::ResearchSlots => Daily(11),
        // Theatre details share one daily slot. LandTheatres itself (id,
        // owner, roster ordinal) falls to the hourly arm below: hourly
        // OrdersGroups rows name their theatre and battle plans draw in its
        // roster order, so the roster must come from the same hour.
        TableId::LandTheatreGroups
        | TableId::LandTheatreMembers
        | TableId::AirTheatres
        | TableId::AirTheatreGroups
        | TableId::AirTheatreMembers => Daily(13),
        // The daily network pass writes every network inside the hour-23
        // daily tick: roster, topology, selectors, graph strengths and gains,
        // and the subnetworks it rebuilds there (their strengths, targets,
        // gain factors and quiet flags; the game reaches that rebuild only
        // from the daily tick). An operative's build or recall mission and a
        // state's controller change reach the rows only through that pass.
        // Slot 0, like CountryIntel, observes its first completed state.
        // Outside it, a country's dynamic reuse (civil war, dynamic and
        // peace-created countries) destroys that country's networks; the
        // collector reads the table again at the hour after a forwarded
        // country appearance reset, which that reuse runs first, and after
        // the debug console's grow_intel_network, which sets local strengths.
        TableId::IntelNetworks | TableId::IntelNetworkSummaries => Daily(0),
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
        | TableId::FuelDemandParts
        | TableId::CountryExperience
        | TableId::HubMotorization
        | TableId::DisruptedSupply
        | TableId::ForeignHomebases
        | TableId::PurchaseContracts => Daily(17),
        // TheatreAreas falls to the hourly arm: the volunteer theatre reveal
        // (radar_group::fog_consumer) reads each area, whose membership the
        // controller-area change rewrites on the hourly occupation path;
        // its traversal is StationedUnits' hourly one, so it adds only the
        // area reads.
        TableId::PreferredTactics => Daily(18),
        // Province name overrides (map and battle labels), the raid army
        // search's strategic-region access modes and the world tension
        // tables change at any hour: rename and reset effects; region access
        // mode setting from player and AI commands and navy reset; and world
        // threat total updates, which every source writer (creation from
        // effects and diplomatic actions, updates and the daily ramp) ends in.
        // The collector's world_input_changes hooks those writers, so an hour
        // none of a table's writers ran publishes its held read. A country's
        // dynamic modifiers (has_dynamic_modifier and scripted scopes) change
        // at any hour too: add and remove effects, existing-entry day changes,
        // the daily update, value recalculation with country modifiers and
        // clearing on country appearance reset; a country none of them touched
        // keeps its held rows.
        TableId::ProvinceNames
        | TableId::NavalRegionAccess
        | TableId::WorldTension
        | TableId::ThreatSources
        | TableId::ThreatCountryAggregates
        | TableId::DynamicModifiers => Hourly,
        // Raids stay hourly: RaidTargets is the same walk's map-icon projection.
        TableId::SunkShips => Daily(19),
        // Focus state and the completed roster, which focus_progress and
        // has_completed_focus read: the daily focus update advances progress,
        // paused state and threshold completion for every country owning a
        // state inside the hour entered at 23, so every country owning a state
        // is read at the next hour, and one that gained or lost its last state
        // since the previous boundary. Other writers mark their country, which
        // is read within the hour: selection and cancellation (forwarded by
        // the politics family), continuous focus setting, completion (effects
        // and joint focuses), completed-focus insertion (bypass and unlock),
        // removal (uncompletion effects), tree setting (tree-load effects),
        // reset, copy for dynamic countries and database clear.
        TableId::Focus | TableId::CompletedFocuses => Sliced,
        TableId::OperationTokens => Daily(20),
        // Agency creation and renaming can run in any tick, and the agency display
        // reads the stored created byte and name in the hour.
        TableId::IntelligenceAgencies => Cadence::Hourly,
        // The persistent role and trait tables are hourly: their writers run between daily ticks,
        // and a tactic's character-scope has_trait reads unit-leader traits. The trait array is
        // filled by learning and officer commands, add-trait effects and experience passing a trait
        // threshold during hourly combat. Removal effects erase traits. Country-leader role effects
        // insert into and erase from the character's leader roles; advisor role effects do the same
        // for its advisor roles. Effects run when an event option is chosen. HiredAdvisors falls to
        // the hourly arm: has_idea triggers read the appointed array, and an appointment changes at
        // any hour through insertion from the activate-advisor effect and command, or unappointment
        // from the fire command, effects, retirement and civil wars. Owner decision 2026-10-11:
        // trigger inputs stay exact every hour, made cheap with change hooks; the collector
        // publishes a quiet hour's held walk (advisor_pool_changes).
        TableId::AirAces | TableId::PeaceConferences => Daily(21),
        TableId::IncomingDiplomaticActions
        | TableId::TechSharingGroups
        | TableId::TechSharingMembers
        // Variant census and the rosters that name designs share a slot.
        // The mutation ledger supplies accepted design edits between censuses.
        | TableId::VariantDefinitions
        | TableId::VariantModules
        | TableId::VariantStats
        | TableId::VariantUpgrades
        | TableId::AvailableEquipmentVariants
        // The persistent unit-bonus list is read whole in this slot, with no
        // writer certificate: doctrine, project and effect rewards append to it
        // at any hour, and the record catches up at the next slot.
        | TableId::CountryUnitBonuses => Daily(22),
        // Lend-lease totals grow at deliveries and sinkings; the exchange is
        // deleted, with its totals, when the lend-lease ends.
        TableId::SunkConvoys | TableId::LendLeaseEquipment => Daily(23),
        // Operations and their assigned slots are one walk. Operation
        // creation and cancellation are commands outside the daily tick, and
        // the map icons read the operations and their preparation every hour.
        TableId::Operations | TableId::OperationAssignments => Hourly,
        // RadarCoverage is hourly because a controller change edits radar
        // groups at any hour. Cost over 25 hours on a vanilla map:
        // 0.11 ms p50 / 0.16 ms p95 read on the game thread, a 0.12 ms/h raw
        // copy on the decode worker, 104 B/h encoded.
        // The DLC mask is fixed for the session, but `human_ai` is a debug
        // toggle the console can flip at any hour; both are single reads.
        TableId::SimulationFeatures => Cadence::Hourly,
        // Hourly publication does not schedule a census: writer occurrences
        // drain on the existing observer worker, and shared bonus/context rows
        // carry between demanded design or country-accumulator changes.
        TableId::VariantChanges
        | TableId::VariantBonusSnapshots
        | TableId::DesignCountryModifiers
        | TableId::DesignCountryTemplates => Cadence::Hourly,
        TableId::DesignObserverContext | TableId::DesignOrganisationLiveness => Cadence::Hourly,
        // A tag switch or observer command can run between any two ticks, and
        // each tick's combat manager tests the viewing choice once; one eight-byte read.
        TableId::LocalPlayer => Cadence::Hourly,
        // Cryptology is hourly: activation and scripted progress can change the
        // raid target-intel predicate between daily ticks.
        TableId::Cryptology => Cadence::Hourly,
        // ScriptFlags, ScriptVariables and ActiveIdeas are hourly: the
        // triggers that read them see an effect's write in the same hour,
        // and effects run whenever an event option, decision or focus fires.
        // Only effects, the console, a load and resets write persistent
        // variable stores through scoped mutation and removal; the temporary
        // mutation helper writes only the thread-local store. No engine pass
        // writes them periodically, and the collector carries unwritten stores
        // (variable_changes). Country and character flag effects set, clear
        // and modify flags; idea effects add, remove and swap ideas.
        // Saving and clearing global event targets can write at any hour.
        // Readers see those writes immediately, so daily reads are stale. The
        // viewer evaluates raid visible/show_target every hour, including
        // event-target existence predicates and tag operands.
        TableId::GlobalSavedEventTargets => Cadence::Hourly,
        TableId::EquipmentDestruction => Cadence::Hourly,
        TableId::AirLossEvents | TableId::BuildingBombingDamage | TableId::AirDailyRoster => Cadence::Hourly,
        // Impact events drained at the hourly cut, like the air ledgers.
        TableId::NuclearStrikes => Cadence::Hourly,
        // The map event interval is sealed at every hourly cut.
        TableId::MapEvents => Cadence::Hourly,
        // Owned simulation operations and coverage are drained at the hourly cut.
        TableId::PowerBalanceModifierBreakdowns => Cadence::Hourly,
        TableId::ScriptFlags
        | TableId::ScriptVariables
        | TableId::ActiveIdeas
        | TableId::TimedIdeas => {
            Cadence::Hourly
        }
        // Every other table has an hourly-tick or any-hour native writer, or
        // is a hook log drained each hour (cadence-audit.md).
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

/// Each table whose cadence changed, with the cadence recordings written before
/// the format were read at. Entries for one table are in ascending format. No
/// table present in two proven formats before main's 101 changed cadence; the
/// tables a format introduced are refused in earlier recordings.
pub const EARLIER_CADENCES: [(TableId, u16, Cadence); 25] = {
    use crate::rows::compatibility::CADENCE_FORMAT as F;
    use Cadence::{Daily, Hourly, Sliced};
    [
        (TableId::AceRangeInputs, F, Sliced),
        (TableId::AirRegionDamage, F, Daily(7)),
        (TableId::AirRegionHistory, F, Daily(7)),
        (TableId::AvailableAdvisors, F, Hourly),
        (TableId::CompletedFocuses, F, Daily(19)),
        (TableId::CountryLocalSubunits, F, Daily(2)),
        (TableId::DivisionRetreatBans, F, Sliced),
        (TableId::DivisionTemplates, F, Daily(9)),
        (TableId::DynamicModifiers, F, Daily(15)),
        (TableId::Focus, F, Daily(19)),
        (TableId::HiredAdvisors, F, Daily(21)),
        (TableId::NavalRegionAccess, F, Daily(11)),
        (TableId::NavalSupremacy, F, Daily(23)),
        (TableId::ProjectHistory, F, Daily(4)),
        (TableId::Projects, F, Daily(4)),
        (TableId::ProvinceNames, F, Daily(1)),
        (TableId::RaidCountryInputs, F, Sliced),
        (TableId::ResourceDefinitions, F, Daily(10)),
        (TableId::StockpileArchetypes, F, Sliced),
        (TableId::SupplyNodes, F, Hourly),
        (TableId::TheatreAreas, F, Daily(18)),
        (TableId::ThreatCountryAggregates, F, Daily(18)),
        (TableId::ThreatSources, F, Daily(18)),
        (TableId::TrainInventory, F, Hourly),
        (TableId::WorldTension, F, Daily(18)),
    ]
};

/// The cadence a recording of `format` read `table` at: the earliest change
/// after it, or today's.
pub fn cadence_in(table: TableId, format: crate::format::FormatVersion) -> Cadence {
    EARLIER_CADENCES
        .iter()
        .find(|(changed, until, _)| *changed == table && format.version() < *until)
        .map_or_else(|| cadence(table), |(_, _, earlier)| *earlier)
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

/// Whether the collector reads `table` at `hour`. A retired table is never
/// read: the current format does not record it, so its producers stay idle.
pub fn should_walk(table: TableId, hour: crate::Hour, force_full: bool) -> bool {
    table
        .last_change()
        .present(crate::format::FormatVersion::CURRENT.version())
        && (force_full || is_refresh(table, hour))
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
    fn hourly_writer_audit_covers_every_active_table_and_reconciles_its_totals() {
        use std::collections::BTreeSet;
        let audit = include_str!("../cadence-audit.md");
        let (summary, detail) = audit.split_once("## Tables").unwrap();
        let rows: Vec<_> = detail
            .lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| Table |"))
            .map(|line| {
                let mut columns = line.split('|').skip(1).map(str::trim);
                (
                    columns.next().unwrap(),
                    columns.next().unwrap().split(';').next().unwrap(),
                )
            })
            .collect();
        let recorded: BTreeSet<_> = rows.iter().map(|row| row.0.to_owned()).collect();
        let expected: BTreeSet<_> = TableId::ALL
            .into_iter()
            .filter(|table| !crate::rows::compatibility::RETIRED_TABLES.contains(table))
            .filter(|&table| cadence(table) == Cadence::Hourly)
            .map(|table| format!("{table:?}"))
            .collect();
        assert_eq!(rows.len(), recorded.len(), "duplicate audited table");
        assert!(
            recorded == expected,
            "missing: {:?}; extra: {:?}",
            expected.difference(&recorded).collect::<Vec<_>>(),
            recorded.difference(&expected).collect::<Vec<_>>()
        );
        let declared = |class: &str| {
            summary
                .lines()
                .find_map(|line| {
                    let columns: Vec<_> = line.split('|').map(str::trim).collect();
                    (columns.get(1) == Some(&class)).then(|| columns[3].parse::<usize>().unwrap())
                })
                .unwrap()
        };
        let mut total = 0;
        for class in ["hourly tick", "any hour", "hook log", "load", "recorder"] {
            let count = rows.iter().filter(|row| row.1 == class).count();
            assert_eq!(count, declared(class), "{class}");
            total += count;
        }
        assert_eq!(total, rows.len(), "unknown writer class");
        assert_eq!(total, declared("total"));
    }

    #[test]
    fn doctrine_cadence_audit_covers_each_table_and_reconciles_totals() {
        let audit = include_str!("../cadence-audit.md");
        let (summary, detail) = audit.split_once("## Tables").unwrap();
        let rows: Vec<_> = detail
            .lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| Table |"))
            .collect();
        for table in [
            TableId::FactionAdvisorModifiers,
            TableId::FactionAccounts,
            TableId::FactionGoalStatus,
            TableId::FactionGoalEntries,
            TableId::FactionProjection,
            TableId::FactionResources,
            TableId::DoctrineCountries,
            TableId::DoctrineFolders,
            TableId::DoctrineTracks,
            TableId::DoctrineLeaderMastery,
            TableId::DoctrineCostReductions,
            TableId::DoctrineMasteryGrants,
            TableId::DoctrineEquipmentBonuses,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            let prefix = format!("| {table:?} |");
            assert_eq!(
                rows.iter()
                    .filter(|line| line.starts_with(prefix.as_str()))
                    .count(),
                1,
                "{table:?}"
            );
        }
        let mut total = 0;
        for class in ["hourly tick", "any hour", "hook log", "load", "recorder"] {
            let count = rows
                .iter()
                .filter(|line| {
                    line.split('|')
                        .nth(2)
                        .unwrap()
                        .trim()
                        .split(';')
                        .next()
                        .unwrap()
                        == class
                })
                .count();
            let prefix = format!("| {class} |");
            let declared = summary
                .lines()
                .find(|line| line.starts_with(prefix.as_str()))
                .unwrap()
                .split('|')
                .nth(3)
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            assert_eq!(declared, count, "{class}");
            total += count;
        }
        let declared = summary
            .lines()
            .find(|line| line.starts_with("| total |"))
            .unwrap()
            .split('|')
            .nth(3)
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        assert_eq!(total, rows.len());
        assert_eq!(declared, total);
    }

    #[test]
    fn hourly_cadence_audit_covers_active_tables_and_reconciles_totals() {
        use std::collections::{BTreeMap, BTreeSet};
        let audit = include_str!("../cadence-audit.md");
        let (summary, inventory) = audit
            .split_once("| Table | Writers | Writer evidence |")
            .expect("hourly table inventory");
        let expected: BTreeSet<_> = TableId::ALL
            .iter()
            .filter(|&&table| cadence(table) == Cadence::Hourly && should_walk(table, 0, true))
            .map(|table| format!("{table:?}"))
            .collect();
        let mut listed = BTreeSet::new();
        let mut counts = BTreeMap::new();
        for row in inventory.lines().filter(|line| line.starts_with("| ")) {
            let columns: Vec<_> = row.split('|').map(str::trim).collect();
            let name = columns[1];
            assert!(
                expected.contains(name),
                "non-hourly or unknown table {name}"
            );
            assert!(listed.insert(name.to_owned()), "duplicate table {name}");
            assert!(!columns[3].is_empty(), "missing writer evidence for {name}");
            let class = columns[2].split(';').next().unwrap();
            assert!(
                ["hourly tick", "any hour", "hook log", "load", "recorder"].contains(&class),
                "unknown writer class for {name}: {class}"
            );
            *counts.entry(class).or_insert(0_usize) += 1;
        }
        assert_eq!(
            listed, expected,
            "audit must cover every active hourly table"
        );
        counts.insert("total", listed.len());
        for (class, count) in counts {
            let prefix = format!("| {class} |");
            let row = summary
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap();
            let recorded = row
                .split('|')
                .nth(3)
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            assert_eq!(recorded, count, "stale {class} audit total");
        }
    }

    #[test]
    fn retired_tables_are_never_walked() {
        for table in crate::rows::compatibility::RETIRED_TABLES {
            for hour in 0..STAGGER_PERIOD {
                assert!(!should_walk(table, hour, true), "{table:?}");
                assert!(!should_walk(table, hour, false), "{table:?}");
            }
        }
        // The walks they shared keep their own tables.
        assert!(should_walk(TableId::UnitLeaderTraits, 0, false));
        assert!(should_walk(TableId::LandCombatDivisions, 0, false));
        assert!(should_walk(TableId::NavalCombatShips, 0, false));
        assert!(should_walk(TableId::AirStatistics, 0, true));
    }

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
                assert_eq!(
                    should_walk(table, base, true),
                    table
                        .last_change()
                        .present(crate::format::FormatVersion::CURRENT.version()),
                    "{table:?}"
                );
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
            TableId::LandTheatres,
            TableId::FactionTheatres,
            TableId::RadarCoverage,
            TableId::NavalRegionalMines,
            TableId::NavalAccidentReports,
            TableId::RelationLinks,
            // A template edit changes its counters' icons at any hour.
            TableId::DivisionTemplateIcons,
            TableId::CountryVisibility,
            TableId::StateVisibility,
            TableId::ShipModels,
            // What the supply map mode and railways draw; the decorative
            // trains' nodes and inventory are daily slices.
            TableId::ProvinceSupply,
            // A volunteer theatre's areas, which its fog reveal reads.
            TableId::TheatreAreas,
            TableId::WeatherProvinces,
            TableId::LandCombatDivisions,
            TableId::LandCombatSides,
            TableId::AirBaseRegistrations,
            // The land counter's ability grid: abilities start and change phase at any hour.
            TableId::ActiveAbilities,
            // Bombing changes a building's mesh and fire within the hour.
            TableId::StateBuildings,
            TableId::ProvinceBuildings,
            TableId::RaidTargetStatuses,
            // Raid launch modifiers, autonomy and convoy reservations, the
            // aces' range terms and the essential stock.
            TableId::RaidCountryInputs,
            TableId::AceRangeInputs,
            TableId::StockpileArchetypes,
            // Commander retreat bans the selected-route search reads.
            TableId::DivisionRetreatBans,
            // Roles, traits and scientist rosters whose writers run between
            // daily ticks; each trait and roster walk moves whole.
            TableId::CountryLeaderRoles,
            TableId::AdvisorRoles,
            TableId::UnitLeaderTraits,
            TableId::Scientists,
            TableId::ScientistSkills,
            // The operations the map icons read, created and cancelled by
            // commands at any hour.
            TableId::Operations,
            TableId::OperationAssignments,
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
            // The 3D model selectors' local sub-unit clones, rebuilt by
            // research and doctrine unlocks at any hour.
            TableId::CountryLocalSubunits,
            // Template names and categories, edited at any hour.
            TableId::DivisionTemplates,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert_eq!(refresh_text(table), "hourly");
        }
        // The appointed-advisor list, which has_idea reads, is hourly with the
        // advisor roles it names; its daily neighbours keep their slot.
        assert_eq!(cadence(TableId::HiredAdvisors), Cadence::Hourly);
        assert_eq!(refresh_slot(TableId::AirAces), Some(21));
        for family in [
            &[
                TableId::ProjectPrograms,
                TableId::ProjectBreakthroughs,
                TableId::ScientistDaily,
            ][..],
            &[TableId::Projects, TableId::ProjectHistory][..],
            &[TableId::RelationOpinions, TableId::RelationTuples][..],
            &[
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantStats,
                TableId::AvailableEquipmentVariants,
                TableId::CountryUnitBonuses,
            ],
            &[TableId::DivisionEquipment, TableId::ArmyUpgradeGroups],
            &[
                TableId::ProductionLines,
                TableId::CountryProductionModifiers,
                TableId::ProductionLineModifiers,
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationHistory,
                TableId::IndustrialOrganisationDetails,
                TableId::IndustrialOrganisationPolicies,
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
        assert_eq!(refresh_slot(TableId::ProjectPrograms), Some(4));
        assert_eq!(cadence(TableId::Projects), Cadence::Sliced);
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
            TableId::RaidAirSites,
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

    /// Province names, region access modes, world tension and dynamic
    /// modifiers change at any hour; they were daily (slots 1, 11, 18 and
    /// 15), up to a day stale.
    #[test]
    fn world_inputs_are_read_every_hour() {
        for table in [
            TableId::ProvinceNames,
            TableId::NavalRegionAccess,
            TableId::WorldTension,
            TableId::ThreatSources,
            TableId::ThreatCountryAggregates,
            TableId::DynamicModifiers,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert!((-24..24).all(|hour| should_walk(table, hour, false)));
        }
        // Their former slot neighbours keep their slots.
        assert_eq!(refresh_slot(TableId::ResearchSlots), Some(11));
        assert_eq!(refresh_slot(TableId::PreferredTactics), Some(18));
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

    /// Owner decision 2026-10-11: the decorative supply trains and the
    /// panel-only advisor pools refresh once a game day; they were Hourly.
    #[test]
    fn supply_trains_and_advisor_pools_read_each_owner_once_a_day() {
        for table in [
            TableId::SupplyNodes,
            TableId::TrainInventory,
            TableId::AvailableAdvisors,
        ] {
            assert_eq!(cadence(table), Cadence::Sliced, "{table:?}");
            assert!(is_sliced(table) && !is_staggered(table), "{table:?}");
            assert_eq!(refresh_slot(table), None);
            for base in [0, 60_827_795, -48] {
                // Walked every hour, for that hour's owner slice.
                assert!((base..base + 24).all(|hour| should_walk(table, hour, false)));
            }
        }
        // The owner partition is shared by every sliced table.
        for base in [0, 60_827_795, -48] {
            // 5,000 countries: each owner is read once per day, and a full
            // hour reads every owner.
            let mut reads = vec![0_u32; 5_001];
            for hour in base..base + 24 {
                for owner in 1..=5_000_u32 {
                    if slice_active(owner, hour, false) {
                        reads[owner as usize] += 1;
                    }
                    assert!(slice_active(owner, hour, true));
                }
            }
            assert!(reads[1..].iter().all(|&count| count == 1), "near {base}");
        }
        // What triggers and the map read from the same walks stays hourly.
        for table in [TableId::AdvisorTokens, TableId::ProvinceSupply] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
        }
    }

    #[test]
    fn a_recording_reads_each_table_at_the_cadence_of_its_format() {
        use crate::format::FormatVersion;
        let previous = FormatVersion::read(crate::rows::compatibility::CADENCE_FORMAT - 1).unwrap();
        assert_eq!(
            cadence_in(TableId::WorldTension, previous),
            Cadence::Daily(18)
        );
        assert_eq!(cadence_in(TableId::Projects, previous), Cadence::Daily(4));
        assert_eq!(
            cadence_in(TableId::NavalSupremacy, previous),
            Cadence::Daily(23)
        );
        assert_eq!(
            cadence_in(TableId::Countries, previous),
            cadence(TableId::Countries)
        );
        for table in TableId::ALL {
            assert_eq!(
                cadence_in(table, FormatVersion::CURRENT),
                cadence(table),
                "{table:?}"
            );
        }
        // Each entry records a real change, and one table's entries ascend.
        for (index, &(table, until, earlier)) in EARLIER_CADENCES.iter().enumerate() {
            let next = EARLIER_CADENCES[index + 1..]
                .iter()
                .find(|(other, _, _)| *other == table);
            assert!(next.is_none_or(|&(_, later, _)| later > until), "{table:?}");
            let after = next.map_or_else(|| cadence(table), |&(_, _, cadence)| cadence);
            assert_ne!(earlier, after, "{table:?}");
        }
    }

    #[test]
    fn event_ledger_tables_drain_each_hour() {
        for table in [
            TableId::AirLossEvents,
            TableId::BuildingBombingDamage,
            TableId::AirDailyRoster,
            TableId::NuclearStrikes,
            TableId::MapEvents,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            assert_eq!(refresh_slot(table), None);
            assert!((-48..48).all(|hour| should_walk(table, hour, false)));
        }
    }

    #[test]
    fn occupation_province_hover_is_acquired_each_hour() {
        assert_eq!(cadence(TableId::StateResistance), Cadence::Hourly);
        assert!((-48..48).all(|hour| should_walk(TableId::StateResistance, hour, false)));
        assert_eq!(cadence(TableId::StateGarrisons), Cadence::Daily(3));
    }
    #[test]
    fn owner_daily_progress_exception_is_limited_to_approved_items() {
        assert_eq!(
            cadence(TableId::UnitLeaderTraitExperience),
            Cadence::Daily(12)
        );
        assert_eq!(COMMANDER_PROGRESS_SLOT, COMBAT_LOG_NUMERIC_SLOT);
        assert_eq!(COMMANDER_PROGRESS_SLOT, DIVISION_STATUS_DAILY_SLOT);
        for hour in -48..48 {
            assert_eq!(
                should_walk(TableId::UnitLeaderTraitExperience, hour, false),
                hour.rem_euclid(24) == 12
            );
            assert!(should_walk(TableId::UnitLeaderTraitExperience, hour, true));
            assert_eq!(
                Cadence::Daily(DIVISION_STATUS_DAILY_SLOT).is_refresh(hour),
                hour.rem_euclid(24) == 12
            );
            assert!(should_walk(TableId::DivisionStatus, hour, false));
        }
        for table in [
            TableId::UnitLeaders,
            TableId::CombatLogObservers,
            TableId::IntelContributions,
            TableId::FactionAccounts,
            TableId::WarParticipation,
            TableId::States,
            TableId::TaskForceCounterInputs,
            TableId::AirRegionCombat,
            TableId::DivisionStatus,
            TableId::DivisionLandInputs,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
        }
    }

    #[test]
    fn combat_log_hourly_tables_have_writer_certificate_audit_entries() {
        let audit = include_str!("../cadence-audit.md");
        for table in [
            TableId::CombatLogGroups,
            TableId::CombatLogMonths,
            TableId::CombatLogManpower,
            TableId::CombatLogTemplates,
            TableId::CombatLogReferences,
            TableId::CombatLogBattles,
            TableId::CombatLogEnemyEquipment,
            TableId::CombatLogBattleSides,
            TableId::CombatLogBattleEquipment,
            TableId::CombatLogBattleModifiers,
            TableId::CombatLogBattleCountries,
            TableId::CombatLogObservers,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            assert!(
                audit
                    .lines()
                    .any(|line| line.starts_with(&format!("| {table:?} |"))),
                "{table:?} has no audit evidence"
            );
        }
        let rows = audit
            .lines()
            .filter(|line| {
                line.split('|').nth(2).map(str::trim).is_some_and(|writer| {
                    ["hourly tick", "any hour", "hook log", "load", "recorder"]
                        .iter()
                        .any(|class| {
                            writer == *class
                                || writer
                                    .strip_prefix(*class)
                                    .is_some_and(|rest| rest.starts_with(';'))
                        })
                })
            })
            .count();
        assert!(audit.contains(&format!("| total | | {rows} |")));
    }
}
