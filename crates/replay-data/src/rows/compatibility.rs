//! Proven field, wire and meaning history; no translation or old decoders.
use super::TableId;
#[cfg(test)]
use crate::FORMAT_VERSION;

/// Includes compact scalar/name/icon edits in VariantChanges.scalar.
pub const DESIGN_STATE_FORMAT: u16 = 104;

/// Unused research grants and project/scientist detail operands.
pub const RESEARCH_PROJECT_DETAILS_FORMAT: u16 = 104;
/// Earlier history has not been audited for today's readers.
pub const PROVEN_FROM: u16 = 91;
/// Occupation hover suffix, including per-key total availability.
pub const OCCUPATION_HOVER_FORMAT: u16 = 104;
/// Stored occupation marker admission and active modifier identities.
pub const OCCUPATION_MAP_FORMAT: u16 = 105;
/// Native retained Combat Log history.
pub const COMBAT_LOG_FORMAT: u16 = 104;
/// EventFeedHistory's interval batch uses exact per-field numeric streams,
/// row-implied generation, sequence anchor/deltas and read-hour-relative dates.
pub const EVENT_FEED_COLUMNS_FORMAT: u16 = 106;
/// Stable semantic observer occurrences and independent signed numeric lanes.
pub const COMBAT_LOG_OBSERVER_KEYS_FORMAT: u16 = 106;
/// Private format 95 was retired when #343 merged main's format 96; its
/// divergent table contracts were never published (PLAN format ledger).
pub const RETIRED_FORMAT: u16 = 95;
/// Independent intel sources, network auxiliary occurrences and UI estimate
/// holders. Estimate keys include the signed category bits; caller full-info
/// is a payload operand. This branch is assigned final numbers at integration.
pub const INTEL_CACHES_FORMAT: u16 = 104;
/// Provisional introduction format for the batched recorder inputs.
pub const RECORDER_INPUTS_FORMAT: u16 = 98;
/// Provisional introduction format for the second recorder-input batch.
pub const EFFECT_INPUTS_FORMAT: u16 = 99;
// Provisional until all widget capture families are integrated.
pub const WIDGET_CAPTURE_FORMAT: u16 = 100;
/// Provisional introduction format; central allocation is reconciled at merge.
pub const PRODUCTION_MODIFIERS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction format for applied fuel and logistics operands.
pub const ECONOMY_INPUTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional main+1 construction outcome suffix; parent finalizes the shared landing.
pub const CONSTRUCTION_COMPLETION_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional append after the accepted published recorder-input prefix.
pub const OCCUPATION_GROUP_LAWS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of ace presentation and live end-screen operands.
pub const GAME_SCORE_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional Intel suffix and observation format; renumber in the parent union.
pub const INTEL_INPUTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of the recorded native war-overview operands.
pub const WAR_OVERVIEW_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of captured peace-settlement and resident bid rows.
pub const PEACE_SETTLEMENTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction for observed air loss and applied building damage events.
pub const AIR_LEDGER_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Main's political integration: runtime, selected-route and technology
/// occurrence inputs, with the provisional suffixes reconciled beside them.
pub const POLITICAL_FORMAT: u16 = 101;
/// Provisional main+1: packets on hourly host rows stop storing their read
/// hour ([`crate::rows::PacketHour`]). Older nested records keep their stamp
/// and stay readable, so no table is refused.
pub const PACKET_HOUR_FORMAT: u16 = 102;
/// Provisional main+1: the reserved singleton's transport packet was split
/// into keyed hourly tables ([`super::RESERVED_SPLIT_TABLES`]), which retired
/// in the same provisional format; no published build wrote them.
pub const RESERVED_SPLIT_FORMAT: u16 = 102;
/// Provisional main+1: per-division speed graphs and fresh counter-route
/// operands, the movement inputs only arrival times used, attrition modifier
/// breakdowns, selected-attack operands and the selected transport and
/// selected-route previews (the reserved transport packet, owner transport
/// speeds, convoy pools, unit transport and selection operands, selected
/// areas and carrier transport plane sums) stop
/// being recorded, and DivisionLandInputs stops copying the supply operands
/// DivisionStatus records. Hover-only breakdowns and operands, oracle-only
/// values and values playback derives exactly from other recorded fields stop
/// being recorded too. Their tables retire and their row fields become
/// [`super::Retired`], [`super::RetiredI64`] or sibling fixed-width
/// tombstones ([`RETIRED_FIELDS`]); older recordings discard them by name.
/// The selected-attack operands' fields instead decode older payloads again
/// ([`ATTACK_INPUTS_FORMAT`]).
pub const RETIRED_INPUTS_FORMAT: u16 = 102;
/// Provisional main+1: each moving division's stat-block base speed, from
/// which playback estimates arrival times.
pub const BASE_SPEED_FORMAT: u16 = 102;
/// Provisional main+1: DivisionStatus records the camouflage category selected from a
/// division's acclimatization (field 27, `camouflage`) instead of every entry and the
/// maximum, which current writers leave empty. Format 101 and earlier keep their entries,
/// from which consumers still decide. Format-102 captures written before this change
/// share the version number but not the field count, so their blocks read as malformed.
pub const CAMOUFLAGE_FORMAT: u16 = 102;
/// Provisional main+1: drifting values the game displays are stored at its
/// display precision ([`crate::display`]): each stepped column holds whole
/// multiples of its step and its wire holds the value divided by the step.
/// Field counts and widths are unchanged, so no table is refused: format 101
/// and earlier decode those columns exact and unscaled. Format-102 captures
/// written before this change share the version number and hold exact values
/// in those columns, which this reader would multiply by the step; no
/// published build wrote them.
pub const DISPLAY_PRECISION_FORMAT: u16 = 102;
/// Provisional main+1: CountryFuel stores the day of applied fuel updates
/// before a read summed as the Fuel widget shows it (`award_day`, field 22)
/// and retires the update-by-update window (`awards`, field 20). Older
/// recordings discard their windows by name and show no applied-update
/// totals. Format-102 captures written before this change share the version
/// number but not CountryFuel's field count, so their blocks read as
/// malformed; no published build wrote them.
pub const FUEL_AWARD_DAY_FORMAT: u16 = 102;
/// Provisional main+1: LandCombatDetails appends each side's CAS and
/// commander hover inputs (field 10, `side_inputs`,
/// [`super::LandCombatSideInputs`]): the anti-air cache, prevented CAS damage,
/// skill progress, offence and defence, description token, timed traits and
/// shown learning traits, at their display precision, in a field that
/// re-encodes only when they change. The side layout (field 7) keeps the
/// anti-air cache and prevented lanes as retired parts that current writers
/// leave empty; formats 101 and earlier hold both there, exact, and still
/// answer the CAS hover with them
/// ([`super::LandCombatDetailsRow::cas_inputs`]), while their other hover
/// parts stay discarded and the field reads as not recorded. The skill
/// hover's leader block and the shown learning rows are optional: `None` when
/// their read failed, the progress its unread word. Format-102 captures
/// written before this change, or before those two parts became optional,
/// share the version number but not LandCombatDetails' layout, so their
/// blocks read as malformed; no published build wrote them.
pub const SIDE_INPUTS_FORMAT: u16 = 102;

/// Provisional main+1: each LandCombatDetails side (field 7,
/// [`super::LandCombatSideDetails`]) stores the sum of its 24 organisation loss samples,
/// one i64, where earlier layouts store the samples (`organisation_loss_sum`): the
/// marker's survival estimate is their only displayed reader, and it takes only the sum.
/// The side marks the layout by storing its committed division count inverted. Field
/// counts are unchanged, so no table is refused: format 101 and earlier decode the
/// samples and sum them, and format-102 captures written before this change, which share
/// the version number, are told apart by their committed count (below 2^31, where an
/// inverted one is above) and decode the same way. A reader built before this change
/// refuses the whole block holding a marked side with the payload error `nested
/// collection length exceeds remaining payload`; it never misreads it. Main+1 is never
/// released and is renumbered at merge, so only in-flight tools predate the change.
pub const LOSS_SUM_FORMAT: u16 = 102;
/// Provisional main+1: LandCombatDivisions records the stat block's initiative again
/// (field 16, which format 101 holds with the same meaning) and appends each reserve's
/// reinforcement speed share (`reinforcement_speed`, field 30), which the collector
/// composes from the native speed-share operands; the reinforcement operand packets
/// (fields 22 to 26) stay retired. Older recordings hold no share, so their reserve
/// chance arm stays unknown. Format-102 captures written before this change share the
/// version number but not LandCombatDivisions' field count, so their blocks read as
/// malformed; no published build wrote them.
pub const RESERVE_SPEED_FORMAT: u16 = 102;
/// Provisional main+1: LandCombatDivisions stores fuel and capacity, the
/// five stats and the four scales as a numeric field each (fields 31 to
/// 41, [`super::LandCombatDivisionRow::fuel_inputs`],
/// [`super::LandCombatDivisionRow::stat_values`] and
/// [`super::LandCombatDivisionRow::scales`]), so the grouped layout predicts
/// each from its own changes instead of rewriting a packet when one drifts.
/// The packets (fields 10, 11 and 18) are tombstones current writers leave
/// empty ([`RETIRED_FIELDS`]); format 101 and earlier hold the values there,
/// exact, and still answer with them. Format-102 captures written before
/// this change share the version number but not LandCombatDivisions' field
/// count, so their blocks read as malformed; no published build wrote them.
pub const LAND_DIVISION_NUMBERS_FORMAT: u16 = 102;
/// Provisional main+1: AirRegionCombat records each retained regional air-combat holder (the
/// strategic air's region-combat table) with its records and their damage together, read when the
/// record updater touched it and carried otherwise; playback derives the daily pruning. The retired
/// AirRegionHistory and AirRegionDamage tables (a whole-table daily walk without a parent link from
/// damage to record) stay retired.
pub const AIR_REGION_COMBAT_FORMAT: u16 = 102;

/// Provisional main+1: Politics appends what the `has_elections` trigger tests
/// (`has_elections`, field 6): the elections byte nonzero and the frequency positive, as
/// the trigger reads them. The retired `election_frequency` and `elections_allowed`
/// (fields 1 and 2) stay tombstones. Older recordings hold no value, so the trigger stays
/// refused by name for them. Format-102 captures written before this change share the
/// version number but not Politics' field count, so their blocks read as malformed; no
/// published build wrote them.
pub const HAS_ELECTIONS_FORMAT: u16 = 102;

/// Provisional main+1: the selected army's attack list is derived on playback
/// during playback from the inputs it reads that nothing else records:
/// DivisionStatus gains each division's stat-block operands (field 28, `attack_stats`)
/// and acclimatization factor (29, `weather_factor`), OrdersGroupCounterInputs the direct
/// commander's operands (6, `commander_attack`), CountryCounterInputs the raw night and
/// bunker modifier entries (19, `attack_modifiers`), Provinces the state's bunker factor
/// for a fortified province's controller (9, `fort_bunker`) and WeatherRegions the raw
/// weather attack and breakthrough entries (8, `combat`). The native operands format 101
/// recorded in each table's `attack` field decode again ([`super::Witnessed`]) and answer
/// for those recordings; current writers leave them empty. Format-102 captures written
/// before this change share the version number but not these tables' field counts, so
/// their blocks read as malformed.
pub const ATTACK_INPUTS_FORMAT: u16 = 102;
/// Provisional main+1: the movement-speed and attrition terms arrival times and the
/// attrition hover read: the stat block's and commander's movement words, province and
/// region weather speed and attrition entries, out-of-supply days, fuel, the exact supply
/// of a division with days out of supply, the selected commander block's entries, the
/// country's movement entries, the regional air superiority entry's movement impact and
/// the loaded movement defines. WeatherRegions.combat ([`super::RegionCombatWeather`])
/// gained its army speed and attack speed entries inside the existing field.
/// WeatherProvinces.modifiers ([`super::ProvinceWeatherModifiers`]) holds the province
/// record's whole modifier accumulator, every key; captures of this version that hold only
/// the movement and attrition keys share the version but not the field's layout, so their
/// blocks read as malformed.
pub const MOVEMENT_TERMS_FORMAT: u16 = 102;

/// Provisional main+1: DivisionStatus.supply (field 22), the division supply
/// footer, is recorded again for every division at display precision
/// ([`super::DivisionSupplyFooter`], owner decision 2026-10-11) with the
/// wire layout formats 99 to 101 recorded exact, which they still decode
/// unscaled. Format-102 recordings written while the field was a provisional
/// tombstone hold its empty payload, which reads as not recorded
/// (`opaque_value!(T, restored)`); format-102 captures written before that
/// retirement hold exact amounts this reader multiplies by the step. No
/// published build wrote either.
pub const SUPPLY_FOOTER_FORMAT: u16 = 102;

/// Provisional main+1: DivisionStatus.supply ([`super::DivisionSupplyFooter`])
/// stores its received amount as the shortfall from the asked amount, and
/// each origin's received amount as the shortfall from its requested amount,
/// in whole display steps (wrapping), and tags its origins list `2` or `3`
/// instead of `0` or `1` to say so. A supplied division's received words then
/// repeat while its asked and requested words drift. Field counts and widths
/// are unchanged, so no table is refused: earlier recordings decode the
/// amounts themselves, and format-102 captures written before this change,
/// which share the version number, are told apart by the origins tag and
/// decode exactly. A reader built before this change refuses the whole block
/// holding a marked footer with the payload error `invalid nested option
/// tag`; it never misreads it. Main+1 is never released and is renumbered at
/// merge, so only in-flight tools predate the change.
pub const SUPPLY_SHORTFALL_FORMAT: u16 = 102;

/// Provisional main+1: the task force and railway gun supply footers
/// (TaskForceCounterInputs.supply, field 11, and RailwayGunCounterInputs.supply,
/// [`super::UnitSupplyInputs`]) are stored at the footer's display step, as
/// DivisionStatus.supply is: the same formatter prints every unit's footer. Field counts
/// and widths are unchanged, so no table is refused: format 101 and earlier decode those
/// amounts exact and unscaled. Format-102 captures written before this change share the
/// version number and hold exact amounts, which this reader multiplies by the step; no
/// published build wrote them.
pub const UNIT_SUPPLY_FOOTER_FORMAT: u16 = 102;

