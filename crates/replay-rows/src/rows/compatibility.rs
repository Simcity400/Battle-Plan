//! Recording format numbers: the format each row and value change arrived in.

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
/// EventFeedHistory's owner-local refusal markers: a delayed owner's
/// DelayedRefused change (code 18) and a decision owner's Refused operation
/// (14), each carrying its reason as the token. The table's layout did not
/// change at this format, and the new codes appear only from it; the
/// occurrence deltas of EVENT_FEED_OCCURRENCE_DELTAS_FORMAT later replaced the
/// layout.
pub const EVENT_FEED_OWNER_REFUSALS_FORMAT: u16 = 109;
/// ProductionLineOutput rows carry each tick's purpose and split a line's
/// buckets by it; ProductionLineModifiers records the conversion control.
pub const CONVERSION_PURPOSE_FORMAT: u16 = 110;
/// Grouped-storage alias columns do not change public rows or field masks.
pub const PRIVATE_ALIASES_FORMAT: u16 = 113;
/// Combat, province and production alias marks leave native row schemas.
pub const PRIVATE_COPY_ALIASES_FORMAT: u16 = 114;
/// Owner-approved daily commander/observer progress and three division display columns.
pub const DAILY_PROGRESS_FORMAT: u16 = 116;
/// Stable semantic observer occurrences and independent signed numeric lanes.
pub const COMBAT_LOG_OBSERVER_KEYS_FORMAT: u16 = 106;
/// Owner-approved daily CountryUnitBonuses: every country's persistent unit-bonus list.
pub const UNIT_BONUSES_FORMAT: u16 = 117;
/// Owner-approved IndustrialOrganisationPolicies.cost_modifier_raw: each
/// country's effective policy-cost modifier, read in the daily MIO census.
pub const MIO_POLICY_COST_FORMAT: u16 = 118;
/// EventFeedHistory stores each occurrence as the operand words that differ
/// from the occurrence before it, and the writer keeps each distinct token
/// once per pack.
pub const EVENT_FEED_OCCURRENCE_DELTAS_FORMAT: u16 = 119;
/// Owner-approved DivisionPanelStats.suppression: the stat block's suppression word, the
/// division designer's witness of its suppression row, read in the panel cache's copy.
pub const PANEL_SUPPRESSION_FORMAT: u16 = 120;
/// Sessions and frames name their capture mode; frames omit the tables it leaves out.
pub const CAPTURE_MODE_FORMAT: u16 = 121;
/// Owner-approved DivisionModelInputs: the land model's three DivisionStatus columns,
/// recorded in Map, which leaves DivisionStatus out.
pub const DIVISION_MODEL_INPUTS_FORMAT: u16 = 122;
/// Recordings grow in place: the header names the last complete save, and each
/// save's trailer adds to the one before it. Earlier recordings have neither.
pub const CHECKPOINT_FORMAT: u16 = 123;
/// Grouped replay blocks store later sections in dense 168-hour chunks.
pub const CHUNKED_FORMAT: u16 = 124;
/// Owner-approved separate land and naval raid memo keys.
pub const RAID_DISPATCHERS_FORMAT: u16 = 125;
/// CountryMarket retires market-access and purchase auto-acceptance (fields 4
/// and 6) as tombstones: the game 1.19.4 has neither. Older recordings still decode
/// the table and discard those values.
pub const MARKET_AUTOMATION_FORMAT: u16 = 126;
/// Exact combat base stats, accumulators and individual modifier values.
pub const EXACT_COMBAT_FORMAT: u16 = 127;
pub const PLAYER_COMMANDS_FORMAT: u16 = 131;
/// Owner-approved: WeatherProvinces' temperature pass (field 13) ends with the province
/// temperature witness, a sum of per-slot terms ([`crate::temperature::witness`]), where earlier
/// formats hold an order-dependent checksum in the same eight bytes. Those decode as no witness
/// ([`super::TemperatureWitness`]): their derived temperatures are refused by name.
pub const PROVINCE_WITNESS_FORMAT: u16 = 132;
pub const CONSTRUCTION_OUTPUT_OWNER_FORMAT: u16 = 133;
pub const EXACT_NAVAL_CONDITION_FORMAT: u16 = 134;
pub const AIR_COMBAT_MENU_FORMAT: u16 = 135;
pub const AIR_HISTORY_SLOTS_FORMAT: u16 = 136;
pub const MAP_MODE_PARITY_FORMAT: u16 = 137;
/// Owner-approved: every columnar restart holds the last complete read of Divisions,
/// DivisionEquipmentDeletions and SunkShips with its read hour (H4COLUMN version 4, which
/// refuses earlier columnar recordings by name). Row tables and frames are unchanged.
pub const LAST_COMPLETE_RESTARTS_FORMAT: u16 = 138;
/// Owner-approved: a columnar recording stores each table's hours once per block, written by the
/// recorder from the collector's journal (H4COLUMN version 5, which refuses earlier columnar
/// recordings by name). Row tables and frames are unchanged.
pub const BLOCK_RECORDINGS_FORMAT: u16 = 139;
/// Owner-approved: a fact another table of the same hour holds is stored once, a copy kept only
/// where that table does not give it. DivisionStatus' maxima and DivisionMovement's base speed
/// are DivisionPanelStats' words, the status pass's maxima kept on the panel row where a word
/// does not give them; the shown supply ratio is not stored beside its exact value; a land combat
/// member's division values are its division's rows'; a member's modifier row takes its division
/// from its LandCombatDivisions row. The relaid tables' tombstones go, and older recordings
/// refuse them by name.
pub const STORED_ONCE_FORMAT: u16 = 140;
/// Owner-approved: a columnar page's units each decode alone, in chunks of 2 MB raw compressed
/// apart (H4COLUMN version 8, which refuses earlier columnar recordings by name). Row tables and
/// frames are unchanged.
pub const STANDALONE_UNITS_FORMAT: u16 = 141;
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

/// A nested value whose fields were appended over time ([`crate::value`]'s `appended` record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NestedRecord {
    NavalCommanderInputs,
    LandBorderWarInputs,
    AirCombatRecord,
}

#[derive(Clone, Copy, Debug)]
pub struct AppendedField {
    pub index: usize,
    pub name: &'static str,
    pub introduced: u16,
}

impl NestedRecord {
    /// Its appended fields, each with the format it arrived in.
    pub const fn appended(self) -> &'static [AppendedField] {
        match self {
            Self::NavalCommanderInputs => &[AppendedField {
                index: 17,
                name: "NavalCommanderInputs.timed_traits",
                introduced: RECORDER_INPUTS_FORMAT,
            }],
            Self::LandBorderWarInputs => &[AppendedField {
                index: 2,
                name: "LandBorderWarInputs.combat_width",
                introduced: RECORDER_INPUTS_FORMAT,
            }],
            // Format 135 appended AirCombatRecord.equipment within AirRegionCombat's records.
            Self::AirCombatRecord => &[AppendedField {
                index: 10,
                name: "AirCombatRecord.equipment",
                introduced: AIR_COMBAT_MENU_FORMAT,
            }],
        }
    }
}