/// Provisional main+1: DivisionStatus.supply_grace_remaining (field 19) is
/// stored at the whole percent of the supply grace share the game shows
/// ([`super::DivisionStatusRow::displayed_grace`]) instead of every 0.01
/// hour. The wire and its step are unchanged and every consumer reads the
/// same share, so no table is refused and nothing decodes differently.
/// Format-102 captures written before this change share the version number
/// and hold the finer value, which the save comparison reports as differing;
/// no published build wrote them.
pub const GRACE_PERCENT_FORMAT: u16 = 102;

/// Provisional main+1: the naval hovers' source trees, keyed by recorded
/// source identities and carried while no hooked writer touches a country or
/// commander tree (NavalModifierTrees), and their diagnostic oracle samples
/// (NavalModifierTreeSamples). Older recordings refuse both by name; the
/// retired NavalModifierBreakdowns stays refused.
pub const NAVAL_TREES_FORMAT: u16 = 102;

/// Provisional main+1: Provinces rows label each province's controller and
/// owner areas by their smallest member and hold each controller area's
/// representative and ordered contacts on its label province's row (fields
/// 10-12, `controller_area`, `owner_area`, `area_node`), which the collector
/// reads again only where a controller change can have touched them. Format
/// 101 and earlier lack them, and the selected-route search refuses by name
/// there. Format-102 captures written before this change share the version
/// number but not Provinces' field count, so their blocks read as malformed;
/// no published build wrote them.
pub const AREA_FORMAT: u16 = 102;

/// Provisional main+1: DivisionRetreatBans holds every division's commander
/// bans at the frame's hour instead of each roster owner's daily read, which
/// the selected-route search accepted only at that owner's read hour. The
/// layout is unchanged; older recordings refuse the table by name.
/// Format-102 captures written before this change share the version number
/// and hold daily reads; no published build wrote them.
pub const RETREAT_BANS_FORMAT: u16 = 102;

/// Provisional main+1 (owner decision 2026-10-11): StockpileArchetypes holds,
/// every hour, only the archetypes a raid type lists as essential equipment,
/// in whole units capped at the largest amount any raid type requires
/// (`units`, I32), instead of each owner's daily read of every archetype
/// total (`quantity_raw`, I64). Older recordings refuse the table by name.
/// Format-102 captures written before this change share the version number
/// but not the field's width, so their blocks read as malformed; no
/// published build wrote them.
pub const ESSENTIAL_STOCK_FORMAT: u16 = 102;

/// Provisional main+1: EventFeedHistory events append what caused them
/// (`EventFeedEvent.cause`, [`crate::event_feed_rows::EventFeedCause`]): the
/// native operation each happened in, named by its caller, the operation
/// that one ran inside, and the operands the game passed. The feed also
/// records ended war relations, releases, state owner changes and peace
/// conferences (`EventFeedSource` 4 to 7). Format 101 and earlier events
/// decode with their cause not recorded. Format-102 captures written before
/// this change share the version number but not the event layout, so their
/// blocks read as malformed; no published build wrote them.
pub const EVENT_CAUSE_FORMAT: u16 = 102;
/// Script occurrence identities and their owned token and scope arenas.
pub const SCRIPT_EVENT_FORMAT: u16 = 104;

/// Provisional main+1: the tables in [`crate::cadence::EARLIER_CADENCES`]
/// changed cadence (most daily tables read within the hour, a few sliced or
/// moved to another slot). Recordings before it were written with their
/// earlier cadence, which [`crate::cadence::cadence_in`] returns.
pub const CADENCE_FORMAT: u16 = 102;

/// Provisional main+1: NuclearStrikes records each nuclear impact with its
/// launcher, type, target and the outcome the impact applied before the
/// authored `on_nuke_drop` effects ([`crate::rows::NuclearStrikeRow`]).
/// Older recordings refuse the table by name.
pub const NUCLEAR_STRIKES_FORMAT: u16 = 102;

/// Provisional main+1: LandCombatDivisionModifiers holds each combat
/// member's ordered modifier entries, a row a member keyed (combat, side,
/// ordinal) as LandCombatDivisions, so an hour that changes one division's
/// entries stores only that row. LandCombatModifiers keeps a row a side with
/// its border values; its list of every member's entries is a retired
/// tombstone ([`RETIRED_FIELDS`]), so older recordings keep its framing and
/// border values and refuse only LandCombatDivisionModifiers, by name.
/// Format-102 captures written before this change share the version number
/// but not the table set, so their blocks read as malformed; no published
/// build wrote them.
pub const LAND_MODIFIER_ROWS_FORMAT: u16 = 102;

/// Provisional main+1: FuelDemandParts holds each element of a CountryFuel
/// row's demand pack lists, a row a part keyed by country, list and identity,
/// so a read that changes one division's rate stores only that row; the pack
/// in CountryFuel keeps its header. The pack's hour is a tag
/// ([`crate::rows::FuelDemandHour`]): a pack read with its row, as every
/// current pack is, stores no hour, so a daily read of an unchanged pack
/// stores no change. Older recordings keep their packs whole, hours
/// included, and refuse only FuelDemandParts, by name. Format-102 captures
/// written before this change share the version number but not the table
/// set or the pack's layout, so their blocks read as malformed; no
/// published build wrote them.
pub const FUEL_DEMAND_PARTS_FORMAT: u16 = 102;
/// Map-mode inputs: each state's population (States `population`), its
/// claims (StateCores `claims`) and each country's owned war goals'
/// recipients (Capitulation `war_goal_recipients`). Format-102 captures
/// written before them hold none of these fields.
pub const MAP_MODE_INPUTS_FORMAT: u16 = 102;
/// Provisional main+1: IntelNetworks rows carry each state's daily gain and
/// its two parts, the network's quiet byte for the row's selector and the
/// selected subnetwork's gain factor, strength target, the target's two parts
/// and quiet byte (fields 3 to 11), which the Operatives map mode reads.
/// Format 101 and earlier refuse the table by name. Format-102 captures
/// written before this change share the version number but not the table's
/// field count, so their blocks read as malformed; no published build wrote
/// them.
pub const INTEL_NETWORK_MAP_FORMAT: u16 = 102;
/// Each state's live category (States `category`). Format-102 captures
/// written before it hold none.
pub const STATE_CATEGORY_FORMAT: u16 = 102;
/// Map icons: each state's strategic locations and its provinces' own
/// (StateCores `strategic_locations` and `province_strategic_locations`).
/// Format-102 captures written before them hold neither field.
pub const STRATEGIC_LOCATIONS_FORMAT: u16 = 102;

/// Provisional main+1: MapEvents records the simulation edges behind the
/// game's floating map text and critical shore bombardment explosions as an
/// hourly interval with each family's coverage ([`crate::rows::MapEventsRow`]).
/// Older recordings refuse the table by name.
pub const MAP_EVENTS_FORMAT: u16 = 102;
/// Scripted-map cached state choices and physical array extent, plus the
/// eighth MapEvents family and its expanded interval health layout.
pub const SCRIPTED_MAP_FORMAT: u16 = 104;

/// Provisional main+1: OwnerTransportSpeeds holds each railway-gun owner's
/// transport speed in the strategic regions its guns stand in or route
/// through, which a gun's sea-crossing and transfer legs move at. Older
/// recordings refuse only that table, by name, and their railway-gun sea legs
/// refuse their duration. Format-102 captures
/// written before this change share the version number but not the table
/// set, so their blocks read as malformed; no published build wrote them.
pub const OWNER_TRANSPORT_SPEEDS_FORMAT: u16 = 102;

/// Provisional main+1: ReserveModifierTrees holds the printed reinforcement-rate
/// trees of the hour's reserves' countries and contextual commanders, which the
/// reserve chance arm prints. Older recordings refuse only that table, by name,
/// and their chance arm omits those fragments by name. Format-102 captures
/// written before this change share the version number but not the table set,
/// so their blocks read as malformed; no published build wrote them.
pub const RESERVE_MODIFIER_TREES_FORMAT: u16 = 102;

/// Provisional main+1: BattleSounds holds the battle ambience's battle list as
/// the game holds it at each hourly boundary ([`crate::rows::BattleSoundRow`]).
/// Older recordings refuse only that table, by name, and play no battle
/// ambience. Format-102 captures written before this change share the version
/// number but not the table set, so their blocks read as malformed; no
/// published build wrote them.
pub const BATTLE_SOUNDS_FORMAT: u16 = 102;
/// Provisional main+1: OperativeMissions holds every agency's active
/// operatives hourly with what the Operatives map's icons read: assignment and
/// cooldown, the active and pending missions with their targets, and the
/// leader's own portrait. Older recordings refuse only that table, by name.
/// Format-102 captures written before this change share the version number
/// but not the table set, so their blocks read as malformed; no published
/// build wrote them.
pub const OPERATIVE_MISSIONS_FORMAT: u16 = 102;

/// Provisional main+1: StrategicOperatives holds each country's strategic
/// operative hourly with what the Operatives map's icons read: the mission
/// and operation activity against the country and the control-trade
/// influence and diplomatic pressure other countries hold over it. Older
/// recordings refuse only that table, by name. Format-102 captures written
/// before this change share the version number but not the table set, so
/// their blocks read as malformed; no published build wrote them.
pub const STRATEGIC_OPERATIVES_FORMAT: u16 = 102;

/// Current-format client agency UI buffer membership and native bucket order.
pub const AGENCY_OPERATION_BUFFERS_FORMAT: u16 = 102;

/// Provisional main+1: Operations rows carry the selected phases, the
/// requirement records and the inline equipment pool (fields 16 to 20),
/// which the map icons' preparation display reads, and the table is read
/// every hour. Older recordings refuse the table by name. Format-102
/// captures written before this change share the version number but not the
/// table's field count, so their blocks read as malformed; no published
/// build wrote them.
pub const OPERATION_PREPARATION_FORMAT: u16 = 102;
/// Provisional main+1: the resources map mode's inputs. ResourceOrigins, ResourceRights,
/// CountryOilFactors and StateTemporaryResources hold what each state's resource icon reads
/// beyond the raid state tables; ConvoyClients, TradeRoutes and RegionalConvoyEfficiency hold
/// every trade-route line and its hotspots.
/// Earlier recordings lack these tables and refuse only their dependent features by name.
pub const RESOURCES_MAP_FORMAT: u16 = 103;
pub const INDUSTRIAL_ORGANISATION_DETAILS_FORMAT: u16 = 104;

/// Stored division/commander XP and complete commander training progress.
pub const EXPERIENCE_FORMAT: u16 = 104;
/// Stored faction state supplied through native writer observation. Account
/// numeric terms and research bonuses use independent scalar occurrences; owner
/// refresh boundaries certify derived inputs. Advisor modifier occurrences retain
/// slot/source ownership. FactionAccounts adds the independently carried optional
/// effective subject-contribution and population-growth accumulator operands.
/// Projection source occurrences
/// retain pre-division products, a factory component and supplied unit counts.
/// This provisional contract replaces the lane's earlier
/// vector layout; it has no migration or legacy reader.
pub const FACTION_STATE_FORMAT: u16 = 104;

/// Doctrine choices, progression and grants as independent changed columns.
pub const DOCTRINES_FORMAT: u16 = 104;
/// Provisional main+1: the land movement speed's remaining recorded operands
/// beside the new tables below: DivisionStatus `combat_movement` (the combat
/// movement base), the `move_flags` bits [`crate::rows::DivisionStatusRow::MOVE_NONCORE_AIR`]
/// and [`crate::rows::DivisionStatusRow::MOVE_NONCORE_AIR_REFUSED`],
/// and DivisionTemplateIcons `speed_keys` (the movement priority sub-unit's and its
/// categories' modifier keys). Older recordings lack the fields by name.
pub const MOVEMENT_OPERANDS_FORMAT: u16 = 104;

/// Provisional main+1: MovementStateTerms holds the location state's speed
/// modifier entries a division's land movement speed adds. This and its companion
/// tables and fields use only the final format-104 layout.
pub const MOVEMENT_STATE_TERMS_FORMAT: u16 = 104;

/// Provisional main+1: CoreSupplyAccess holds the owner's core supply access in
/// a division's location controller area, which gates the noncore-supply speed
/// term. Older recordings refuse only that table, by name, as
/// [`MOVEMENT_STATE_TERMS_FORMAT`] describes.
pub const CORE_SUPPLY_ACCESS_FORMAT: u16 = 104;

/// Provisional main+1: CountryUnitSpeedEntries holds the country accumulator
/// entries of the movement priority sub-unit's speed keys. Older recordings
/// refuse only that table, by name, as [`MOVEMENT_STATE_TERMS_FORMAT`] describes.
pub const COUNTRY_UNIT_SPEED_ENTRIES_FORMAT: u16 = 104;

/// Provisional main+1: RiverRelations holds the relation river crossing factor a
/// crossing into a controller's province adds. Older recordings refuse only that
/// table, by name, as [`MOVEMENT_STATE_TERMS_FORMAT`] describes.
pub const RIVER_RELATIONS_FORMAT: u16 = 104;

/// Provisional main+1: DivisionTransportSpeeds holds the transport speeds a
/// division's naval-transfer and sea legs move at. Older recordings refuse only
/// that table, by name, as [`MOVEMENT_STATE_TERMS_FORMAT`] describes.
pub const DIVISION_TRANSPORT_SPEEDS_FORMAT: u16 = 104;
/// Regional active-mission counters supplying the Air map mode's zero-weight colours.
pub const AIR_MAP_FORMAT: u16 = 104;

/// A row field whose capture ended at `retired`. A whole field keeps its
/// Opaque framing as a [`super::Retired`] tombstone; a nested name
/// (`field.part`) left its parent's nested layout, which still decodes older
/// bytes through their source format.
#[derive(Clone, Copy, Debug)]
pub struct RetiredField {
    pub table: TableId,
    pub index: usize,
    pub name: &'static str,
    pub retired: u16,
}

const fn retired(table: TableId, index: usize, name: &'static str) -> RetiredField {
    RetiredField {
        table,
        index,
        name,
        retired: RETIRED_INPUTS_FORMAT,
    }
}

/// Every retired table: no current writer emits it, and older recordings
/// discard it through its framing and refuse it by name.
pub static RETIRED_TABLES: [TableId; 19] = [
    TableId::NavalMineLevels,
    TableId::DivisionEtaInputs,
    TableId::AttritionModifierBreakdowns,
    TableId::ReservedNearestNavies,
    TableId::ReservedNearestBases,
    // Hover-only modifier breakdowns.
    TableId::ReserveModifierBreakdowns,
    TableId::NavalModifierBreakdowns,
    // Read only by create_wargoal's effect-description hover text.
    TableId::OwnedWargoals,
    // Superseded by AirRegionCombat, which records both in one row.
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    // Oracle-only.
    // The selected transport preview's split tables (RESERVED_SPLIT_TABLES).
    TableId::TransportNavies,
    TableId::TransportLedgers,
    TableId::TransportReceivers,
    TableId::TransportSearchRegions,
    TableId::TransportSpottingOwners,
    TableId::TransportSpottingRegions,
    TableId::TransportAirRoots,
    TableId::TransportAirRegions,
    TableId::TransportRiskShips,
];

/// Every retired row field, beside the retired tables' lifetimes in [`HISTORY`].
pub static RETIRED_FIELDS: [RetiredField; 62] = [
    retired(TableId::Provinces, 8, "selected_areas"),
    retired(TableId::DivisionMovement, 18, "fresh"),
    retired(TableId::DivisionMovement, 20, "selected"),
    retired(TableId::DivisionMovement, 21, "transport"),
    retired(TableId::DivisionStatus, 23, "attrition_inputs"),
    retired(TableId::DivisionLandInputs, 1, "received_supply"),
    retired(TableId::DivisionLandInputs, 2, "maximum_grace"),
    retired(TableId::DivisionLandInputs, 3, "remaining_grace"),
    retired(TableId::CountryAppearance, 21, "transport_convoys"),
    retired(TableId::CountryCounterInputs, 13, "movement"),
    retired(TableId::RailwayGunCounterInputs, 29, "transport"),
    retired(TableId::AirBaseRegistrations, 3, "transport_planes"),
    retired(TableId::ReservedCountryNaming, 15, "movement"),
    retired(
        TableId::ReservedCountryNaming,
        21,
        "raw_country_observations",
    ),
    retired(
        TableId::ReservedCountryNaming,
        22,
        "transport_mission_inputs",
    ),
    // The reserve reinforcement speed operands, which the appended
    // `reinforcement_speed` share replaces ([`RESERVE_SPEED_FORMAT`]), and
    // the division-commander hover operands; playback takes the division
    // commander's name, skill and traits from UnitLeaders and
    // UnitLeaderTraits.
    retired(TableId::LandCombatDivisions, 21, "commander_inputs"),
    retired(TableId::LandCombatDivisions, 22, "reinforcement"),
    retired(TableId::LandCombatDivisions, 23, "reinforcement_modifiers"),
    retired(TableId::LandCombatDivisions, 24, "reinforcement_supply"),
    retired(
        TableId::LandCombatDivisions,
        25,
        "reinforcement_acclimatization",
    ),
    retired(TableId::LandCombatDivisions, 26, "reinforcement_transport"),
    // Fuel, the stats and the scales as packets, which a numeric field each
    // replaces ([`LAND_DIVISION_NUMBERS_FORMAT`]); older recordings' packets
    // still answer.
    retired(TableId::LandCombatDivisions, 10, "fuel_packet"),
    retired(TableId::LandCombatDivisions, 11, "stat_packet"),
    retired(TableId::LandCombatDivisions, 18, "scale_packet"),
    // The side's list of every member's entries, which
    // LandCombatDivisionModifiers holds a row a member
    // ([`LAND_MODIFIER_ROWS_FORMAT`]).
    retired(TableId::LandCombatModifiers, 1, "divisions"),
    // The side's hover and oracle round values and its commander's hover
    // operands, which the side inputs record ([`SIDE_INPUTS_FORMAT`]); the
    // marker's organisation-loss ring and the window's air damage lanes stay.
    retired(TableId::LandCombatDetails, 7, "sides.anti_air_attack"),
    retired(TableId::LandCombatDetails, 7, "sides.damage[2..6]"),
    retired(TableId::LandCombatDetails, 7, "sides.commander_experience"),
    retired(
        TableId::LandCombatDetails,
        7,
        "sides.commander_description_token",
    ),
    retired(TableId::LandCombatDetails, 7, "sides.timed_traits"),
    retired(TableId::LandCombatDetails, 7, "sides.learning_traits"),
    // Playback derives progress and direction from LandCombats, Divisions
    // and DivisionMovement.
    retired(TableId::LandCombatProgress, 0, "attacker_progress"),
    retired(TableId::LandCombatProgress, 1, "defender_progress"),
    retired(TableId::LandCombatProgress, 2, "direction_attacker"),
    retired(TableId::LandCombatProgress, 3, "direction_province"),
    // The port strike-force readiness hover's strike operands.
    retired(TableId::ShipModels, 7, "strike"),
    // Oracle-only values. Ships.experience is recorded again, at its display
    // step ([`super::ShipRow::EXPERIENCE_STEP`]); format-102 captures written
    // while it was retired hold zero there, and no published build wrote them.
    retired(TableId::CountryFuel, 2, "gain_raw"),
    retired(TableId::CountryFuel, 3, "cost_raw"),
    retired(TableId::CountryFuel, 4, "gain_from_states_raw"),
    retired(TableId::CountryFuel, 6, "lend_lease_consumption_raw"),
    retired(TableId::CountryFuel, 7, "gain_per_oil_raw"),
    retired(TableId::CountryFuel, 8, "history_index"),
    retired(TableId::CountryFuel, 9, "remaining_hours"),
    retired(TableId::CountryFuel, 13, "ring_requested_0_raw"),
    retired(TableId::CountryFuel, 14, "ring_requested_1_raw"),
    retired(TableId::CountryFuel, 15, "ring_requested_2_raw"),
    // The update-by-update fuel award window; `award_day` sums what the Fuel
    // widget shows, and only the oracle read the hourly source witness.
    retired(TableId::CountryFuel, 20, "awards"),
    retired(TableId::Politics, 1, "election_frequency"),
    retired(TableId::Politics, 2, "elections_allowed"),
    retired(TableId::Capitulation, 2, "last_surrender_hour"),
    retired(TableId::Capitulation, 3, "faction_join_hour"),
    retired(TableId::Capitulation, 4, "legitimacy_raw"),
    retired(TableId::Capitulation, 5, "exile_army_leaders"),
    retired(TableId::ProductionLines, 8, "produced_raw"),
    retired(TableId::ProductionLineOutput, 4, "mio_credit"),
    retired(TableId::ProductionLineOutput, 10, "instant_raw"),
    // No reader at all.
    retired(TableId::ProductionLineModifiers, 7, "conversion"),
    // Playback derives the glyph byte from the row's own value and threshold.
    retired(TableId::DivisionStatus, 7, "attrition"),
    // Path-cost operands no consumer reads; CountryCounterInputs records
    // the same slot-0 relation bytes.
    retired(TableId::InvasionPreparations, 9, "zero_war_present"),
    retired(TableId::InvasionPreparations, 10, "zero_war_retired"),
    retired(TableId::InvasionPreparations, 11, "zero_co_belligerent"),
    retired(TableId::InvasionPreparations, 12, "co_belligerents"),
];
/// Provisional main+1: playback derives what the game computes from the hour
/// and the content ([`crate::day_night`]). WeatherProvinces appends each
/// province clock's source (fields 6 and 7 then hold only a held clock).
/// Format-102 captures written before this change share the version number
/// but not this layout, so their blocks read as malformed; format 101 keeps
/// its recorded clocks.
pub const DERIVED_CLOCKS_FORMAT: u16 = 102;
/// Provisional main+1: WeatherProvinces rows hold each province's temperature
/// words at their frame's anchor and slot zero the hourly pass's inputs since
/// ([`crate::rows::ProvinceTemperature`], [`crate::temperature`]), so field 4
/// is the anchor's temperature. Format-102 captures written before this
/// change share the version number but not the layout, so their blocks read
/// as malformed; format 101 keeps each hour's recorded temperature.
pub const DERIVED_TEMPERATURE_FORMAT: u16 = 102;

/// Provisional main+1: a same-process reattach that continues its branch
/// records the event interval it could not observe as a control record
/// (`ControlRecord::EventLoss`, spool and trailer tag 8). Older recordings
/// hold none; their reattached first intervals stay unexplained.
pub const EVENT_LOSS_FORMAT: u16 = 102;

/// Provisional main+1: DivisionStatus holds each name member's own template
/// reference (30, `name_template`), a [`Witnessed`](crate::rows::Witnessed)
/// packet only a `name_template_witness` collector fills, so the oracle can
/// compare a division whose member names a deleted template against what the
/// game holds rather than the template a save rebinds on load. Older
/// recordings have no field.
pub const NAME_TEMPLATE_FORMAT: u16 = 102;

/// Provisional main+1: DivisionTemplateIcons holds each composition grid's
/// (definition, cell count) entries (15, `grids`,
/// [`TemplateGrids`](crate::rows::TemplateGrids)), so battle ambience counts
/// the regiment and support grids' slots without the regimental-support
/// grid's. Older recordings have no field.
pub const TEMPLATE_GRIDS_FORMAT: u16 = 102;
/// Spatial grid cells and stored template restrictions and equipment settings.
pub const TEMPLATE_SETTINGS_FORMAT: u16 = 104;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    ReplayHeader,
    ReplayBlock,
    ReplayTrailer,
    ReplayFooter,
    SpoolEnvelope,
    Frame,
    Identity,
    SessionStart,
    BranchStart,
    Gap,
    RecoveryNote,
    Detach,
    ClockWitness,
    GapSummary,
    Partial,
    NavalCommanderInputs,
    LandBorderWarInputs,
    EventFeedEvent,
    EventLoss,
}

impl RecordKind {
    pub const ALL: [Self; 19] = [
        Self::ReplayHeader,
        Self::ReplayBlock,
        Self::ReplayTrailer,
        Self::ReplayFooter,
        Self::SpoolEnvelope,
        Self::Frame,
        Self::Identity,
        Self::SessionStart,
        Self::BranchStart,
        Self::Gap,
        Self::RecoveryNote,
        Self::Detach,
        Self::ClockWitness,
        Self::GapSummary,
        Self::Partial,
        Self::NavalCommanderInputs,
        Self::LandBorderWarInputs,
        Self::EventFeedEvent,
        Self::EventLoss,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Table(TableId),
    Record(RecordKind),
}

#[derive(Clone, Copy, Debug)]
pub struct LastChange {
    pub subject: Subject,
    /// Table lifetime in the timing/update stream. Baseline entries were
    /// already present at PROVEN_FROM; their earlier introduction is unaudited.
    pub introduced: u16,
    /// Retain retired TableIds and their field framing as wire tombstones.
    /// Never reuse an id or shift the remaining timing/update positions.
    pub retired: Option<u16>,
    /// For baseline entries: unchanged since this proven horizon, possibly older.
    pub format: u16,
    pub commit: &'static str,
    pub kind: ChangeKind,
    pub appended: &'static [AppendedField],
}

/// Audited changes retain enough framing to skip incompatible rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Unchanged,
    /// Every older layout decodes: fields were appended, or retired as
    /// tombstones that keep their framing ([`RETIRED_FIELDS`]).
    AppendedOnly,
    Layout,
    Meaning,
}

#[derive(Clone, Copy, Debug)]
pub struct AppendedField {
    pub index: usize,
    pub name: &'static str,
    pub introduced: u16,
}

/// One code-owned catalog for tables, nested values and framing/control kinds.
/// Evidence: row declarations and typed_codec diffs at each listed commit,
/// including the wider DivisionStatus mask (the recorded field count owns it).
/// The baseline is #302; #323/#325/#328/#341/#343 were audited against it. New
/// fields were appended; nested Opaque values keep their byte-length framing.
/// Nested suffixes use source format even when parent field counts are unchanged.
/// Record-kind dates 1/24/33 are inert below the proven horizon of 91.
/// Adding/retiring a table changes its lifetime, not the Frame framing date.
pub static HISTORY: [LastChange; TableId::ALL.len() + RecordKind::ALL.len()] = {
    let baseline = LastChange {
        subject: Subject::Table(TableId::Clock),
        introduced: PROVEN_FROM,
        retired: None,
        format: PROVEN_FROM,
        commit: "0e632a9b16",
        kind: ChangeKind::Unchanged,
        appended: &[],
    };
    let mut history = [baseline; TableId::ALL.len() + RecordKind::ALL.len()];
    history[TableId::CountryWargoals.index()] = table(TableId::CountryWargoals, 104, "wargoals");
    history[TableId::CountryWargoals.index()].introduced = 104;
    history[TableId::WargoalTargets.index()] = table(TableId::WargoalTargets, 104, "wargoals");
    history[TableId::WargoalTargets.index()].introduced = 104;
    history[TableId::WargoalJustifications.index()] =
        table(TableId::WargoalJustifications, 104, "wargoals");
    history[TableId::WargoalJustifications.index()].introduced = 104;
    history[TableId::DoctrineCountries.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineCountries,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineFolders.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineFolders,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineTracks.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineTracks,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineLeaderMastery.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineLeaderMastery,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineCostReductions.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineCostReductions,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineMasteryGrants.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineMasteryGrants,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineEquipmentBonuses.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineEquipmentBonuses,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::FactionAdvisorModifiers.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionAdvisorModifiers,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionMinimumUpgrades.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionMinimumUpgrades,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionDoctrine.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionDoctrine,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionProjection.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionProjection,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionResources.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionResources,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionResearch.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionResearch,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionAccounts.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionAccounts,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionVariables.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionVariables,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionRules.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionRules,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionPrograms.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionPrograms,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionUnlocks.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionUnlocks,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionManpower.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionManpower,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionGoalStatus.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionGoalStatus,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionGoalEntries.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionGoalEntries,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::RocketProductionOutput.index()] = table(
        TableId::RocketProductionOutput,
        WIDGET_CAPTURE_FORMAT,
        "wc/rocket",
    );
    history[TableId::RocketProductionOutput.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::ProductionLineOutput.index()] = appended_at(
        TableId::ProductionLineOutput,
        WIDGET_CAPTURE_FORMAT,
        "production-spending-inputs",
        &[
            AppendedField {
                index: 9,
                name: "spent_raw",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "instant_raw",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    let mut i = 0;
    while i < TableId::ALL.len() {
        history[i].subject = Subject::Table(TableId::ALL[i]);
        i += 1;
    }
    history[TableId::StateResistance.index()] = LastChange {
        // Opaque native packets cannot decode an absent suffix. Keep its
        // declared prefix for schema accounting, with whole-layout admission.
        kind: ChangeKind::Layout,
        ..appended_at(
            TableId::StateResistance,
            OCCUPATION_MAP_FORMAT,
            "occupation hover and map markers",
            &[
                AppendedField {
                    index: 6,
                    name: "hover",
                    introduced: OCCUPATION_HOVER_FORMAT,
                },
                AppendedField {
                    index: 7,
                    name: "modifier_sources",
                    introduced: OCCUPATION_HOVER_FORMAT,
                },
                AppendedField {
                    index: 8,
                    name: "map",
                    introduced: OCCUPATION_MAP_FORMAT,
                },
            ],
        )
    };
    history[TableId::ResearchTechnologies.index()] = table(
        TableId::ResearchTechnologies,
        POLITICAL_FORMAT,
        "technology-occurrences",
    );
    // The changed key has the same width; the old payload's five scalar
    // fields can be skipped by the compatibility reader without decoding rows.
    history[TableId::ResearchTechnologies.index()].appended = &[
        AppendedField {
            index: 5,
            name: "technology_handle",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 6,
            name: "applied_bonus_count",
            introduced: POLITICAL_FORMAT,
        },
    ];
    history[TableId::TechnologyDefinitions.index()] = table(
        TableId::TechnologyDefinitions,
        POLITICAL_FORMAT,
        "technology-occurrences",
    );
    history[TableId::ResearchSlots.index()] = appended_at(
        TableId::ResearchSlots,
        POLITICAL_FORMAT,
        "technology-occurrences",
        &[AppendedField {
            index: 5,
            name: "database_index",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ResearchInputs.index()] = appended_at(
        TableId::ResearchInputs,
        POLITICAL_FORMAT,
        "technology-occurrences",
        &[AppendedField {
            index: 15,
            name: "database_index",
            introduced: POLITICAL_FORMAT,
        }],
    );
    let politics = [
        TableId::CountryPowerDaily,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
        TableId::GovernmentChanges,
        TableId::PoliticsCaptureCoverage,
    ];
    let mut p = 0;
    while p < politics.len() {
        let id = politics[p];
        history[id.index()] = table(id, WIDGET_CAPTURE_FORMAT, "3b4f34f2ea");
        history[id.index()].introduced = WIDGET_CAPTURE_FORMAT;
        p += 1;
    }
    history[TableId::WarOwnedRelations.index()] = table(
        TableId::WarOwnedRelations,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarOwnedRelations.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarRelationGoals.index()] = table(
        TableId::WarRelationGoals,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarRelationGoals.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarOverviewCountries.index()] = table(
        TableId::WarOverviewCountries,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarOverviewCountries.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarNameRules.index()] = table(
        TableId::WarNameRules,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarNameRules.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::PeaceSettlements.index()] = table(
        TableId::PeaceSettlements,
        PEACE_SETTLEMENTS_FORMAT,
        "capture/w4-peace-deals",
    );
    history[TableId::PeaceSettlements.index()].introduced = PEACE_SETTLEMENTS_FORMAT;
    history[TableId::PeaceBids.index()] = table(
        TableId::PeaceBids,
        PEACE_SETTLEMENTS_FORMAT,
        "capture/w4-peace-deals",
    );
    history[TableId::PeaceBids.index()].introduced = PEACE_SETTLEMENTS_FORMAT;
    let mut r = 0;
    while r < RecordKind::ALL.len() {
        let kind = RecordKind::ALL[r];
        let (format, commit) = match kind {
            RecordKind::SessionStart => (24, "9ae9a5498c"),
            RecordKind::SpoolEnvelope => (33, "1b7474a555"),
            RecordKind::Frame => (91, "0e632a9b16"),
            RecordKind::GapSummary => (1, "7185d6e386"),
            RecordKind::Partial => (92, "11b905d2b2"),
            RecordKind::NavalCommanderInputs => (RECORDER_INPUTS_FORMAT, "d53fa4e05c"),
            RecordKind::LandBorderWarInputs => (RECORDER_INPUTS_FORMAT, "9d09404c0d"),
            RecordKind::EventFeedEvent => (SCRIPT_EVENT_FORMAT, "pol/capture-events-decisions"),
            RecordKind::EventLoss => (EVENT_LOSS_FORMAT, "record/reattach-loss"),
            _ => (1, "5d3b4dd945"),
        };
        history[i + r] = LastChange {
            subject: Subject::Record(kind),
            introduced: format,
            retired: None,
            format,
            commit,
            kind: if matches!(kind, RecordKind::Partial) {
                ChangeKind::Layout
            } else {
                ChangeKind::Unchanged
            },
            appended: &[],
        };
        r += 1;
    }
    history[TableId::NavalResultSources.index()] = table(
        TableId::NavalResultSources,
        WIDGET_CAPTURE_FORMAT,
        "widget-native-naval",
    );
    history[TableId::NavalResultSources.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::AirAces.index()] = appended_at(
        TableId::AirAces,
        GAME_SCORE_FORMAT,
        "17d59e11b4",
        &[AppendedField {
            index: 11,
            name: "presentation",
            introduced: GAME_SCORE_FORMAT,
        }],
    );
    history[TableId::CountryGameScoreInputs.index()] = table(
        TableId::CountryGameScoreInputs,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::CountryGameScoreInputs.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::AirWingGameScoreStats.index()] = table(
        TableId::AirWingGameScoreStats,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::AirWingGameScoreStats.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::GameScoreMultipliers.index()] = table(
        TableId::GameScoreMultipliers,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::GameScoreMultipliers.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::Countries.index()] = table(TableId::Countries, 93, "9fb09a565f");
    history[TableId::LandCombatDetails.index()] =
        table(TableId::LandCombatDetails, 93, "9fb09a565f");
    history[TableId::RaidAirScores.index()] = table(
        TableId::RaidAirScores,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy regional active-mission counters and aircraft spotting scale",
    );
    history[TableId::RadarCoverage.index()] = table(
        TableId::RadarCoverage,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy certified radar symbols",
    );
    history[TableId::SimulationFeatures.index()] = appended_at(
        TableId::SimulationFeatures,
        SCRIPTED_MAP_FORMAT,
        "pending political runtime inputs, pol/finish-eta movement defines, pol/mapmode-scripted state array extent",
        &[
            AppendedField {
                index: 2,
                name: "weather_suspend",
                introduced: 93,
            },
            AppendedField {
                index: 3,
                name: "instant_decisions",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "runtime_scalars",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "movement_defines",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "state_array_len",
                introduced: SCRIPTED_MAP_FORMAT,
            },
        ],
    );
    history[TableId::Provinces.index()] = appended_at(
        TableId::Provinces,
        ATTACK_INPUTS_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs, restore/move areas",
        &[
            AppendedField {
                index: 7,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "selected_areas",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "fort_bunker",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "controller_area",
                introduced: AREA_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "owner_area",
                introduced: AREA_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "area_node",
                introduced: AREA_FORMAT,
            },
        ],
    );
    history[TableId::CountryWars.index()] = appended_at(
        TableId::CountryWars,
        POLITICAL_FORMAT,
        "pending selected route inputs",
        &[AppendedField {
            index: 3,
            name: "ordered",
            introduced: POLITICAL_FORMAT,
        }],
    );
    // MOVEMENT_TERMS_FORMAT grew `combat` by its army speed and attack speed entries.
    history[TableId::WeatherRegions.index()] = appended_at(
        TableId::WeatherRegions,
        AIR_MAP_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs, pol/finish-eta combat speed entries",
        &[
            AppendedField {
                index: 7,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "combat",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "air_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "map_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "naval_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "air_attack",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "air_defence",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::WeatherProvinces.index()] = table(
        TableId::WeatherProvinces,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
    );
    history[TableId::StateFlags.index()] = table(TableId::StateFlags, 93, "9fb09a565f");
    // WING_NAME extended the meaning of the existing packed entry family.
    history[TableId::RaidTooltipOperands.index()] =
        table(TableId::RaidTooltipOperands, 94, "ccf740b81d");
    history[TableId::DivisionMovement.index()] = table(
        TableId::DivisionMovement,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
    );
    history[TableId::CountryAppearance.index()] = table(
        TableId::CountryAppearance,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::IntelNetworks.index()] = table(
        TableId::IntelNetworks,
        INTEL_NETWORK_MAP_FORMAT,
        "pol/mapmode-operatives network map inputs",
    );
    history[TableId::States.index()] = appended_at(
        TableId::States,
        SCRIPTED_MAP_FORMAT,
        "pending political runtime inputs, pol/mapmode-capture population, pol/mapmode-infrastructure category, pol/mapmode-scripted cached choices",
        &[
            AppendedField {
                index: 6,
                name: "continent",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "railway_infrastructure",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "population",
                introduced: MAP_MODE_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "category",
                introduced: STATE_CATEGORY_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "manpower_locked",
                introduced: 104,
            },
            AppendedField {
                index: 11,
                name: "best_vp_index",
                introduced: SCRIPTED_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "single_controller",
                introduced: SCRIPTED_MAP_FORMAT,
            },
        ],
    );
    history[TableId::Capitulation.index()] = appended_at(
        TableId::Capitulation,
        MAP_MODE_INPUTS_FORMAT,
        "pol/mapmode-capture owned war goals",
        &[AppendedField {
            index: 6,
            name: "war_goal_recipients",
            introduced: MAP_MODE_INPUTS_FORMAT,
        }],
    );
    history[TableId::StateCores.index()] = appended_at(
        TableId::StateCores,
        STRATEGIC_LOCATIONS_FORMAT,
        "pol/strategic-locations state and province strategic locations",
        &[
            AppendedField {
                index: 1,
                name: "claims",
                introduced: MAP_MODE_INPUTS_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "strategic_locations",
                introduced: STRATEGIC_LOCATIONS_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "province_strategic_locations",
                introduced: STRATEGIC_LOCATIONS_FORMAT,
            },
        ],
    );
    history[TableId::RailCooldowns.index()] = appended_at(
        TableId::RailCooldowns,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 1,
                name: "building",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "selection",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "selection_hour",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::ArmyOrderPlans.index()] = appended_at(
        TableId::ArmyOrderPlans,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 20,
                name: "child_fronts",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "invasion_transports",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "execution",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::DivisionTemplates.index()] =
        table(TableId::DivisionTemplates, 97, "d313f6a6e3");
    history[TableId::Decisions.index()] = table(TableId::Decisions, 97, "d313f6a6e3");
    history[TableId::Countries.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Countries.index()].appended = &[AppendedField {
        index: 9,
        name: "border_land_doctrine_level",
        introduced: 93,
    }];
    history[TableId::Politics.index()] = appended_at(
        TableId::Politics,
        104,
        "restore/economy has_elections",
        &[
            AppendedField {
                index: 6,
                name: "has_elections",
                introduced: HAS_ELECTIONS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "election_interval",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "next_election_cache",
                introduced: 104,
            },
        ],
    );
    history[TableId::LandCombatDivisions.index()] = appended_at(
        TableId::LandCombatDivisions,
        LAND_DIVISION_NUMBERS_FORMAT,
        "restore/combat reserve speed share, size/lcd-flat numeric fields",
        &[
            AppendedField {
                index: 30,
                name: "reinforcement_speed",
                introduced: RESERVE_SPEED_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "fuel_inputs[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "fuel_inputs[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "stat_values[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "stat_values[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 35,
                name: "stat_values[2]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 36,
                name: "stat_values[3]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 37,
                name: "stat_values[4]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 38,
                name: "scales[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 39,
                name: "scales[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 40,
                name: "scales[2]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 41,
                name: "scales[3]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
        ],
    );
    history[TableId::LandCombatDetails.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatDetails.index()].appended = &[
        AppendedField {
            index: 9,
            name: "border_war",
            introduced: 93,
        },
        AppendedField {
            index: 10,
            name: "side_inputs",
            introduced: SIDE_INPUTS_FORMAT,
        },
    ];
    history[TableId::WeatherProvinces.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::WeatherProvinces.index()].appended = &[
        AppendedField {
            index: 4,
            name: "temperature",
            introduced: 93,
        },
        AppendedField {
            index: 5,
            name: "raw_mud",
            introduced: 93,
        },
        AppendedField {
            index: 6,
            name: "local_hour",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 7,
            name: "night_factor",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 8,
            name: "display",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 9,
            name: "clock_source",
            introduced: DERIVED_CLOCKS_FORMAT,
        },
        AppendedField {
            index: 10,
            name: "temperature_drift",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 11,
            name: "temperature_staged",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 12,
            name: "temperature_working",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 13,
            name: "temperature_pass",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 14,
            name: "modifiers",
            introduced: MOVEMENT_TERMS_FORMAT,
        },
        AppendedField {
            index: 15,
            name: "daylight",
            introduced: AIR_MAP_FORMAT,
        },
    ];
    history[TableId::WeatherProvinces.index()].format = AIR_MAP_FORMAT;
    history[TableId::WeatherProvinces.index()].commit = "perf/temperature";
    history[TableId::StateFlags.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::StateFlags.index()].appended = &[
        AppendedField {
            index: 1,
            name: "border_conflict_active",
            introduced: 93,
        },
        AppendedField {
            index: 2,
            name: "border_conflict_countries",
            introduced: 93,
        },
    ];
    history[TableId::DivisionMovement.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionMovement.index()].appended = &[
        AppendedField {
            index: 14,
            name: "strategic_action",
            introduced: 96,
        },
        AppendedField {
            index: 15,
            name: "withdrawal",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 16,
            name: "movement_priority",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 17,
            name: "order_transport",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 18,
            name: "fresh",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 19,
            name: "combat_references",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 20,
            name: "selected",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 21,
            name: "transport",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 22,
            name: "base_speed",
            introduced: BASE_SPEED_FORMAT,
        },
        AppendedField {
            index: 23,
            name: "fuel",
            introduced: MOVEMENT_TERMS_FORMAT,
        },
        AppendedField {
            index: 24,
            name: "transport_speed",
            introduced: EXPERIENCE_FORMAT,
        },
    ];
    history[TableId::RelationLinks.index()] = appended_at(
        TableId::RelationLinks,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 1,
            name: "transport_subject",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::CountryAppearance.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::CountryAppearance.index()].format = POLITICAL_FORMAT;
    history[TableId::CountryAppearance.index()].commit = "pending political runtime inputs";
    history[TableId::CountryAppearance.index()].appended = &[
        AppendedField {
            index: 18,
            name: "subjects",
            introduced: 97,
        },
        AppendedField {
            index: 19,
            name: "original_tag",
            introduced: EFFECT_INPUTS_FORMAT,
        },
        AppendedField {
            index: 20,
            name: "original_tag_reference",
            introduced: EFFECT_INPUTS_FORMAT,
        },
        AppendedField {
            index: 21,
            name: "transport_convoys",
            introduced: POLITICAL_FORMAT,
        },
    ];
    history[TableId::DivisionTemplates.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionTemplates.index()].appended = &[AppendedField {
        index: 3,
        name: "name_key",
        introduced: 97,
    }];
    history[TableId::Decisions.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Decisions.index()].appended = &[AppendedField {
        index: 7,
        name: "description_seed",
        introduced: 97,
    }];
    history[TableId::RaidTooltipOperands.index()].kind = ChangeKind::Meaning;
    history[TableId::AvailableAdvisors.index()] = table(
        TableId::AvailableAdvisors,
        RECORDER_INPUTS_FORMAT,
        "d53fa4e05c",
    );
    history[TableId::AvailableAdvisors.index()].introduced = RECORDER_INPUTS_FORMAT;
    history[TableId::OwnedWargoals.index()] =
        table(TableId::OwnedWargoals, RECORDER_INPUTS_FORMAT, "9d09404c0d");
    history[TableId::OwnedWargoals.index()].introduced = RECORDER_INPUTS_FORMAT;
    history[TableId::CountryExperienceGains.index()] = table(
        TableId::CountryExperienceGains,
        WIDGET_CAPTURE_FORMAT,
        "ecb7ef4c1a",
    );
    history[TableId::CountryExperienceGains.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::EventFeedHistory.index()] = table(
        TableId::EventFeedHistory,
        EVENT_FEED_COLUMNS_FORMAT,
        "event-feed-numeric-streams",
    );
    history[TableId::EventFeedHistory.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::AdvisorRoles.index()] = appended_at(
        TableId::AdvisorRoles,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[
            AppendedField {
                index: 4,
                name: "modifier_cache",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "tooltip_keys",
                introduced: EFFECT_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::UnitLeaders.index()] = appended_at(
        TableId::UnitLeaders,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[
            AppendedField {
                index: 20,
                name: "exile_country",
                introduced: 93,
            },
            AppendedField {
                index: 21,
                name: "codename",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "formation_reference",
                introduced: EFFECT_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::CharacterOperatives.index()] = appended(
        TableId::CharacterOperatives,
        "d53fa4e05c",
        &[
            AppendedField {
                index: 6,
                name: "codename",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "name",
                introduced: RECORDER_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::OperativeMemberships.index()] = appended_at(
        TableId::OperativeMemberships,
        INTEL_INPUTS_FORMAT,
        "intel-native-union",
        &[
            AppendedField {
                index: 8,
                name: "ordinal",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "target_country",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "operation",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "name",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "cooldown_kind",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "cooldown_start_hour",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "cooldown_end_hour",
                introduced: INTEL_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::Operations.index()] = table(
        TableId::Operations,
        OPERATION_PREPARATION_FORMAT,
        "pol/mapmode-operative-icons",
    );
    history[TableId::CountryTemplateState.index()] = table(
        TableId::CountryTemplateState,
        TEMPLATE_SETTINGS_FORMAT,
        "country template state",
    );
    history[TableId::CountryTemplateState.index()].introduced = TEMPLATE_SETTINGS_FORMAT;
    let intel = [TableId::AgencyUpgradeLevels, TableId::OperationCompletions];
    let mut i = 0;
    while i < intel.len() {
        let id = intel[i];
        history[id.index()] = table(id, INTEL_INPUTS_FORMAT, "intel-native-union");
        history[id.index()].introduced = INTEL_INPUTS_FORMAT;
        i += 1;
    }
    history[TableId::DivisionStatus.index()] = appended_at(
        TableId::DivisionStatus,
        MOVEMENT_OPERANDS_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 16,
                name: "border_combat_stats",
                introduced: 93,
            },
            AppendedField {
                index: 17,
                name: "naming",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "supply_grace_maximum",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "supply_grace_remaining",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "country_ordinal",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "invalid_orders_reason",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "supply",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 23,
                name: "attrition_inputs",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "dig_in_factor",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "camouflage",
                introduced: CAMOUFLAGE_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "attack_stats",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "weather_factor",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 30,
                name: "name_template",
                introduced: NAME_TEMPLATE_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "movement_stats",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "out_of_supply_days",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "commander_modifiers",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "supply_ratio_exact",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 35,
                name: "supply_grace_exact",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 36,
                name: "experience",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 37,
                name: "pending_template",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 38,
                name: "officer_experience",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 39,
                name: "career_counter",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 40,
                name: "panel_source",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 41,
                name: "combat_movement",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 42,
                name: "organisation_exact",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 43,
                name: "maximum_organisation_exact",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 44,
                name: "commander_width_factor",
                introduced: EXPERIENCE_FORMAT,
            },
        ],
    );
    history[TableId::OrdersGroupCounterInputs.index()] = appended_at(
        TableId::OrdersGroupCounterInputs,
        TEMPLATE_SETTINGS_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs",
        &[
            AppendedField {
                index: 1,
                name: "hq_distance",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "leader_enroute_days",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "border_war_presence",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "invasion_source_distance",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "commander_attack",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "commander_movement",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "hq_nearest_front",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "template_context",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
        ],
    );
    history[TableId::DivisionTemplateIcons.index()] = appended_at(
        TableId::DivisionTemplateIcons,
        104,
        "pending political runtime inputs, pol/finish-audio template grids, pol/capture-manpower exile predicate",
        &[
            AppendedField {
                index: 13,
                name: "formation_division_count",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "display_name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "grids",
                introduced: TEMPLATE_GRIDS_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "manpower_exile",
                introduced: 104,
            },
            AppendedField {
                index: 17,
                name: "base_width",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "layout",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "settings",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "speed_keys",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
        ],
    );
    // From provisional format 102 `directed_wars` (index 18) is no longer
    // captured: writers store None and playback derives every cell from the
    // hour's WarRelations (`directed_war_derive`). The layout is unchanged,
    // so format 101 and earlier 102 development recordings keep answering
    // from their packets and no table is refused.
    history[TableId::FleetCounterInputs.index()] = appended_at(
        TableId::FleetCounterInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy fleet region membership",
        &[AppendedField {
            index: 6,
            name: "mission_regions",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::CountryCounterInputs.index()] = appended_at(
        TableId::CountryCounterInputs,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 4,
                name: "owned_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "controlled_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "core_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "army_leaders",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "navy_leaders",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "mio_policies",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "neighbors",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "co_belligerents",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "zero_war",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "movement",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "zero_subject",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "pride_name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "forward_visibility",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "directed_wars",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "attack_modifiers",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "movement_modifiers",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "recruitment_modifiers",
                introduced: 104,
            },
            AppendedField {
                index: 22,
                name: "sent_expeditionaries",
                introduced: 104,
            },
            AppendedField {
                index: 23,
                name: "air_modifiers",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "mines_effect_reduction",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "naval_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "air_superiority_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "air_intercept_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "air_cas_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "air_nav_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 30,
                name: "air_attack_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "air_defence_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "air_strategic_bomber_defence_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "air_combat_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "combat_width_factor",
                introduced: EXPERIENCE_FORMAT,
            },
        ],
    );
    history[TableId::OccupationSettings.index()] = appended_at(
        TableId::OccupationSettings,
        OCCUPATION_GROUP_LAWS_FORMAT,
        "occupation-group-law-union",
        &[
            AppendedField {
                index: 3,
                name: "occupied",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "group_laws",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fallback_law",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
        ],
    );
    history[TableId::ScientistDaily.index()] = table(
        TableId::ScientistDaily,
        RESEARCH_PROJECT_DETAILS_FORMAT,
        "daily scientist observations",
    );
    history[TableId::ScientistDaily.index()].introduced = RESEARCH_PROJECT_DETAILS_FORMAT;
    history[TableId::Scientists.index()] = appended_at(
        TableId::Scientists,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[AppendedField {
            index: 4,
            name: "active",
            introduced: EFFECT_INPUTS_FORMAT,
        }],
    );
    history[TableId::CountryLeaderRoles.index()] = appended_at(
        TableId::CountryLeaderRoles,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[AppendedField {
            index: 1,
            name: "current_ideology",
            introduced: EFFECT_INPUTS_FORMAT,
        }],
    );
    history[TableId::AirWings.index()] = appended_at(
        TableId::AirWings,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 15,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "pool_subunit",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "raid_reference",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "strategic_bombing_raw",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "acquired_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 20,
                name: "exile_country",
                introduced: 104,
            },
            AppendedField {
                index: 21,
                name: "replacement_setting",
                introduced: DESIGN_STATE_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "map_flags",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 23,
                name: "fuel_base_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "fuel_current_type",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "fuel_effective_aircraft",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "fuel_context_mode",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "fuel_capacity_class",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "fuel_override_resolved",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AceRangeInputs.index()] = appended_at(
        TableId::AceRangeInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy normalization projection",
        &[
            AppendedField {
                index: 4,
                name: "normalization",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fuel_modifier",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "combat_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "speed_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "attack_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "defence_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "type_speed_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "type_combat_flags",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::VolunteerHosts.index()] = appended_at(
        TableId::VolunteerHosts,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy volunteer relation modifier",
        &[AppendedField {
            index: 2,
            name: "air_capacity_modifier",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::RaidCountryInputs.index()] = appended_at(
        TableId::RaidCountryInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy volunteer stockpile total",
        &[AppendedField {
            index: 4,
            name: "volunteer_stockpile_planes",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::AirSortieModifiers.index()] = table(
        TableId::AirSortieModifiers,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy certified detection overrides",
    );
    history[TableId::AirMissionInputs.index()] = appended_at(
        TableId::AirMissionInputs,
        AIR_MAP_FORMAT,
        "pending political runtime inputs and pol/mapmode-air-navy mission operands",
        &[
            AppendedField {
                index: 6,
                name: "cas_ratio_raw",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "interception_alert_gate",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "current_type",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "effective_aircraft",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "normalization",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "detection_stat_13",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "detection_stat_14",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "patrol_disruption_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "patrol_attack_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "patrol_defence_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "patrol_speed_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "patrol_wing_attack_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "patrol_wing_defence_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "patrol_night_applied_raw",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AirMissionInputs.index()].introduced = AIR_MAP_FORMAT;
    history[TableId::AirNavalPatrolHistories.index()] = appended_at(
        TableId::AirNavalPatrolHistories,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 2,
            name: "scenario_channels",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ShipModels.index()] = appended_at(
        TableId::ShipModels,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy repair eligibility",
        &[
            AppendedField {
                index: 6,
                name: "subunit_index",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "strike",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "hull_variant",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "has_criticals",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "air_capacity_kind",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "naval_power",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::TaskForceCounterInputs.index()] = appended_at(
        TableId::TaskForceCounterInputs,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 10,
                name: "name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "supply",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "port_mission",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::RailwayGunCounterInputs.index()] = appended_at(
        TableId::RailwayGunCounterInputs,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 23,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "eta",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "supply",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "withdrawal",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "fresh",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "combat_references",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "transport",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::TaskForceMapMotion.index()] = appended_at(
        TableId::TaskForceMapMotion,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 4,
                name: "movement_numerator",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "retreat",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "withdrawal",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "naval_transfer",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "combat_links",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "fresh",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::TimedIdeas.index()] =
        table(TableId::TimedIdeas, EFFECT_INPUTS_FORMAT, "ffd3e03cc6");
    history[TableId::TimedIdeas.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::DecisionRandomItems.index()] = table(
        TableId::DecisionRandomItems,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::DecisionRandomItems.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::CapturedOperativeReferences.index()] = table(
        TableId::CapturedOperativeReferences,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::CapturedOperativeReferences.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::ReservedCountryNaming.index()] = appended_at(
        TableId::ReservedCountryNaming,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 8,
                name: "original_tag",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "original_tag_reference",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "original_tag_buckets",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "country_array",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "owned_state_count",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "dynamic_tag_boundary",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "faction",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "movement",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "land_theatre_count",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "launch_sources",
                // The native-null naval discriminator preserves the original
                // unavailable/list bytes and this historical field boundary.
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "reverse_visibility",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "country_intel_sample",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "raw_country_observations",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "transport_mission_inputs",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::ReservedCountryNaming.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::AttritionModifierBreakdowns.index()] = table(
        TableId::AttritionModifierBreakdowns,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::AttritionModifierBreakdowns.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::AirBases.index()] = appended_at(
        TableId::AirBases,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 5,
            name: "carrier",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::AirBaseRegistrations.index()] = appended_at(
        TableId::AirBaseRegistrations,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 2,
                name: "carrier",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "transport_planes",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "fuel_demand_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fuel_allocated_raw",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::CountryProductionModifiers.index()] = table(
        TableId::CountryProductionModifiers,
        PRODUCTION_MODIFIERS_FORMAT,
        "production-modifiers",
    );
    history[TableId::CountryProductionModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
    history[TableId::ProductionLineModifiers.index()] = table(
        TableId::ProductionLineModifiers,
        PRODUCTION_MODIFIERS_FORMAT,
        "production-modifiers",
    );
    history[TableId::ProductionLineModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
    let air_ledgers = [
        TableId::AirLossEvents,
        TableId::BuildingBombingDamage,
        TableId::AirDailyRoster,
    ];
    let mut ledger = 0;
    while ledger < air_ledgers.len() {
        let id = air_ledgers[ledger];
        history[id.index()] = LastChange {
            introduced: AIR_LEDGER_FORMAT,
            ..table(id, AIR_LEDGER_FORMAT, "7fa2b75e16")
        };
        ledger += 1;
    }
    history[TableId::AirRegionCombat.index()] = LastChange {
        introduced: AIR_REGION_COMBAT_FORMAT,
        ..table(
            TableId::AirRegionCombat,
            AIR_REGION_COMBAT_FORMAT,
            "restore/experience",
        )
    };
    history[TableId::NuclearStrikes.index()] = LastChange {
        introduced: NUCLEAR_STRIKES_FORMAT,
        ..table(
            TableId::NuclearStrikes,
            NUCLEAR_STRIKES_FORMAT,
            "record/nuclear-strikes",
        )
    };
    history[TableId::LandCombatDivisionModifiers.index()] = LastChange {
        introduced: LAND_MODIFIER_ROWS_FORMAT,
        ..table(
            TableId::LandCombatDivisionModifiers,
            LAND_MODIFIER_ROWS_FORMAT,
            "perf/land-modifiers",
        )
    };
    history[TableId::FuelDemandParts.index()] = LastChange {
        introduced: FUEL_DEMAND_PARTS_FORMAT,
        ..table(
            TableId::FuelDemandParts,
            FUEL_DEMAND_PARTS_FORMAT,
            "perf/fuel-air-rows",
        )
    };
    history[TableId::MapEvents.index()] = LastChange {
        introduced: MAP_EVENTS_FORMAT,
        ..table(
            TableId::MapEvents,
            SCRIPTED_MAP_FORMAT,
            "pol/mapmode-scripted refresh family and interval health",
        )
    };
    history[TableId::OwnerTransportSpeeds.index()] = LastChange {
        introduced: OWNER_TRANSPORT_SPEEDS_FORMAT,
        ..table(
            TableId::OwnerTransportSpeeds,
            OWNER_TRANSPORT_SPEEDS_FORMAT,
            "pol/finish-smallcap",
        )
    };
    history[TableId::ReserveModifierTrees.index()] = LastChange {
        introduced: RESERVE_MODIFIER_TREES_FORMAT,
        ..table(
            TableId::ReserveModifierTrees,
            RESERVE_MODIFIER_TREES_FORMAT,
            "pol/finish-smallcap",
        )
    };
    history[TableId::BattleSounds.index()] = LastChange {
        introduced: BATTLE_SOUNDS_FORMAT,
        ..table(
            TableId::BattleSounds,
            BATTLE_SOUNDS_FORMAT,
            "pol/finish-audio",
        )
    };
    history[TableId::OperativeMissions.index()] = LastChange {
        introduced: OPERATIVE_MISSIONS_FORMAT,
        ..table(
            TableId::OperativeMissions,
            OPERATIVE_MISSIONS_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::AgencyOperationBuffers.index()] = LastChange {
        introduced: AGENCY_OPERATION_BUFFERS_FORMAT,
        ..table(
            TableId::AgencyOperationBuffers,
            AGENCY_OPERATION_BUFFERS_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::StrategicOperatives.index()] = LastChange {
        introduced: STRATEGIC_OPERATIVES_FORMAT,
        ..table(
            TableId::StrategicOperatives,
            STRATEGIC_OPERATIVES_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::CombatLogGroups.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogGroups,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogMonths.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogMonths,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogManpower.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogManpower,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogTemplates.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogTemplates,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogReferences.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogReferences,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattles.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattles,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogEnemyEquipment.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogEnemyEquipment,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleSides.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleSides,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleEquipment.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleEquipment,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleModifiers.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleModifiers,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleCountries.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleCountries,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogObservers.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogObservers,
            COMBAT_LOG_OBSERVER_KEYS_FORMAT,
            "combat-log-observer-keys",
        )
    };
    history[TableId::ResourceOrigins.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::ResourceOrigins,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::ResourceRights.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::ResourceRights,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::CountryOilFactors.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::CountryOilFactors,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::StateTemporaryResources.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::StateTemporaryResources,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::ConvoyClients.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::ConvoyClients,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::TradeRoutes.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::TradeRoutes,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::DivisionPanelStats.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..table(
            TableId::DivisionPanelStats,
            EXPERIENCE_FORMAT,
            "pol/capture-experience",
        )
    };
    history[TableId::DivisionPanelSources.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..table(
            TableId::DivisionPanelSources,
            EXPERIENCE_FORMAT,
            "pol/capture-experience",
        )
    };
    history[TableId::DivisionHistory.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..table(
            TableId::DivisionHistory,
            EXPERIENCE_FORMAT,
            "pol/capture-experience",
        )
    };
    history[TableId::RegionalConvoyEfficiency.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::RegionalConvoyEfficiency,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::MovementStateTerms.index()] = LastChange {
        introduced: MOVEMENT_STATE_TERMS_FORMAT,
        ..table(
            TableId::MovementStateTerms,
            MOVEMENT_STATE_TERMS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CoreSupplyAccess.index()] = LastChange {
        introduced: CORE_SUPPLY_ACCESS_FORMAT,
        ..table(
            TableId::CoreSupplyAccess,
            CORE_SUPPLY_ACCESS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CountryUnitSpeedEntries.index()] = LastChange {
        introduced: COUNTRY_UNIT_SPEED_ENTRIES_FORMAT,
        ..table(
            TableId::CountryUnitSpeedEntries,
            COUNTRY_UNIT_SPEED_ENTRIES_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::RiverRelations.index()] = LastChange {
        introduced: RIVER_RELATIONS_FORMAT,
        ..table(
            TableId::RiverRelations,
            RIVER_RELATIONS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::DivisionTransportSpeeds.index()] = LastChange {
        introduced: DIVISION_TRANSPORT_SPEEDS_FORMAT,
        ..table(
            TableId::DivisionTransportSpeeds,
            DIVISION_TRANSPORT_SPEEDS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CountryMarket.index()] = LastChange {
        introduced: 104,
        ..table(TableId::CountryMarket, 104, "pol/capture-market-lendlease")
    };
    history[TableId::MarketContractTerms.index()] = LastChange {
        introduced: 104,
        ..table(
            TableId::MarketContractTerms,
            104,
            "pol/capture-market-lendlease",
        )
    };
    history[TableId::LendLeaseAgreements.index()] = LastChange {
        introduced: 104,
        ..table(
            TableId::LendLeaseAgreements,
            104,
            "pol/capture-market-lendlease",
        )
    };
    history[TableId::MarketEvents.index()] = LastChange {
        introduced: 104,
        ..table(TableId::MarketEvents, 104, "pol/capture-market-lendlease")
    };
    let naval = [
        TableId::NavalRepairCountries,
        TableId::NavalRepairFleets,
        TableId::NavalRepairTaskForces,
        TableId::NavalTaskForceComposition,
        TableId::NavalRepairBases,
        TableId::NavalRepairQueue,
        TableId::NavalRefitShips,
        TableId::NavalRefitProgress,
        TableId::NavalRepairChildren,
    ];
    let mut index = 0;
    while index < naval.len() {
        let id = naval[index];
        history[id.index()] = LastChange {
            introduced: 104,
            ..table(id, 104, "pol/capture-naval-repair")
        };
        index += 1;
    }
    // Shared immutable headquarters membership uses the owned-value field
    // framing; the current mission layout includes its raw spotting suffix.
    history[TableId::NavalMissionTaskForces.index()] = table(
        TableId::NavalMissionTaskForces,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy shared mission source membership",
    );
    history[TableId::NavalMissionCommanders.index()] = appended_at(
        TableId::NavalMissionCommanders,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy mission modifier sources",
        &[
            AppendedField {
                index: 4,
                name: "detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "night_spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "visibility",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "speed",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::NavalMissionCountries.index()] = appended_at(
        TableId::NavalMissionCountries,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy mission sources",
        &[
            AppendedField {
                index: 8,
                name: "detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "night_spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "visibility",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "escort_tree",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "speed",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AirGroundCrews.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::AirGroundCrews,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy certified Ground Crews",
        )
    };
    history[TableId::AirTransferInputs.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::AirTransferInputs,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy incoming Air transfers",
        )
    };
    history[TableId::NavalAccidentReports.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalAccidentReports,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy retained naval accidents",
        )
    };
    history[TableId::NavalRegionalMines.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalRegionalMines,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy complete regional mines",
        )
    };
    history[TableId::NavalDominanceTaskForceRanges.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalDominanceTaskForceRanges,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy demanded strike-force sibling reach",
        )
    };
    history[TableId::NavalPatrolMissionInputs.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalPatrolMissionInputs,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy patrol detection operands",
        )
    };
    history[TableId::VariantDefinitions.index()] = appended_at(
        TableId::VariantDefinitions,
        DESIGN_STATE_FORMAT,
        "pol/capture-designs",
        &[
            AppendedField {
                index: 14,
                name: "version",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "design",
                introduced: DESIGN_STATE_FORMAT,
            },
        ],
    );
    history[TableId::VariantChanges.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::VariantChanges,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::VariantBonusSnapshots.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::VariantBonusSnapshots,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignCountryModifiers.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignCountryModifiers,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignCountryTemplates.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignCountryTemplates,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };

    history[TableId::DesignOrganisationLiveness.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignOrganisationLiveness,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignObserverContext.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignObserverContext,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::ScriptVariables.index()] = appended(
        TableId::ScriptVariables,
        "c8c5165b33",
        &[AppendedField {
            index: 1,
            name: "seed",
            introduced: RECORDER_INPUTS_FORMAT,
        }],
    );
    history[TableId::TrainInventory.index()] = appended(
        TableId::TrainInventory,
        "c8c5165b33",
        &[
            AppendedField {
                index: 3,
                name: "allocated_trucks",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "required_trucks_raw",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "required_trains",
                introduced: RECORDER_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].introduced =
        PROVEN_FROM;
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].kind =
        ChangeKind::AppendedOnly;
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].appended =
        &[AppendedField {
            index: 17,
            name: "NavalCommanderInputs.timed_traits",
            introduced: RECORDER_INPUTS_FORMAT,
        }];
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].introduced = PROVEN_FROM;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].kind =
        ChangeKind::AppendedOnly;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].appended =
        &[AppendedField {
            index: 2,
            name: "LandBorderWarInputs.combat_width",
            introduced: RECORDER_INPUTS_FORMAT,
        }];
    history[TableId::ALL.len() + RecordKind::EventFeedEvent as usize].introduced =
        WIDGET_CAPTURE_FORMAT;
    history[TableId::ALL.len() + RecordKind::EventFeedEvent as usize].kind = ChangeKind::Layout;
    history[TableId::CountryFuel.index()] = appended_at(
        TableId::CountryFuel,
        ECONOMY_INPUTS_FORMAT,
        "a72768d69e",
        &[
            AppendedField {
                index: 19,
                name: "country_instance",
                introduced: ECONOMY_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "awards",
                introduced: ECONOMY_INPUTS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "demand",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "award_day",
                introduced: FUEL_AWARD_DAY_FORMAT,
            },
        ],
    );
    history[TableId::LogisticsEquipment.index()] = appended_at(
        TableId::LogisticsEquipment,
        ECONOMY_INPUTS_FORMAT,
        "afa36ffdef",
        &[AppendedField {
            index: 3,
            name: "available_raw",
            introduced: ECONOMY_INPUTS_FORMAT,
        }],
    );
    history[TableId::NavalBases.index()] = appended_at(
        TableId::NavalBases,
        POLITICAL_FORMAT,
        "naval base serialized country tag witnesses (provisional)",
        &[AppendedField {
            index: 5,
            name: "country_tags",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ConstructionLineOutput.index()] = appended_at(
        TableId::ConstructionLineOutput,
        POLITICAL_FORMAT,
        "construction completion and count-reset witnesses (provisional)",
        &[
            AppendedField {
                index: 13,
                name: "completion",
                introduced: CONSTRUCTION_COMPLETION_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "count_reset",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::StateGarrisons.index()] = appended_at(
        TableId::StateGarrisons,
        WIDGET_CAPTURE_FORMAT,
        "garrison-strength-operands",
        &[AppendedField {
            index: 5,
            name: "equipment",
            introduced: WIDGET_CAPTURE_FORMAT,
        }],
    );
    // Signed net rows preserve every historical positive amount. Missing new
    // columns remain field gaps; correctly decoded old prefixes still open.
    history[TableId::EquipmentDestruction.index()] = appended_at(
        TableId::EquipmentDestruction,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 1,
                name: "coverage",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "unavailable_reason",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::Fleets.index()] = appended_at(
        TableId::Fleets,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 2,
                name: "name",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "leader",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::OrdersGroups.index()] = appended_at(
        TableId::OrdersGroups,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[AppendedField {
            index: 9,
            name: "name",
            introduced: WIDGET_CAPTURE_FORMAT,
        }],
    );
    history[TableId::LandCombatProgress.index()] = appended_at(
        TableId::LandCombatProgress,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 4,
                name: "attacker_manpower_loss",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "defender_manpower_loss",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "attacker_equipment_losses",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "defender_equipment_losses",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::DivisionEquipmentDeletions.index()] = table(
        TableId::DivisionEquipmentDeletions,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DivisionEquipmentDeletions.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::DivisionLandInputs.index()] = table(
        TableId::DivisionLandInputs,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DivisionLandInputs.index()].introduced = WIDGET_CAPTURE_FORMAT;
    // Its retired supply copies keep their framing: every layout decodes.
    history[TableId::DivisionLandInputs.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatResults.index()] = table(
        TableId::LandCombatResults,
        COMBAT_LOG_FORMAT,
        "gap-combat-log",
    );
    history[TableId::LandCombatResults.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::DestroyedDivisions.index()] = table(
        TableId::DestroyedDivisions,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DestroyedDivisions.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::LandEventCoverage.index()] = table(
        TableId::LandEventCoverage,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::LandEventCoverage.index()].introduced = WIDGET_CAPTURE_FORMAT;
    // Independent suffixes preserve existing payloads and retired inputs.
    history[TableId::CountryCounterInputs.index()].format = EXPERIENCE_FORMAT;
    history[TableId::DivisionMovement.index()].format = EXPERIENCE_FORMAT;
    history[TableId::DivisionStatus.index()].format = EXPERIENCE_FORMAT;
    history[TableId::UnitLeaders.index()].format = EXPERIENCE_FORMAT;
    history[TableId::UnitLeaders.index()].kind = ChangeKind::Meaning;
    history[TableId::UnitLeaderTraitExperience.index()] = table(
        TableId::UnitLeaderTraitExperience,
        EXPERIENCE_FORMAT,
        "capture/experience",
    );
    history[TableId::IndustrialOrganisationPolicies.index()] = LastChange {
        introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        ..table(
            TableId::IndustrialOrganisationPolicies,
            INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            "pol/capture-mios",
        )
    };
    history[TableId::IndustrialOrganisationDetails.index()] = LastChange {
        introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        ..table(
            TableId::IndustrialOrganisationDetails,
            INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            "pol/capture-mios",
        )
    };
    history[TableId::IndustrialOrganisationHistory.index()] = appended_at(
        TableId::IndustrialOrganisationHistory,
        INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        "pol/capture-mios",
        &[AppendedField {
            index: 1,
            name: "design_hour",
            introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        }],
    );
    history[TableId::VariantStats.index()] = appended_at(
        TableId::VariantStats,
        INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        "pol/capture-mios",
        &[
            AppendedField {
                index: 10,
                name: "organisation",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "organisation_trait_count",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "organisation_bonuses",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
        ],
    );
    // These raw suffixes cannot represent NotRecorded. Keep their wire prefix
    // framing in the catalogue without promising an absent-field row decoder.
    history[TableId::IndustrialOrganisationHistory.index()].kind = ChangeKind::Layout;
    history[TableId::VariantStats.index()].kind = ChangeKind::Layout;
    history[TableId::IntelNetworkSummaries.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelNetworkSummaries,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::PropagandaEfforts.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::PropagandaEfforts,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    // Source tables preserve empty extents after certified clears, including
    // the daily allied clear outside sharing replacements. Local UI estimates
    // have no recording contract; bounds derive from game-state operands.
    history[TableId::IntelContributions.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelContributions,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::IntelNetworkAuxiliaries.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelNetworkAuxiliaries,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::IntelSources.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelSources,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    let mut participation = 0;
    while participation < 3 {
        let id = [
            TableId::WarParticipation,
            TableId::WarCapturedProvinces,
            TableId::WarLendLeaseHistories,
        ][participation];
        history[id.index()] = table(id, 104, "capture/war-participation");
        history[id.index()].introduced = 104;
        participation += 1;
    }
    apply_catalog_passes(&mut history);
    history[TableId::StateResourceInputs.index()] = appended_at(
        TableId::StateResourceInputs,
        104,
        "pol/capture-manpower",
        &[
            AppendedField {
                index: 6,
                name: "local_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 7,
                name: "local_noncore_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "recruitment_add",
                introduced: 104,
            },
            AppendedField {
                index: 9,
                name: "recruitment_factor",
                introduced: 104,
            },
        ],
    );
    history[TableId::Ships.index()] = appended_at(
        TableId::Ships,
        104,
        "pol/capture-manpower",
        &[
            AppendedField {
                index: 7,
                name: "acquired_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "required_manpower",
                introduced: 104,
            },
        ],
    );
    declare_change(
        &mut history[TableId::Ships.index()],
        table(
            TableId::Ships,
            107,
            "live ship cache and saved naming operands",
        ),
    );
    history[TableId::CountryStabilityInputs.index()] = appended_at(
        TableId::CountryStabilityInputs,
        104,
        "country manpower and exile operands",
        &[
            AppendedField {
                index: 37,
                name: "recruitment_ratio",
                introduced: 104,
            },
            AppendedField {
                index: 38,
                name: "exile_host",
                introduced: 104,
            },
            AppendedField {
                index: 39,
                name: "exile_legitimacy",
                introduced: 104,
            },
            AppendedField {
                index: 40,
                name: "exile_leaders",
                introduced: 104,
            },
            AppendedField {
                index: 41,
                name: "exile_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 42,
                name: "exile_manpower_cap",
                introduced: 104,
            },
        ],
    );

    history
};

// Common changes can follow declarations added by another capture family.
// Their older formats must not erase a later field, layout or its attribution.
const fn apply_catalog_passes(
    history: &mut [LastChange; TableId::ALL.len() + RecordKind::ALL.len()],
) {
    // Hourly hosts' nested packets stopped storing their read hour. The
    // stamped nested layout decodes through its source format, so these
    // tables stay readable and only their nested records changed.
    let mut host = 0;
    while host < super::PACKET_HOUR_HOSTS.len() {
        let id = super::PACKET_HOUR_HOSTS[host];
        note_change(
            &mut history[id.index()],
            PACKET_HOUR_FORMAT,
            "perf/stamps packet hours",
        );
        host += 1;
    }
    // The singleton's transport packet was split into hourly tables in this
    // provisional format, and both retired in it; no published build wrote
    // the tables.
    note_change(
        &mut history[TableId::ReservedCountryNaming.index()],
        RESERVED_SPLIT_FORMAT,
        "perf/reserved-split",
    );
    let mut split = 0;
    while split < super::RESERVED_SPLIT_TABLES.len() {
        let id = super::RESERVED_SPLIT_TABLES[split];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: RESERVED_SPLIT_FORMAT,
                ..table(id, RESERVED_SPLIT_FORMAT, "perf/reserved-split")
            },
        );
        split += 1;
    }
    // The nearest-cache tables were introduced and retired in this same
    // provisional format; no published build wrote them.
    let mut nearest = 0;
    while nearest < 2 {
        let id = [
            TableId::ReservedNearestNavies,
            TableId::ReservedNearestBases,
        ][nearest];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: RESERVED_SPLIT_FORMAT,
                ..table(id, RESERVED_SPLIT_FORMAT, "perf/reserved-split")
            },
        );
        nearest += 1;
    }
    // The naval hover source trees and their diagnostic oracle samples.
    let mut naval = 0;
    while naval < 2 {
        let id = [
            TableId::NavalModifierTrees,
            TableId::NavalModifierTreeSamples,
        ][naval];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: NAVAL_TREES_FORMAT,
                ..table(id, NAVAL_TREES_FORMAT, "restore/naval")
            },
        );
        naval += 1;
    }
    declare_change(
        &mut history[TableId::DivisionRetreatBans.index()],
        LastChange {
            kind: ChangeKind::Meaning,
            ..table(
                TableId::DivisionRetreatBans,
                RETREAT_BANS_FORMAT,
                "cadence/raid",
            )
        },
    );
    declare_change(
        &mut history[TableId::StockpileArchetypes.index()],
        table(
            TableId::StockpileArchetypes,
            ESSENTIAL_STOCK_FORMAT,
            "cadence/raid",
        ),
    );
    // Arrival-time, attrition-breakdown, selected-attack and selected
    // transport inputs retire. Tables keep their ids and framing so older
    // recordings discard them by name.
    // Hover-only breakdowns, oracle-only histories and the owned-wargoal
    // roster only effect-description hovers read retire the same way.
    let mut r = 0;
    while r < RETIRED_TABLES.len() {
        history[RETIRED_TABLES[r].index()].retired = Some(RETIRED_INPUTS_FORMAT);
        r += 1;
    }
    let mut field = 0;
    while field < RETIRED_FIELDS.len() {
        let id = RETIRED_FIELDS[field].table;
        // A tombstone keeps its field's framing, so every layout since the
        // table's last layout change still decodes. A table whose layout
        // changed within its lifetime keeps that refusal and its format.
        let entry = history[id.index()];
        let earlier_layout = !matches!(entry.kind, ChangeKind::AppendedOnly)
            && !matches!(entry.kind, ChangeKind::Unchanged)
            && entry.format > entry.introduced;
        if earlier_layout {
            field += 1;
            continue;
        }
        history[id.index()].kind = ChangeKind::AppendedOnly;
        note_change(
            &mut history[id.index()],
            RETIRED_INPUTS_FORMAT,
            "perf/eta-tooltips, perf/supply-dedupe, perf/retire-tooltips and perf/retire-previews retired inputs",
        );
        field += 1;
    }
    note_change(
        &mut history[TableId::DivisionStatus.index()],
        RETIRED_INPUTS_FORMAT,
        "perf/supply-dedupe, perf/retire-tooltips retired inputs, perf/division-status-2 camouflage, perf/attack-preview inputs, restore/supply footer, pol/oracle-base-diffs name template",
    );
}

const fn note_change(entry: &mut LastChange, format: u16, commit: &'static str) {
    if format >= entry.format {
        entry.format = format;
        entry.commit = commit;
    }
}

const fn declare_change(entry: &mut LastChange, declaration: LastChange) {
    if declaration.format >= entry.format {
        *entry = declaration;
    }
}

const fn appended(
    id: TableId,
    commit: &'static str,
    fields: &'static [AppendedField],
) -> LastChange {
    appended_at(id, RECORDER_INPUTS_FORMAT, commit, fields)
}

const fn appended_at(
    id: TableId,
    format: u16,
    commit: &'static str,
    fields: &'static [AppendedField],
) -> LastChange {
    let mut change = table(id, format, commit);
    change.kind = ChangeKind::AppendedOnly;
    change.appended = fields;
    change
}

const fn table(id: TableId, format: u16, commit: &'static str) -> LastChange {
    LastChange {
        subject: Subject::Table(id),
        introduced: PROVEN_FROM,
        retired: None,
        format,
        commit,
        kind: ChangeKind::Layout,
        appended: &[],
    }
}

impl LastChange {
    pub const fn present(&self, format: u16) -> bool {
        format >= self.introduced
            && match self.retired {
                Some(retired) => format < retired,
                None => true,
            }
    }

    pub fn table_gap(&self, table: TableId, format: u16) -> Option<String> {
        if format < self.introduced {
            Some(format!(
                "{table:?} not recorded at format {format} (introduced at format {})",
                self.introduced
            ))
        } else if let Some(retired) = self
            .retired
            .filter(|retired| *retired <= crate::FORMAT_VERSION)
        {
            Some(format!(
                "{table:?} retired at format {retired}; this recording is format {format}"
            ))
        } else if format < self.format && self.kind != ChangeKind::AppendedOnly {
            Some(format!(
                "{table:?} layout or meaning changed at format {}; this recording is format {format}",
                self.format
            ))
        } else {
            None
        }
    }

    pub fn fields(&self, table: TableId, format: u16) -> usize {
        self.appended
            .iter()
            .find(|field| format < field.introduced)
            .map_or(table.field_count(), |field| field.index)
    }
}

impl TableId {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[self.index()]
    }
}

impl RecordKind {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[TableId::ALL.len() + self as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An older recording's DivisionRetreatBans are each owner's daily read,
    /// not the hour's bans, and its StockpileArchetypes every archetype's raw
    /// total: it refuses both by name.
    #[test]
    fn older_retreat_bans_and_stock_are_refused() {
        for table in [TableId::DivisionRetreatBans, TableId::StockpileArchetypes] {
            assert!(
                table
                    .last_change()
                    .table_gap(table, POLITICAL_FORMAT)
                    .unwrap()
                    .contains(&format!("{table:?}"))
            );
            assert!(
                table
                    .last_change()
                    .table_gap(table, FORMAT_VERSION)
                    .is_none()
            );
        }
    }

    #[test]
    fn technology_key_changes_refuse_only_the_changed_contracts() {
        let historical = WIDGET_CAPTURE_FORMAT;
        for table in [
            TableId::ResearchTechnologies,
            TableId::TechnologyDefinitions,
        ] {
            assert!(
                table
                    .last_change()
                    .table_gap(table, historical)
                    .unwrap()
                    .contains(&format!("{table:?}"))
            );
            assert!(
                table
                    .last_change()
                    .table_gap(table, FORMAT_VERSION)
                    .is_none()
            );
        }
        for table in [
            TableId::ResearchSlots,
            TableId::ResearchInputs,
            TableId::Countries,
            TableId::Clock,
        ] {
            assert!(table.last_change().table_gap(table, historical).is_none());
        }
        assert_eq!(
            TableId::ResearchSlots
                .last_change()
                .fields(TableId::ResearchSlots, historical),
            5
        );
        assert_eq!(
            TableId::ResearchInputs
                .last_change()
                .fields(TableId::ResearchInputs, historical),
            15
        );
    }

    #[test]
    fn native_war_operands_are_named_gaps_before_their_introduction() {
        for table in [
            TableId::WarOwnedRelations,
            TableId::WarRelationGoals,
            TableId::WarOverviewCountries,
            TableId::WarNameRules,
        ] {
            let history = table.last_change();
            for format in PROVEN_FROM..WAR_OVERVIEW_FORMAT {
                assert!(!history.present(format));
                assert!(
                    history
                        .table_gap(table, format)
                        .unwrap()
                        .contains("not recorded")
                );
            }
            assert!(history.present(WAR_OVERVIEW_FORMAT));
            assert!(history.table_gap(table, WAR_OVERVIEW_FORMAT).is_none());
        }
    }

    #[test]
    fn peace_actions_are_named_gaps_before_their_introduction() {
        for table in [TableId::PeaceSettlements, TableId::PeaceBids] {
            let history = table.last_change();
            for format in PROVEN_FROM..PEACE_SETTLEMENTS_FORMAT {
                assert!(!history.present(format));
                assert!(
                    history
                        .table_gap(table, format)
                        .unwrap()
                        .contains("not recorded")
                );
            }
            assert!(history.present(PEACE_SETTLEMENTS_FORMAT));
            assert!(history.table_gap(table, PEACE_SETTLEMENTS_FORMAT).is_none());
        }
    }

    #[test]
    fn retired_inputs_refuse_by_name_in_every_recording() {
        for table in RETIRED_TABLES {
            let history = table.last_change();
            assert!(!history.present(FORMAT_VERSION), "{table:?}");
            assert!(
                history
                    .table_gap(table, FORMAT_VERSION)
                    .unwrap()
                    .contains("retired at format")
            );
        }
        // Older recordings held the arrival and attrition tables; they are
        // framed for discard and refused by the same name.
        for table in [
            TableId::DivisionEtaInputs,
            TableId::AttritionModifierBreakdowns,
            TableId::ReserveModifierBreakdowns,
            TableId::NavalModifierBreakdowns,
            TableId::OwnedWargoals,
            TableId::AirRegionHistory,
            TableId::AirRegionDamage,
        ] {
            let history = table.last_change();
            assert!(history.present(POLITICAL_FORMAT));
            assert!(
                history
                    .table_gap(table, POLITICAL_FORMAT)
                    .unwrap()
                    .contains(&format!(
                        "{table:?} retired at format {RETIRED_INPUTS_FORMAT}"
                    ))
            );
        }
        // Tables holding retired fields stay readable in every recording.
        for field in RETIRED_FIELDS {
            let history = field.table.last_change();
            assert_eq!(history.kind, ChangeKind::AppendedOnly, "{:?}", field.table);
            let current = history
                .appended
                .iter()
                .map(|operand| operand.introduced)
                .max()
                .unwrap_or(RETIRED_INPUTS_FORMAT)
                .max(RETIRED_INPUTS_FORMAT);
            assert_eq!(history.format, current, "{:?}", field.table);
            assert!(history.table_gap(field.table, POLITICAL_FORMAT).is_none());
            assert!(history.table_gap(field.table, FORMAT_VERSION).is_none());
        }
    }

    #[test]
    fn combat_log_observer_semantic_keys_and_numeric_lanes_change_at_106() {
        let table = TableId::CombatLogObservers;
        let history = table.last_change();
        assert_eq!(history.introduced, COMBAT_LOG_FORMAT);
        assert_eq!(history.format, COMBAT_LOG_OBSERVER_KEYS_FORMAT);
        assert_eq!(history.kind, ChangeKind::Layout);
        assert!(history.table_gap(table, 105).is_some());
        assert!(history.table_gap(table, 106).is_none());
    }

    #[test]
    fn resource_format_keeps_mains_table_and_field_introductions() {
        assert_eq!(RESOURCES_MAP_FORMAT, 103);
        assert_eq!(TableId::TradeRoutes.field_count(), 6);
        let route_history = TableId::TradeRoutes.last_change();
        assert_eq!((route_history.introduced, route_history.format), (103, 103));
        assert!(route_history.appended.is_empty());
        for format in [
            STATE_CATEGORY_FORMAT,
            STRATEGIC_LOCATIONS_FORMAT,
            BATTLE_SOUNDS_FORMAT,
            OPERATIVE_MISSIONS_FORMAT,
            STRATEGIC_OPERATIVES_FORMAT,
            AGENCY_OPERATION_BUFFERS_FORMAT,
            OPERATION_PREPARATION_FORMAT,
        ] {
            assert_eq!(format, 102);
        }
        assert!(TableId::BattleSounds.last_change().present(102));
        assert!(!TableId::ResourceOrigins.last_change().present(102));
        assert!(TableId::ResourceOrigins.last_change().present(103));
        assert_eq!(
            TableId::ResourceOrigins as u16,
            TableId::AgencyOperationBuffers as u16 + 1
        );
        assert_eq!(
            TableId::RegionalConvoyEfficiency as u16,
            TableId::ResourceOrigins as u16 + 6
        );
    }

    #[test]
    fn manpower_exile_field_has_its_current_format_catalog_entry() {
        let history = TableId::DivisionTemplateIcons.last_change();
        let field = history
            .appended
            .iter()
            .find(|field| field.index == 16)
            .unwrap();
        assert_eq!(field.name, "manpower_exile");
        assert_eq!(field.introduced, 104);
        assert_eq!(history.format, 104);
        assert_eq!(history.kind, ChangeKind::AppendedOnly);
    }

    #[test]
    fn later_appended_fields_survive_every_catalog_pass() {
        for kind in [
            ChangeKind::AppendedOnly,
            ChangeKind::Layout,
            ChangeKind::Meaning,
        ] {
            let mut history = HISTORY;
            for table in TableId::ALL {
                history[table.index()] = LastChange {
                    kind,
                    ..appended_at(
                        table,
                        FORMAT_VERSION,
                        "later declaration fixture",
                        &[AppendedField {
                            index: 0,
                            name: "later_operand",
                            introduced: FORMAT_VERSION,
                        }],
                    )
                };
            }

            apply_catalog_passes(&mut history);

            for table in TableId::ALL {
                let entry = history[table.index()];
                assert_eq!(entry.format, FORMAT_VERSION, "{table:?} {kind:?}");
                assert_eq!(entry.kind, kind, "{table:?}");
                assert_eq!(entry.introduced, PROVEN_FROM, "{table:?}");
                assert_eq!(entry.commit, "later declaration fixture", "{table:?}");
                assert_eq!(entry.appended.len(), 1, "{table:?}");
                assert_eq!(entry.appended[0].name, "later_operand", "{table:?}");
                assert_eq!(entry.appended[0].introduced, FORMAT_VERSION, "{table:?}");
                assert_eq!(
                    entry.retired,
                    RETIRED_TABLES
                        .contains(&table)
                        .then_some(RETIRED_INPUTS_FORMAT),
                    "{table:?}"
                );
            }
        }
    }

    #[test]
    fn occupation_layout_survives_catalog_passes() {
        // The occupation family appends opaque suffixes with whole-layout
        // admission. Exercise that declaration without importing its row data.
        let table = TableId::StateResistance;
        let mut history = HISTORY;
        history[table.index()] = LastChange {
            kind: ChangeKind::Layout,
            ..appended_at(
                table,
                OCCUPATION_MAP_FORMAT,
                "occupation layout fixture",
                &[
                    AppendedField {
                        index: 6,
                        name: "hover",
                        introduced: 104,
                    },
                    AppendedField {
                        index: 7,
                        name: "modifier_sources",
                        introduced: 104,
                    },
                    AppendedField {
                        index: 8,
                        name: "map",
                        introduced: OCCUPATION_MAP_FORMAT,
                    },
                ],
            )
        };

        apply_catalog_passes(&mut history);

        let entry = history[table.index()];
        assert_eq!(entry.format, OCCUPATION_MAP_FORMAT);
        assert_eq!(entry.kind, ChangeKind::Layout);
        assert_eq!(entry.commit, "occupation layout fixture");
        assert_eq!(
            entry
                .appended
                .iter()
                .map(|field| (field.index, field.name, field.introduced))
                .collect::<Vec<_>>(),
            [
                (6, "hover", 104),
                (7, "modifier_sources", 104),
                (8, "map", 105)
            ]
        );
        assert!(entry.table_gap(table, OCCUPATION_MAP_FORMAT).is_none());
    }

    #[test]
    fn occupation_map_suffix_has_current_layout_history() {
        let history = TableId::StateResistance.last_change();
        assert_eq!(history.format, OCCUPATION_MAP_FORMAT);
        assert_eq!(history.kind, ChangeKind::Layout);
        assert_eq!(TableId::StateResistance.field_count(), 9);
        assert_eq!(
            history
                .appended
                .iter()
                .map(|field| (field.index, field.name, field.introduced))
                .collect::<Vec<_>>(),
            [
                (6, "hover", 104),
                (7, "modifier_sources", 104),
                (8, "map", 105)
            ]
        );
    }

    #[test]
    fn wargoal_inventory_catalog_entries_survive_every_pass() {
        for table in [
            TableId::CountryWargoals,
            TableId::WargoalTargets,
            TableId::WargoalJustifications,
        ] {
            let entry = table.last_change();
            assert_eq!(entry.introduced, 104);
            assert_eq!(entry.format, 104);
            assert_eq!(entry.kind, ChangeKind::Layout);
            assert!(entry.table_gap(table, 103).is_some());
            assert!(entry.table_gap(table, 104).is_none());
        }
    }

    #[test]
    fn saved_design_tables_begin_at_their_declared_introduction() {
        for table in [
            TableId::VariantChanges,
            TableId::VariantBonusSnapshots,
            TableId::DesignCountryModifiers,
            TableId::DesignCountryTemplates,
            TableId::DesignObserverContext,
            TableId::DesignOrganisationLiveness,
        ] {
            let entry = table.last_change();
            assert_eq!(entry.introduced, DESIGN_STATE_FORMAT, "{table:?}");
            assert_eq!(entry.format, DESIGN_STATE_FORMAT, "{table:?}");
            assert_eq!(entry.kind, ChangeKind::Layout, "{table:?}");
            for version in [PROVEN_FROM, 97, DESIGN_STATE_FORMAT - 1] {
                assert!(!entry.present(version), "{table:?} in format {version}");
                assert_eq!(
                    entry.table_gap(table, version).unwrap(),
                    format!(
                        "{table:?} not recorded at format {version} (introduced at format {DESIGN_STATE_FORMAT})"
                    ),
                );
            }
            assert!(entry.present(DESIGN_STATE_FORMAT), "{table:?}");
            assert!(entry.table_gap(table, DESIGN_STATE_FORMAT).is_none());
            assert!(entry.present(FORMAT_VERSION), "{table:?}");
            assert!(entry.table_gap(table, FORMAT_VERSION).is_none());
        }
    }

    #[test]
    fn template_extensions_keep_their_format_after_packet_hour_changes() {
        for table in [
            TableId::DivisionTemplateIcons,
            TableId::OrdersGroupCounterInputs,
        ] {
            let entry = table.last_change();
            assert_eq!(entry.format, TEMPLATE_SETTINGS_FORMAT, "{table:?}");
            assert!(
                entry
                    .appended
                    .iter()
                    .all(|field| field.introduced <= entry.format)
            );
        }
    }

    #[test]
    fn scripted_map_catalog_records_cached_fields_and_expanded_event_layout() {
        for (table, fields) in [
            (
                TableId::States,
                &[(11, "best_vp_index"), (12, "single_controller")][..],
            ),
            (TableId::SimulationFeatures, &[(6, "state_array_len")][..]),
        ] {
            let history = table.last_change();
            assert_eq!(history.format, SCRIPTED_MAP_FORMAT);
            assert_eq!(history.kind, ChangeKind::AppendedOnly);
            for &(index, name) in fields {
                let field = history
                    .appended
                    .iter()
                    .find(|field| field.index == index)
                    .unwrap();
                assert_eq!((field.name, field.introduced), (name, SCRIPTED_MAP_FORMAT));
            }
        }
        let events = TableId::MapEvents.last_change();
        assert_eq!(events.introduced, MAP_EVENTS_FORMAT);
        assert_eq!(events.format, SCRIPTED_MAP_FORMAT);
        assert_eq!(events.kind, ChangeKind::Layout);
        assert_eq!(crate::rows::MapEventFamily::COUNT, 8);
    }

    #[test]
    fn compatibility_catalog_covers_every_table_and_record_once() {
        for table in TableId::ALL {
            let entry = table.last_change();
            assert_eq!(entry.subject, Subject::Table(table));
            assert!(!entry.commit.is_empty());
            assert!(entry.introduced <= entry.format);
            assert!(entry.format <= FORMAT_VERSION);
            // A provisional table can retire in the format that introduced
            // it; no published build ever wrote it.
            assert!(
                entry
                    .retired
                    .is_none_or(|retired| retired > entry.introduced
                        || (retired == entry.introduced && retired == RESERVED_SPLIT_FORMAT))
            );
            if entry.kind == ChangeKind::AppendedOnly {
                if entry.appended.is_empty() {
                    // Only framing-preserving retirements changed it.
                    assert!(
                        RETIRED_FIELDS.iter().any(|field| field.table == table),
                        "{table:?}"
                    );
                    continue;
                }
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(
                        field.introduced > PROVEN_FROM && field.introduced <= entry.format,
                        "{table:?}.{} introduced in {}, latest change {}",
                        field.name,
                        field.introduced,
                        entry.format
                    );
                }
                assert_eq!(first + entry.appended.len(), table.field_count());
            }
        }
        for kind in RecordKind::ALL {
            let entry = kind.last_change();
            assert_eq!(entry.subject, Subject::Record(kind));
            let fields = match kind {
                RecordKind::NavalCommanderInputs => {
                    Some(<super::super::NavalCommanderInputs as crate::value::Value>::STRUCT_FIELDS)
                }
                RecordKind::LandBorderWarInputs => {
                    Some(<super::super::LandBorderWarInputs as crate::value::Value>::STRUCT_FIELDS)
                }
                RecordKind::EventFeedEvent => {
                    assert_eq!(entry.kind, ChangeKind::Layout);
                    assert_eq!(entry.format, SCRIPT_EVENT_FORMAT);
                    assert!(entry.appended.is_empty());
                    None
                }
                _ => None,
            };
            if let Some(fields) = fields {
                assert!(
                    entry.introduced >= PROVEN_FROM
                        && entry.introduced < entry.appended[0].introduced
                );
                assert_eq!(entry.kind, ChangeKind::AppendedOnly);
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(field.name.contains('.'));
                    assert!(
                        field.introduced > PROVEN_FROM && field.introduced <= entry.format,
                        "{:?}.{} introduced {} after last change {}",
                        entry.subject,
                        field.name,
                        field.introduced,
                        entry.format
                    );
                }
                assert_eq!(first + entry.appended.len(), fields);
            }
        }
        let distinct: std::collections::BTreeSet<_> = HISTORY
            .iter()
            .map(|entry| format!("{:?}", entry.subject))
            .collect();
        assert_eq!(distinct.len(), HISTORY.len());
    }
}
