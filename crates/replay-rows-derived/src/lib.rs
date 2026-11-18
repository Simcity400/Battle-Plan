//! The rows the recorder derives from the recorded tables (table ids 422 on) and their codecs,
//! compiled apart from the recorded rows ([`replay_rows`]).
use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::{row_codec, typed_codec};
use std::io;

/// Derived by the recorder from Countries and CountryLinks: the tags some country holds and
/// their holders, the first country by raw id whose link resolves to its canonical slot. The key
/// holds the whole fact (tag number, a run-wide epoch, holder slot and raw id, [`Self::NONE`]
/// while no link resolves), so a row lives exactly one holder's stretch and the table's
/// lifetimes are the run's holder history. Rows numbered [`Self::UNKNOWN`] live while Countries
/// or CountryLinks is not complete, `slot` naming why ([`TagHolderRow::REASONS`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TagHolderRow {
    /// The writer's number for the tag ([`TagNameRow`]).
    pub number: u32,
    /// The run's count of rows before this one.
    pub epoch: u32,
    pub slot: u32,
    /// The holder's raw country id.
    pub id: u32,
    pub tag: Vec<u8>,
}

impl TagHolderRow {
    pub const UNKNOWN: u32 = u32::MAX;
    pub const NONE: u32 = u32::MAX;
    pub const REASONS: [&'static str; 3] = [
        "Countries was not complete at these hours",
        "CountryLinks was not complete at these hours",
        "Countries and CountryLinks were not complete at these hours",
    ];
}

/// Derived by the recorder from the group tables: the run's country groups. Group 0 is the
/// human players; each faction the run met has a number ([`GroupRow`]). The key holds the
/// whole fact (kind, group, value, a run-wide epoch), so a row lives exactly one stretch of it and
/// the table's lifetimes are the run's group history: memberships (value: the member's
/// canonical slot, `ordinal` its order), leaders, a faction's name (value: a [`GroupNameRow`])
/// and presence, war pairs (group and value: the two slots, lower first), a roster that names a
/// country no link resolves, and the hours each history was not observed (group 0 humans, 1
/// factions, 2 wars).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupSpanRow {
    pub kind: u8,
    pub group: u32,
    pub value: u32,
    pub epoch: u32,
    pub ordinal: u32,
}

impl GroupSpanRow {
    pub const MEMBER: u8 = 0;
    pub const LEADER: u8 = 1;
    pub const NAME: u8 = 2;
    pub const PRESENT: u8 = 3;
    pub const INCOMPLETE: u8 = 4;
    pub const WAR: u8 = 5;
    pub const UNOBSERVED: u8 = 6;
    pub const HUMANS: u32 = 0;
    pub const FACTIONS: u32 = 1;
    pub const WARS: u32 = 2;
}

/// Derived beside [`GroupSpanRow`]: each faction's group number, never deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupRow {
    pub number: u32,
    pub faction: u64,
}

/// Derived beside [`GroupSpanRow`]: each faction name's operands, numbered, never deleted: the
/// literal, the template token and that token's spelling (empty when none).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupNameRow {
    pub number: u32,
    pub literal: Vec<u8>,
    pub template_token: Option<u32>,
    pub spelling: Vec<u8>,
}

/// Derived by the recorder: war starts and capitulations as the engine dates them, each a row
/// living the one hour it was first read, the whole fact in its key. A war start's `hour` is the
/// relation's start, `first` the instigating side's canonical slot and `second` the other; a
/// capitulation's `first` is the capitulated country's. [`TagHolderRow::NONE`] where no link
/// resolved the country.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarEventRow {
    pub kind: u8,
    pub hour: i32,
    pub first: u32,
    pub second: u32,
    pub read: u8,
}

impl WarEventRow {
    pub const START: u8 = 0;
    pub const CAPITULATION: u8 = 1;
}

/// Derived by the recorder: what each tag shows (its holder, colour, naming operands and flag
/// inputs), the whole fact in the key, so a row lives one stretch of one look and the table's
/// lifetimes are the run's history: the tag's number ([`TagNameRow`]) and its look's
/// ([`LookFactRow`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LookSpanRow {
    pub number: u32,
    pub fact: u32,
    pub epoch: u32,
    pub read: u8,
}

/// Derived beside [`LookSpanRow`]: each distinct look, numbered and encoded, never deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LookFactRow {
    pub number: u32,
    pub look: Vec<u8>,
}

/// Derived by the recorder: each hour a country's balance fell from one recorded hour to the
/// next, a row living that hour (a log): `kind` 0 political power, 1 command power, keyed with the
/// raw country id and the hour, so drops in consecutive hours are rows apart; the balance before
/// and after, fixed5.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BalanceDropRow {
    pub kind: u8,
    pub country: u32,
    pub hour: i32,
    pub before: i64,
    pub after: i64,
}

/// Derived beside [`TagHolderRow`]: each tag number the run gave, never deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TagNameRow {
    pub number: u32,
    pub tag: Vec<u8>,
}

/// Derived by the recorder from the divisions, their land inputs and templates: each country's
/// land army by canonical slot, each total counted for the division's owner (its positive
/// expeditionary owner, else its roster owner) and for its host (its roster owner). A cell is not
/// recorded while some division of that attribution has an owner no link resolves, or a value
/// the total cannot count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyTotalRow {
    pub slot: u32,
    pub owner_divisions: Recorded<i64>,
    /// Current hit points, fixed5.
    pub owner_strength_raw: Recorded<i64>,
    pub host_divisions: Recorded<i64>,
    pub host_strength_raw: Recorded<i64>,
    pub owner_battalions: Recorded<i64>,
    pub host_battalions: Recorded<i64>,
    pub owner_manpower: Recorded<i64>,
    pub host_manpower: Recorded<i64>,
}

/// Derived by the recorder from States and Provinces: each canonical slot's territory by
/// three rules, owned (the owner), controlled (the controller) and occupied (controlled, owned
/// by another country), each in states, provinces, victory points and land area. Every live country's slot has a
/// row (zeros are recorded zeros), and so does a slot holding territory; a ceased country's slot loses its row.
/// One more row, keyed [`TerritoryTotalRow::WORLD`], holds the world: owned and controlled count every state and
/// province, occupied those a known country controls that another known country owns. A rule's cells are not
/// recorded while some place's country under that rule no link resolves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TerritoryTotalRow {
    pub slot: u32,
    pub owned_states: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub occupied_states: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub controlled_provinces: Recorded<i64>,
    pub occupied_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub occupied_victory_points: Recorded<i64>,
    /// Land area in map cells, from the recorded map's province areas; not recorded while those
    /// are not held or one of the rule's provinces has none.
    pub owned_area: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub occupied_area: Recorded<i64>,
}

impl TerritoryTotalRow {
    /// The world row's key: no canonical slot. A reader of the table by slot skips it.
    pub const WORLD: u32 = u32::MAX;
}

/// Derived by the recorder from each country's CountryStabilityInputs row as last read and its
/// CountryCommandPower row this hour, by the game's arithmetic (`country_stability`): displayed
/// stability, war support and maximum command power less allocated, each fixed5, keyed by raw
/// country id. A value whose operands are not recorded is not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryStabilityRow {
    pub country: u32,
    pub stability: Recorded<i64>,
    pub war_support: Recorded<i64>,
    pub max_command_power: Recorded<i64>,
}

/// Derived beside [`ArmyTotalRow`]: each country's battalions by sub-unit token and attribution
/// (0 owner, 1 host), from its divisions' templates. Not recorded while the attribution's
/// composition is unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyMixRow {
    pub slot: u32,
    pub attribution: u8,
    pub token: u32,
    pub battalions: i64,
}

/// Derived by the recorder from the air wings: each country's planes (each wing's recorded
/// whole-plane word) and wings by canonical slot, counted for the wing's roster owner and for its
/// effective country (its positive country override, else its owner). A cell is not recorded
/// while some wing of that attribution has an owner no link resolves, or, for planes, while a wing
/// counted there has a negative plane word.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceTotalRow {
    pub slot: u32,
    pub roster_planes: Recorded<i64>,
    pub roster_wings: Recorded<i64>,
    pub effective_planes: Recorded<i64>,
    pub effective_wings: Recorded<i64>,
}

/// Derived beside [`AirForceTotalRow`] from the wings' equipment: each country's planes by the
/// variant they are counted under and attribution (0 roster, 1 effective); `variant` none holds
/// the planes no equipment entry identifies. Not recorded where a wing's equipment cannot split
/// its planes exactly. While an attribution is unknown it has no rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForcePartRow {
    pub slot: u32,
    pub ownership: u8,
    pub variant: Option<u64>,
    pub planes: Recorded<i64>,
}

/// Derived beside [`AirForcePartRow`]: each country's wings by attribution and composition, the
/// composition numbered in [`AirForceWingSignatureRow`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceWingKindRow {
    pub slot: u32,
    pub ownership: u8,
    pub signature: u32,
    pub wings: i64,
}

/// Derived beside [`AirForceWingKindRow`]: each wing composition a live wing holds, numbered when
/// first shown, deleted when no live wing holds it (a number is never given twice in a run): the
/// variants a wing's equipment holds positive amounts of, ascending, each as eight
/// little-endian bytes ([`Self::encode`]); empty for a wing whose equipment identifies none.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceWingSignatureRow {
    pub number: u32,
    pub variants: Vec<u8>,
}

impl AirForceWingSignatureRow {
    pub fn encode(variants: &[u64]) -> Vec<u8> {
        variants.iter().flat_map(|variant| variant.to_le_bytes()).collect()
    }

    pub fn variants(&self) -> io::Result<Vec<u64>> {
        if self.variants.len() % 8 != 0 {
            return Err(invalid("a wing signature is not whole variants"));
        }
        Ok(self.variants.chunks_exact(8).map(|bytes| u64::from_le_bytes(bytes.try_into().expect("eight bytes"))).collect())
    }
}

/// Derived by the recorder from the ships and their task forces: each country's ships, current
/// hull HP and maximum hull HP (fixed5) by canonical slot, counted for the task force's roster
/// owner and for its logical owner. A cell is not recorded while some ship of that attribution
/// has an owner no link resolves (or no task force); hull HP while a ship counted there has a
/// negative one; maximum hull HP while one has no model counter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavyTotalRow {
    pub slot: u32,
    pub roster_ships: Recorded<i64>,
    pub roster_strength_raw: Recorded<i64>,
    pub roster_max_strength_raw: Recorded<i64>,
    pub logical_ships: Recorded<i64>,
    pub logical_strength_raw: Recorded<i64>,
    pub logical_max_strength_raw: Recorded<i64>,
}

/// Derived beside [`NavyTotalRow`]: each country's ships and hull HP by attribution (0 roster,
/// 1 logical) and class token. While an attribution is unknown it has no rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavyClassRow {
    pub slot: u32,
    pub ownership: u8,
    pub class: u32,
    pub ships: i64,
    pub strength_raw: Recorded<i64>,
}

/// Derived by the recorder from the divisions, ships and air wings: each live country's deployed
/// manpower by canonical slot, as the game attributes it. The army: the country's own contributor
/// entry in each ordinary-template division it owns, rosters and commands, and in each live
/// division it lists as sent expeditionaries or volunteers; the navy: the crews its fleets' ships
/// hold and need; the air force: its ordinary wings' crews, and the crews of wings flying for it
/// in exile. Each sum wraps as the game's dwords do. A cell is not recorded while a term of it is
/// not known.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryDeployedManpowerRow {
    pub slot: u32,
    pub army: Recorded<i32>,
    pub navy: Recorded<i32>,
    pub navy_required: Recorded<i32>,
    pub air: Recorded<i32>,
    pub exile_air: Recorded<i32>,
}

/// Derived by the recorder from each country's sent volunteers and expeditionaries resolved
/// against the live divisions: by sender and host canonical slot and channel, the listed live
/// divisions the host holds; a row lives while the pair has any. [`Self::TALLIES`] holds the
/// game's own tally of each sender's volunteer hosts (VolunteerHosts), its first count per host,
/// a row for every host listed. A sender whose channel cannot be read whole holds no pair rows of
/// it but one with host [`TagHolderRow::NONE`] and its count not recorded; a list whose sender no
/// link resolves makes that row of sender and host [`TagHolderRow::NONE`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MilitaryAidRow {
    pub sender: u32,
    pub host: u32,
    pub channel: u8,
    pub divisions: Recorded<i64>,
}

impl MilitaryAidRow {
    pub const VOLUNTEERS: u8 = 0;
    pub const EXPEDITIONARIES: u8 = 1;
    pub const TALLIES: u8 = 2;
}

/// Derived by the recorder's SupplyHealth writer from the divisions, their land inputs and status
/// and the content's supply defines: each live country's divisions by canonical slot, counted by
/// the supply icon their counters draw (very low: no supply; low, or low supply alone: strained;
/// none: in supply), each for its owner (its positive expeditionary owner, else its roster owner).
/// The three are not recorded together while some division's owner no link resolves (every row)
/// or one of the country's divisions has a supply operand not known.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SupplyHealthRow {
    pub slot: u32,
    pub no_supply: Recorded<i64>,
    pub strained: Recorded<i64>,
    pub in_supply: Recorded<i64>,
}

/// Derived by the recorder's EquipmentLosses writer: one country identity's (canonical slot, raw
/// country id of its Countries row) equipment lost from one source of one design, fixed5. Drains,
/// convoys and lend-lease add from the run's first read of each; a Combat Log leaf is the sum of
/// the amounts the log holds, the amounts of its rows the game dropped ([`CombatLogDroppedRow`])
/// and `freed`, the amounts of dropped rows whose lines the game has since freed (a Combat Log
/// leaf's alone; not recorded for another source, or once one freed amount was not known).
/// Sunk ships are [`ShipLossRow`]s. `total` is not recorded once it overflows, and a drain leaf's
/// once the drain is withheld for the run; `incomplete_from` is the first hour from which an
/// exchange that ended after its last read makes a lend-lease leaf a lower bound.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentLossTotalRow {
    pub slot: u32,
    pub country: u32,
    /// [`Self::COMBAT_LOG`], [`Self::SUNK_CONVOYS`], [`Self::LEND_LEASE`] or [`Self::DESTRUCTION`],
    /// plus the record's own kind or cause.
    pub source: u8,
    /// The design's variant reference id; none where the record names no design.
    pub variant: Option<u32>,
    pub total: Recorded<i64>,
    pub incomplete_from: Option<i32>,
    pub freed: Recorded<i64>,
}

impl EquipmentLossTotalRow {
    /// Plus the Combat Log's stored kind, below [`Self::SUNK_CONVOYS`].
    pub const COMBAT_LOG: u8 = 0;
    pub const SUNK_CONVOYS: u8 = 32;
    /// Plus the exchange pool's kind, below [`Self::DESTRUCTION`].
    pub const LEND_LEASE: u8 = 48;
    /// Plus the [`EquipmentDestructionCause`].
    pub const DESTRUCTION: u8 = 64;

    /// The variant reference id of an equipment design handle; none for another reference type.
    pub fn variant_id(handle: u64) -> Option<u32> {
        (handle as u32 == engine_reference_values::native_values::equipment_variant::REFERENCE_TYPE).then_some((handle >> 32) as u32)
    }
}

/// Derived by the recorder's EquipmentLosses writer: each CombatLogEquipment row the game
/// deleted, under its own key, holding its last cells (its month entry's first write and its
/// amount); deleted again when its key returns, or once the game has freed its line (its month
/// past `NGame.COMBAT_LOG_MAX_MONTHS` and a month more), its amount then kept in its leaf's
/// [`EquipmentLossTotalRow::freed`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogDroppedRow {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub month: u16,
    pub variant: u32,
    pub date: Recorded<i32>,
    pub amount: Recorded<i64>,
}

/// Derived by the recorder's EquipmentLosses writer, a log whose rows live the hour they were
/// read: at a complete SunkShips read, each sunk ship signature of a country identity that read
/// lists more often than the read before it (every listed one at a run's first hour and after an
/// hour not compared, and every one of a slot whose raw ids or identities the hour moved). `signature` is a 48-bit hash of the entry's name,
/// sinking hour, design handle, battle, damage source and cause record, `check` a second hash,
/// `count` its entries in the read. The most `count` of a (signature, check) over the rows up to an
/// hour is the most entries of it one read listed; it is dated at `sink_hour`. Each slot and
/// country is at most [`Self::FIELD`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipLossRow {
    pub slot: u32,
    pub country: u32,
    pub hour: i32,
    pub signature: u64,
    pub sink_hour: i32,
    /// The full design handle as the entry names it.
    pub variant: u64,
    pub count: u32,
    pub check: u64,
}

impl ShipLossRow {
    /// The greatest slot or country a row's key holds.
    pub const FIELD: u32 = (1 << 24) - 1;
    /// The bits of a signature.
    pub const SIGNATURE: u64 = (1 << 48) - 1;
}

/// Derived by the recorder's EquipmentLosses writer. A country identity's row (while its
/// Countries row lives) holds its Combat Log land coverage ([`Self::LOGGED`] ...), the divisions
/// outside orders groups while partial, whether its link resolves to its slot, the convoy counts
/// that fell and were counted anew, and the lend-lease exchanges that ended after their last read
/// with the hour the last of them is lost from (each rise of the count by n at an hour: n
/// exchanges lost from `ended_from`). The run's row ([`Self::RUN`]) holds the drain's reason this
/// hour (empty for a complete drain missing nothing; the fixed reasons are `DRAIN_*`, the others
/// the recording's own), the records not read completely this hour (bit 0 Combat Log, 1 ships, 2
/// convoys, 3 lend-lease), the first complete drain, the hour the drain was withheld for the run,
/// the first hour a counted drain was lost or partly unknown, the first reads convoys and
/// lend-lease count from, and the distinct raw countries the run's loss rows named that no link
/// resolved (drain, ships, convoys and lend-lease; and with the Combat Log's). A cell another kind
/// of row holds is not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentLossCoverageRow {
    pub slot: u32,
    pub country: u32,
    pub land: Recorded<u8>,
    pub ungrouped: Recorded<u32>,
    pub resolved: Recorded<u8>,
    pub convoy_drops: Recorded<u32>,
    pub drain: Recorded<Vec<u8>>,
    pub unread: Recorded<u8>,
    pub counted_from: Recorded<Option<i32>>,
    pub withheld_from: Recorded<Option<i32>>,
    pub incomplete_from: Recorded<Option<i32>>,
    pub convoys_from: Recorded<Option<i32>>,
    pub lend_from: Recorded<Option<i32>>,
    pub exchanges_ended: Recorded<u32>,
    pub ended_from: Recorded<Option<i32>>,
    pub unresolved: Recorded<u32>,
    pub unresolved_all: Recorded<u32>,
}

impl EquipmentLossCoverageRow {
    /// The run's row: slot and country both this.
    pub const RUN: u32 = u32::MAX;
    pub const LOGGED: u8 = 0;
    pub const NOT_LOGGED: u8 = 1;
    pub const PARTIAL: u8 = 2;
    pub const UNKNOWN: u8 = 3;
    pub const PRUNE_UNOBSERVED: u8 = 4;
    pub const UNREAD_COMBAT_LOG: u8 = 1;
    pub const UNREAD_SHIPS: u8 = 2;
    pub const UNREAD_CONVOYS: u8 = 4;
    pub const UNREAD_LEND_LEASE: u8 = 8;
    pub const DRAIN_TRUNCATED: &'static str = "Equipment destruction drain is truncated; withheld for the run";
    pub const DRAIN_SUPPLEMENTAL_NOT_RECORDED: &'static str = "EquipmentDestruction supplemental coverage not recorded";
    /// Joined to a supplemental reason by "; " when both hold.
    pub const DRAIN_OWNER_UNKNOWN: &'static str = "Equipment destruction ownership is unknown: raw country references have no CountryLink";
    pub const DRAIN_NOT_RECORDED: &'static str = "EquipmentDestruction is not recorded in this recording";
    pub const DRAIN_NOT_READ: &'static str = "Equipment destruction drain was not read at this hour";
    pub const DRAIN_LOST: &'static str = "Equipment destruction drain is missing at hours the writer did not run";
}

/// Derived by the recorder's Stockpile writer from Stockpiles: each canonical slot's stock of one
/// design (its full variant handle), fixed5, summed over the slot's raw countries' list positions,
/// and how many positions hold it (a design held at zero is listed). A row lives while some
/// position holds the design. Both are not recorded while some Stockpiles row's country no link
/// resolves; `amount` once the sum leaves i64.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockByVariantRow {
    pub slot: u32,
    pub variant: u64,
    pub amount: Recorded<i64>,
    pub positions: Recorded<i64>,
}

/// Derived by the recorder's DivisionEquipment writer: each canonical slot's divisions by
/// attribution (`side` [`Self::OWNER`], their positive expeditionary owner else roster owner, or
/// [`Self::HOST`], their roster owner), those whose pool and template need are compared and fall
/// short of any type, and those whose comparison is not known, of them those whose pool is not
/// read and those holding a design no recorded definition classifies; the divisions of that
/// attribution, in any slot, whose owner no link resolves; the hour of the daily pool read and of
/// the stockpile read the rows of the slot stand on, none while no such read is held. The counts
/// but `unresolved` are not recorded while `unresolved` is not zero.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentCountRow {
    pub slot: u32,
    pub side: u8,
    pub divisions: Recorded<i64>,
    pub short: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub unpooled: Recorded<i64>,
    pub unclassified: Recorded<i64>,
    pub unresolved: i64,
    pub pool_hour: Option<i32>,
    pub stock_hour: Option<i32>,
}

impl DivisionEquipmentCountRow {
    pub const OWNER: u8 = 0;
    pub const HOST: u8 = 1;
}

/// Derived beside [`DivisionEquipmentCountRow`]: one archetype (its lexer token; none for designs
/// no recorded definition classifies) a slot's divisions hold or need, fixed5. `held` sums every
/// read pool; `needed`, `filled` (each division's holding capped at its own need) and `missing`
/// sum the divisions whose comparison is known, not recorded while none compares the type;
/// `short` counts known divisions short of it; `stock` is the slot's national stock of it at the
/// stockpile read, not recorded while that stock is not known whole. Every cell is not recorded
/// while some division of the attribution has an owner no link resolves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentTypeRow {
    pub slot: u32,
    pub side: u8,
    pub archetype: Option<u32>,
    pub held: Recorded<i64>,
    pub needed: Recorded<i64>,
    pub filled: Recorded<i64>,
    pub missing: Recorded<i64>,
    pub short: Recorded<i64>,
    pub stock: Recorded<i64>,
}

/// Derived beside [`DivisionEquipmentTypeRow`]: what a slot's read pools hold of one design, by
/// its archetype and variant reference id (none for a pool entry that names no design), fixed5.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentDesignRow {
    pub slot: u32,
    pub side: u8,
    pub archetype: Option<u32>,
    pub variant: Option<u32>,
    pub held: Recorded<i64>,
}

/// Derived beside [`DivisionEquipmentCountRow`]: a slot's divisions of one template. Manpower,
/// required manpower and maximum hit points are not recorded while one division lacks them, and
/// `manpower_lacking` and `hp_max_lacking` count the divisions that do;
/// `needed` and `held` (needed archetypes only, each division capped at its own need) sum the
/// divisions whose comparison is known, not recorded while none is; `types_short` counts the
/// archetypes some known division falls short of. `need_missing` says why the template's need
/// is not known; empty while it is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentTemplateRow {
    pub slot: u32,
    pub side: u8,
    pub template: u64,
    pub divisions: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub types_short: Recorded<i64>,
    pub manpower: Recorded<i64>,
    pub manpower_lacking: Recorded<i64>,
    pub required: Recorded<i64>,
    /// Current hit points, fixed5.
    pub hp: Recorded<i64>,
    pub hp_max: Recorded<i64>,
    pub hp_max_lacking: Recorded<i64>,
    pub needed: Recorded<i64>,
    pub held: Recorded<i64>,
    pub need_missing: Vec<u8>,
}

/// Derived by the recorder's Production writer from ProductionLineOutput: one producer's output of
/// one design for one purpose since the observation began, kept after the lines' rows are gone.
/// The producer (`country`) is the raw id of the lowest Countries row whose link resolves to its
/// own canonical slot, for the slot an output row's country links to. `units` sums each row's
/// latest count (a lower bound while `inexact_rows`, the rows not counted exactly or lost without
/// their final count, is above zero); `spent` sums each row's applied factory IC (fixed5) and is
/// not recorded once one row's is unknown (`spent_reason`); `unproven` sums the magnitudes of the
/// applied IC whose purpose was not recorded, not recorded with `spent`; `equipment_token` is the
/// design's equipment token as last spelled, kept after its definition is gone; `archetype` is the
/// lexer token of the design's recorded archetype the total is counted under in
/// [`ProductionTypeTotalRow`] ([`ProductionTypeTotalRow::NO_ARCHETYPE`] while none was recorded).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionTotalRow {
    pub country: u32,
    pub variant: u64,
    /// [`OutputPurpose::raw`].
    pub purpose: u8,
    pub units: i64,
    pub inexact_rows: u32,
    pub spent: Recorded<i64>,
    pub spent_reason: u8,
    pub unproven: Recorded<i64>,
    pub equipment_token: Recorded<Vec<u8>>,
    pub archetype: u32,
}

impl ProductionTotalRow {
    /// Why applied IC is not known, here and in [`ProductionCoverageRow`].
    pub const KNOWN: u8 = 0;
    /// The collector did not record a row's applied IC.
    pub const NOT_RECORDED: u8 = 1;
    /// The collector refused a row's applied IC, or a row's producer could not be told.
    pub const SPENDING_UNAVAILABLE: u8 = 2;
    /// A row left the table without its final count.
    pub const LOST_OUTPUT: u8 = 3;
    /// Output the writer did not read: an hour its inputs were not complete, or a row whose
    /// producer could not be told after it delivered.
    pub const MISSED_PUBLICATION: u8 = 4;
    /// A Rocket row's counters went back, or one changed without a tick or a delivery.
    pub const ROCKET_INPUT_INEXACT: u8 = 5;
    /// A Rocket tick or delivery of a producer that could not be told.
    pub const ROCKET_OWNER_UNREADABLE: u8 = 6;
}

/// Derived beside [`ProductionTotalRow`]: each producer's sums of its rows, with refits and
/// without them, so a ranking and a total read one row. Every live producer has a row, zero while
/// it produced nothing. `*_inexact` counts the producer's designs and purposes whose units are a
/// lower bound; `spent*` is not recorded once one of its parts' applied IC is unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionCountryTotalRow {
    pub country: u32,
    pub units: i64,
    pub units_inexact: u32,
    pub spent: Recorded<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: u32,
    pub spent_without_refits: Recorded<i64>,
}

/// Derived beside [`ProductionTotalRow`]: each producer's sums by equipment type, the recorded
/// archetype of its designs (VariantDefinitions' archetype lexer token; [`Self::NO_ARCHETYPE`] for
/// designs whose definition was never read, which could be of any type), with refits and without,
/// so a ranking or a total over chosen types reads one row a producer and type. The
/// [`Self::WORLD`] producer's rows sum every producer's. `archetype_spelling` is the archetype as
/// last spelled. Columns as in [`ProductionCountryTotalRow`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionTypeTotalRow {
    pub country: u32,
    pub archetype: u32,
    pub units: i64,
    pub units_inexact: u32,
    pub spent: Recorded<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: u32,
    pub spent_without_refits: Recorded<i64>,
    pub archetype_spelling: Recorded<Vec<u8>>,
}

impl ProductionTypeTotalRow {
    pub const WORLD: u32 = u32::MAX;
    pub const NO_ARCHETYPE: u32 = 0;
}

/// Derived by the recorder's Production writer: what production's sources leave unknown from an
/// hour on. The run's row of a source ([`Self::RUN`]) holds where its observation began, and the
/// first hours from which every producer's units and applied IC are unknown (with why); a
/// producer's own row holds the first hours its own Rocket output became unknown. Rows of a
/// source are cleared when its observation restarts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionCoverageRow {
    pub country: u32,
    pub source: u8,
    pub observation_since: Option<i32>,
    pub units_unknown_from: Option<i32>,
    pub spending_unknown_from: Option<i32>,
    /// A [`ProductionTotalRow`] reason.
    pub spending_reason: u8,
}

impl ProductionCoverageRow {
    pub const RUN: u32 = u32::MAX;
    /// Production lines' output ([`ProductionTotalRow`]).
    pub const LINES: u8 = 0;
    /// RocketProductionOutput, read as it is recorded.
    pub const ROCKET: u8 = 1;
}

/// Derived by the recorder's DockyardRepairs writer: a country's dockyard repair output rate,
/// dockyards in use times the daily output one of its dockyards applies (fixed5 IC a day), whole
/// ([`Self::TOTAL`]), and split by the designs its naval bases' current repair selections hold
/// ([`Self::DESIGN`], one row per design selected) and the rest ([`Self::UNATTRIBUTED`]). The
/// country is the raw id of the lowest Countries row whose link resolves to its own canonical slot.
/// The total row also holds the dockyards in use, one dockyard's output and whether the bases'
/// allocations reconcile with the dockyards in use; while they do not, the split is not known:
/// no design rows, the unattributed rate not recorded. A cell another part does not hold is not
/// recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DockyardRepairRateRow {
    pub country: u32,
    pub part: u8,
    /// The design's variant handle; zero for the other parts.
    pub design: u64,
    pub rate: Recorded<i64>,
    pub used_dockyards: Recorded<i32>,
    pub output: Recorded<i64>,
    pub split_known: Recorded<bool>,
}

impl DockyardRepairRateRow {
    pub const TOTAL: u8 = 0;
    pub const UNATTRIBUTED: u8 = 1;
    pub const DESIGN: u8 = 2;
}

/// Derived by the recorder's LandBattles writer: each live land battle by its combat handle. A
/// battle is one handle read at every hour at one province with a duration counter that never
/// falls. `start_exact` when the read before its first was the hour before; `counter_base` is
/// then zero, else the counter at its first read. Each peak is the most distinct units at once
/// (both sides, attackers, defenders) and the earliest hour of it, not recorded while no roster
/// was read whole.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBattleRow {
    pub combat: u64,
    pub start_hour: i32,
    pub start_exact: bool,
    pub counter_base: i32,
    pub peak_total: Recorded<u32>,
    pub peak_total_hour: Recorded<i32>,
    pub peak_attackers: Recorded<u32>,
    pub peak_attackers_hour: Recorded<i32>,
    pub peak_defenders: Recorded<u32>,
    pub peak_defenders_hour: Recorded<i32>,
}

/// Derived by the recorder's LandBattles writer: a row living the one hour a land battle's end
/// was settled, keyed by its combat handle and start hour ([`Self::RESULT_ONLY`] rows by their
/// finalizer's end hour, with `result_only` set). `kind` says how it ended and `end_hour` when:
/// the read that found its row gone or its handle reused, the finalizer's end hour, or for
/// [`Self::UNKNOWN`] the first hour its census was not read. The battle's own columns are as its
/// last read left them; `attackers` and `defenders` are the canonical slots its side countries
/// resolved to then, a bit of `countries_unknown` set ([`Self::ATTACKERS_UNKNOWN`],
/// [`Self::DEFENDERS_UNKNOWN`]) where a side had no country or one no link resolved.
/// `result_read` when the land event drain that would hold its finalizer was read whole: without
/// a `result` the game then finalized none. The result's columns are the finalizer's, its
/// countries resolved at the hour it was read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBattleEndRow {
    pub combat: u64,
    pub start_hour: i32,
    pub result_only: bool,
    pub kind: u8,
    pub end_hour: i32,
    pub start_exact: bool,
    pub counter_base: i32,
    pub province: u32,
    pub last_read: Option<i32>,
    pub last_duration: Recorded<i32>,
    pub peak_total: Recorded<u32>,
    pub peak_total_hour: Recorded<i32>,
    pub peak_attackers: Recorded<u32>,
    pub peak_attackers_hour: Recorded<i32>,
    pub peak_defenders: Recorded<u32>,
    pub peak_defenders_hour: Recorded<i32>,
    /// Each side's manpower loss counter at its last hourly read.
    pub attacker_manpower: Recorded<i32>,
    pub defender_manpower: Recorded<i32>,
    pub attackers: Recorded<Vec<u32>>,
    pub defenders: Recorded<Vec<u32>>,
    pub countries_unknown: u8,
    pub result: Option<u64>,
    pub result_read: bool,
    /// The finalizer's winning branches: bit 0 attacker, bit 1 defender.
    pub winners: Recorded<u8>,
    pub result_duration: Recorded<i32>,
    /// Each side's manpower lost by the finalizer's flush.
    pub attacker_terminal: Recorded<i32>,
    pub defender_terminal: Recorded<i32>,
    pub result_attackers: Recorded<Vec<u32>>,
    pub result_defenders: Recorded<Vec<u32>>,
    pub result_unknown: u8,
    /// Each side's equipment pool by design after the finalizer's flush.
    pub attacker_terminal_equipment: Recorded<Vec<LandEquipmentLoss>>,
    pub defender_terminal_equipment: Recorded<Vec<LandEquipmentLoss>>,
}

impl LandBattleEndRow {
    /// Its combat row was gone at the read after its last.
    pub const ABSENT: u8 = 0;
    /// Its handle was read at another province or with a counter that fell: a battle of its own.
    pub const REUSED: u8 = 1;
    /// A finalizer ended it while its combat row was still read.
    pub const FINALIZED: u8 = 2;
    /// Its census was not read at an hour after its last read: when it ended is not known.
    pub const UNKNOWN: u8 = 3;
    /// A finalizer that ended no battle read live: its result alone.
    pub const RESULT_ONLY: u8 = 4;
    pub const ATTACKERS_UNKNOWN: u8 = 1;
    pub const DEFENDERS_UNKNOWN: u8 = 2;
}

/// Derived by the recorder's NavalBattles writer: each live naval battle by its combat handle,
/// one handle read at every hour at one province. `peak_ships` is the most ships both sides held
/// at once at a read whose participants were read whole, `peak_hour` the earliest hour of it;
/// each side's sunk and convoys-sunk floors are the most its participants listed sunk at one
/// read (groups can be detached, so samples are never summed). Not recorded until a read with
/// participants.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalBattleRow {
    pub combat: u64,
    pub first_hour: i32,
    pub peak_ships: Recorded<u32>,
    pub peak_hour: Recorded<i32>,
    pub attacker_sunk: Recorded<u32>,
    pub defender_sunk: Recorded<u32>,
    pub attacker_convoys_sunk: Recorded<u32>,
    pub defender_convoys_sunk: Recorded<u32>,
}

/// Derived by the recorder's NavalBattles writer from NavalResultSources: for a retained naval
/// result, the live battle its persistent source named, as the battle stood at the read at or
/// before the source's game hour; a row lives while its NavalResults row does. `refusal` is
/// [`Self::OBSERVED`], or why the source observes no battle, its other cells then not recorded.
/// Quantities of a side are not recorded where its participants were not read whole.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalResultObservationRow {
    pub result: u64,
    pub refusal: u8,
    pub combat: u64,
    pub province: Recorded<u32>,
    pub first_hour: Recorded<i32>,
    pub last_hour: Recorded<i32>,
    pub peak_ships: Recorded<u32>,
    pub peak_hour: Recorded<i32>,
    pub attacker_ships: Recorded<u32>,
    pub attacker_convoys: Recorded<u32>,
    pub attacker_sunk: Recorded<u32>,
    pub attacker_convoys_sunk: Recorded<u32>,
    pub defender_ships: Recorded<u32>,
    pub defender_convoys: Recorded<u32>,
    pub defender_sunk: Recorded<u32>,
    pub defender_convoys_sunk: Recorded<u32>,
}

impl NavalResultObservationRow {
    pub const OBSERVED: u8 = 0;
    /// The source's identity or hour is out of order, or it is a persistent merge.
    pub const INVALID_SOURCE: u8 = 1;
    /// Its combat was linked before, or its result was observed or retained before.
    pub const DUPLICATE: u8 = 2;
    /// Its combat's battle began after the source, or after the combat's construction.
    pub const MISMATCH: u8 = 3;
    /// Its combat was not read at the hour or the hour before.
    pub const NOT_OBSERVED: u8 = 4;
}

/// Derived by the recorder's NavalLosses writer from SunkShips and SunkConvoys: by country
/// identity (canonical slot, raw id of its Countries row whose link resolves to it), attribution
/// ([`Self::LOST`], [`Self::SUNK_BY`]) and kind ([`Self::SHIPS`], [`Self::CONVOYS`]), the
/// sinkings counted since the kind's baseline read. A ship is an entry whose multiplicity in
/// SunkShips rose between consecutive complete reads, sunk by its attacker unless a cause record
/// or an assist; convoys are each bucket's signed change. Countries resolve at the read; a
/// reference no link resolves counts under slot [`Self::NONE`] by its raw id, a slot no identity
/// holds under country [`Self::NONE`]. `unknown` counts entries refused (an event hour after its
/// read, a flag above 1). The kind's run row (slot and country [`Self::RUN`], attribution
/// [`Self::LOST`]) holds `counted_from`, the baseline read every total counts after (a total
/// absent then is zero), `known` (its table read whole at the hour) and `lost_from`, the first
/// read after hours unread long enough for the game's retention to have pruned sinkings unseen,
/// or without the retention define; a cell another kind of row holds is not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalLossTotalRow {
    pub slot: u32,
    pub country: u32,
    pub attribution: u8,
    pub kind: u8,
    pub total: Recorded<i64>,
    pub unknown: Recorded<u32>,
    pub counted_from: Recorded<i32>,
    pub known: Recorded<bool>,
    pub lost_from: Recorded<Option<i32>>,
}

impl NavalLossTotalRow {
    pub const NONE: u32 = u32::MAX - 1;
    pub const RUN: u32 = u32::MAX;
    pub const LOST: u8 = 0;
    pub const SUNK_BY: u8 = 1;
    pub const SHIPS: u8 = 0;
    pub const CONVOYS: u8 = 1;
}

/// Derived by the recorder's PowerLedger writer: what one holder (the canonical slot a payment's
/// raw country links to, [`TagHolderRow::NONE`] where none does) paid for one item in one currency
/// from the run's second hour (its first lists what the game held when the run began), fixed5. A
/// balance change counts in the hour it is read; a daily cost item's day counts when the day ends
/// (its open day is a [`PowerDayPartRow`]), and the gains of a day when its daily row gives way to
/// the next day's (the open day is the CountryPowerDaily row). A one-off payment adds to `known` (a charge not recorded adds to
/// `unknown` instead), `payments`, `entries` and, read outside the capture's coverage,
/// `uncovered`; `latest_*` is its latest by native hour, sequence and cost ordinal, and the read it
/// came in. A recurring payment (daily costs, daily deficits, ability upkeep) merges into its
/// native day: `days`, its payments, entries (upkeep's day is one entry), charge, payments without
/// a charge or realized delta, days read outside coverage, the realized balance moves split by
/// sign, the first and latest day, and the latest day's lag from its midnight to its read, first
/// hour, first payment and coverage. `balances` counts rows of the item that are no payment,
/// `credits` negative daily costs. The row with ordinal [`Self::GAINS`] of each currency holds
/// that currency's daily gains instead: gained, kept off by the cap, days whose gain was read and
/// was not, and daily costs whose day's application was not recorded; every column the other kind
/// of row holds is not recorded. A sum that leaves i64 is not recorded from then. Carry: sum on
/// every count and amount, none on `latest_*`, `first_day`, `last_*`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerSpendTotalRow {
    pub holder: u32,
    pub currency: u8,
    pub ordinal: u32,
    pub known: Recorded<i64>,
    pub payments: Recorded<i64>,
    pub entries: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub uncovered: Recorded<i64>,
    pub latest_hour: Recorded<Option<i32>>,
    pub latest_sequence: Recorded<Option<u64>>,
    pub latest_ordinal: Recorded<Option<u32>>,
    pub latest_read: Recorded<Option<i32>>,
    pub days: Recorded<i64>,
    pub day_payments: Recorded<i64>,
    pub day_entries: Recorded<i64>,
    pub day_charge: Recorded<i64>,
    pub day_unknown: Recorded<i64>,
    pub days_uncovered: Recorded<i64>,
    pub realized_gained: Recorded<i64>,
    pub realized_lost: Recorded<i64>,
    pub realized_unknown: Recorded<i64>,
    pub first_day: Recorded<Option<i32>>,
    pub last_day: Recorded<Option<i32>>,
    pub last_lag: Recorded<Option<i32>>,
    pub last_hour: Recorded<Option<i32>>,
    pub last_sequence: Recorded<Option<u64>>,
    pub last_ordinal: Recorded<Option<u32>>,
    pub last_covered: Recorded<bool>,
    pub balances: Recorded<i64>,
    pub credits: Recorded<i64>,
    pub gained: Recorded<i64>,
    pub capped: Recorded<i64>,
    pub gain_days: Recorded<i64>,
    pub gain_unknown_days: Recorded<i64>,
    pub unmatched: Recorded<i64>,
}

impl PowerSpendTotalRow {
    /// The ordinal of a currency's daily gains row.
    pub const GAINS: u32 = u32::MAX;
}

/// Derived by the recorder's PowerLedger writer: each item a holder paid, numbered in the order
/// first paid, never deleted in a run. An item is its typed operands alone: what recorded it
/// ([`Self::CHANGE`], [`Self::COST`], or [`Self::ABILITY_UPKEEP`], every ability's upkeep one
/// item), its currency, cause, daily cost component, whether it charged, and the payment's
/// recorded identity; `site` and `first_*` are its first payment's site, own hour, read hour, raw
/// country, capture epoch and change sequence (a cost's ordinal), where a reply names it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerItemRow {
    pub holder: u32,
    pub ordinal: u32,
    pub kind: u8,
    pub currency: PowerCurrency,
    pub cause: PowerCause,
    pub component: Option<u32>,
    pub charged: bool,
    pub site: PowerSite,
    pub action: PowerAction,
    pub item_token: Option<u32>,
    pub item_key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub object_kind: PowerObjectKind,
    pub object_reference: Option<u64>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
    pub origin_kind: PowerOriginKind,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
    pub first_hour: i32,
    pub first_read: i32,
    pub first_country: u32,
    pub first_epoch: u64,
    pub first_sequence: u64,
}

impl PowerItemRow {
    pub const CHANGE: u8 = 0;
    pub const COST: u8 = 1;
    pub const ABILITY_UPKEEP: u8 = 2;
}

/// Derived by the recorder's PowerLedger writer: what a holder spent in the open native day on one
/// item ([`PowerItemRow`]'s ordinal) in one currency, fixed5; a reply files items into categories
/// from the recorded content, and shows a part only while its currency's spending was covered
/// since the day began ([`PowerSpendCoverageRow`]). A row lives from the day's first such payment
/// to the day's end. A balance change adds to it as it is read (`sequence` none); a daily cost
/// item's row is its day's latest publication (`sequence` its political application, `cost_ordinal`
/// its first cost), counted into [`PowerSpendTotalRow`] when the day ends. The row with ordinal
/// [`PowerSpendTotalRow::GAINS`] counts the publication's costs whose application was not
/// recorded. `spent` is not recorded when a charge was not; `covered` while every payment was;
/// `read` is the hour its first payment was read. Carry none.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerDayPartRow {
    pub holder: u32,
    pub currency: u8,
    pub ordinal: u32,
    pub day: i32,
    pub spent: Recorded<i64>,
    pub payments: i64,
    pub covered: bool,
    pub sequence: Option<u64>,
    pub cost_ordinal: Option<u32>,
    pub read: i32,
}

/// Derived by the recorder's PowerLedger writer: the command power a holder's covered allocations
/// hold, by a hash of the allocation's operands (`check` a second hash of them), fixed5, with the
/// first allocation's operands, and the capture identity (raw country, epoch) the holds were applied
/// under. Every hold is dropped when the capture's command power stream
/// breaks, a refund cannot name its allocation, the holder's allocated total is not read or zero,
/// or what the holds name exceeds it. Carry none.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerHoldRow {
    pub holder: u32,
    pub hold: u64,
    pub capture_country: u32,
    pub capture_epoch: u64,
    pub amount: i64,
    pub check: u64,
    pub cause: PowerCause,
    pub object_kind: PowerObjectKind,
    pub object_reference: Option<u64>,
    pub ability: bool,
    pub item_token: Option<u32>,
    pub item_key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
}

/// Derived by the recorder's PowerLedger writer, one row per live country's canonical slot:
/// whether its political and command power spending was recorded whole this hour, why not
/// ([`Self::REASONS`]), the first hour of each currency's unbroken covered stretch, and whether
/// its capture's daily political cost did not add up to its costs ([`Self::STEADY_MISMATCH`]) or
/// a daily gain was read without its day's application ([`Self::GAIN_UNMATCHED`]), whatever the
/// reason names. `input_reason` is the reason an input partial or unavailable this hour gave, by
/// name (empty when none was).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerSpendCoverageRow {
    pub holder: u32,
    pub political: bool,
    pub command: bool,
    pub reason: u8,
    pub political_since: Option<i32>,
    pub command_since: Option<i32>,
    pub daily_mismatch: u8,
    pub input_reason: Vec<u8>,
}

impl PowerSpendCoverageRow {
    pub const STEADY_MISMATCH: u8 = 1;
    pub const GAIN_UNMATCHED: u8 = 2;
    pub const KNOWN: u8 = 0;
    pub const CHANGES_UNREAD: u8 = 1;
    pub const DAILY_COSTS_UNREAD: u8 = 2;
    pub const POLITICAL_OPERAND: u8 = 3;
    pub const COMMAND_OPERAND: u8 = 4;
    pub const BOTH_OPERANDS: u8 = 5;
    pub const NO_CAPTURE: u8 = 6;
    pub const STALE: u8 = 7;
    pub const COST_UNMATCHED: u8 = 8;
    pub const NEGATIVE_COST: u8 = 9;
    pub const STREAMS: u8 = 10;
    pub const BACKLOG: u8 = 11;
    /// Each reason's text, by code.
    pub const REASONS: [&'static str; 12] = [
        "",
        "Spending events were not recorded at this read",
        "Daily political power costs were not recorded at this read",
        "A political power adjustment has no recorded evaluated operand",
        "A command power adjustment has no recorded evaluated operand",
        "Political and command power adjustments have no recorded evaluated operands",
        "country identity or politics coverage unavailable",
        "politics coverage is stale at this read",
        "Daily political power costs have no matched payment at this read",
        "Negative daily political power cost components are not included in spending",
        "the politics capture did not account for every balance change at this read",
        "the run's first hour lists what the game held when the run began: spending is counted from the next hour",
    ];
}

/// Derived by the recorder's Feed writer from EventFeedHistory: one row per occurrence the
/// collector read, keyed by its own identity (generation, sequence), living the hour of its read;
/// registry changes are not occurrences. Also a country's owned states falling to none
/// ([`Self::CEASED_TO_EXIST`]), a guarantee first read ([`Self::GUARANTEE_GRANTED`]) and a read
/// not complete ([`Self::READ_INCOMPLETE`]: its coverage code, outstanding observations, refused
/// script family bits and failure text), keyed [`Self::DERIVED`] and the hour and ordinal of
/// their read. `kind` is the occurrence's
/// EventFeedSource code. Countries are raw (reference, id, instance) with the canonical slot the
/// reference resolved to at the read, [`TagHolderRow::NONE`] for no country, not recorded where no
/// link and holder witnessed it; cause columns are not recorded when the cause was not, `token`
/// when it was not captured, `scopes` when the scope graph was not. `opener` is the sequence of the
/// dispatch, selection or activation the row continues and `opener_hour` that row's read hour, so
/// a card's later rows are found by it; `dated` the hour a derived row dates
/// itself from (a guarantee's start, the last read a country still owned states); the powers the
/// realized political and command power charges proven for a decision's execution or timeout;
/// `backlog` that the occurrence was listed when the run began (a log input's backlog, dated at
/// its own `hour`), not read as an event of `read_hour`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedEventRow {
    pub generation: u64,
    pub sequence: u64,
    pub hour: i32,
    pub read_hour: i32,
    pub kind: u8,
    pub actor_reference: u32,
    pub actor_id: Recorded<u32>,
    pub actor_instance: Recorded<u32>,
    pub actor_slot: Recorded<u32>,
    pub target_reference: u32,
    pub target_id: Recorded<u32>,
    pub target_instance: Recorded<u32>,
    pub target_slot: Recorded<u32>,
    pub route: Recorded<u8>,
    pub within: Recorded<u8>,
    pub conference: Recorded<Option<u64>>,
    pub state: Recorded<Option<u32>>,
    pub ally_reference: Recorded<Option<u32>>,
    pub ally_id: Recorded<Option<u32>>,
    pub ally_instance: Recorded<Option<u32>>,
    pub ally_slot: Recorded<Option<u32>>,
    pub reason: Recorded<Option<u32>>,
    pub flags: Recorded<u8>,
    pub token: Recorded<Vec<u8>>,
    pub native_event_kind: Option<u32>,
    pub native_event_id: Option<u32>,
    pub instance: Option<u32>,
    pub option: Option<i32>,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
    pub lifecycle: Option<i32>,
    pub days: Option<i32>,
    pub scopes: Recorded<Vec<replay_rows::event_feed_rows::EventFeedScope>>,
    pub opener: Option<u64>,
    pub opener_hour: Option<i32>,
    pub dated: Option<i32>,
    pub political_power: Recorded<Option<i64>>,
    pub command_power: Recorded<Option<i64>>,
    pub coverage: Option<u32>,
    pub outstanding: Option<u64>,
    pub refusals: Option<u32>,
    pub failure: Vec<u8>,
    pub backlog: bool,
    /// An incomplete read's facet: its coverage in the low byte, the script families it refused
    /// in the next ([`FeedEventRow::incomplete_of`]); none on every other row.
    pub incomplete: Option<u32>,
}

impl FeedEventRow {
    /// Where the refused script families sit in [`FeedEventRow::incomplete`].
    pub const REFUSALS_SHIFT: u32 = 8;

    /// The facet value of an incomplete read with `coverage` and `refusals`.
    pub const fn incomplete_of(coverage: u8, refusals: u8) -> u32 {
        coverage as u32 | (refusals as u32) << Self::REFUSALS_SHIFT
    }

    /// The generation of derived rows, whose sequence is their read hour's bits, then their ordinal.
    pub const DERIVED: u64 = u64::MAX;
    pub const CEASED_TO_EXIST: u8 = 128;
    pub const GUARANTEE_GRANTED: u8 = 129;
    pub const READ_INCOMPLETE: u8 = 130;
}

/// Derived by the recorder's Feed writer: an event dispatch still awaiting its choice, keyed by what
/// a choice is matched by (native event kind and id, instance, the receiving country's id), holding
/// the dispatch's FeedEvents identity (generation, sequence) and read hour. Deleted when the choice
/// is read (an immediate-effects option leaves it open) or a new generation begins.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedOpenEventRow {
    pub kind: u32,
    pub id: u32,
    pub instance: u32,
    pub receiver: u32,
    pub generation: u64,
    pub sequence: u64,
    pub read_hour: i32,
}

/// Derived by the recorder's Feed writer: a decision selected or activated and still awaiting a
/// later step, keyed by its FeedEvents identity (generation, sequence), holding what a later step is
/// matched by (the country's id, the decision token, the country and numeric targets) and its read
/// hour. Deleted when it completes, is cancelled, times out or is removed, at a read not complete or
/// not following the one before, and when a new generation begins.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedOpenDecisionRow {
    pub generation: u64,
    pub sequence: u64,
    pub country: u32,
    pub token: Vec<u8>,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
    pub read_hour: i32,
}

/// Derived by the recorder's Feed writer: a guarantee relation (first and second raw references,
/// start hour) starting after the run's first hour while RelationTuples rows hold it, cancelled or
/// not: `rows` how many do, `hour` the hour it was written to FeedEvents (none while no read has
/// listed it uncancelled). Deleted when no row holds it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedGuaranteeRow {
    pub first: u32,
    pub second: u32,
    pub start: i32,
    pub hour: Option<i32>,
    pub rows: u32,
}

/// Derived by the recorder's NavalControl writer from the tables the naval control rule reads
/// (NavalSupremacy, Countries, CountryLinks, FactionMembers, Subjects, WarRelations, RelationTuples,
/// AirCountryRelations; ReservedCountryNaming and CountryCounterInputs for the reserved slot),
/// whenever one of them changes: the rule's operands for one viewing country (canonical slot) in
/// one sea region. `friendly` and `enemy` are the control's sums of the region's currents of the
/// viewer's family and of the countries at war with it; `ratio_friendly` and `ratio_enemy` the
/// explanatory share's operands before the region's minimum floors the enemy's; `own` the viewer's
/// own current. A reader takes control as `control(friendly, enemy, minimum, ratio)` and the share
/// as `share(ratio_friendly, max(ratio_enemy, minimum))` with the region's minimum supremacy and the
/// controlled-ratio define of the recorded content. A viewer without a row in a region holds the
/// region's [`Self::EVERY_VIEWER`] row, or all zeros without one, while the table is complete. A
/// pair's cells the rule refused are not recorded, with the refusal in `control_refusal` or
/// `share_refusal` (empty when none); `own` is not recorded while an owner of a nonzero current
/// there has no link.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRegionControlRow {
    pub viewer: u32,
    pub region: u32,
    pub friendly: Recorded<i64>,
    pub enemy: Recorded<i64>,
    pub ratio_friendly: Recorded<i64>,
    pub ratio_enemy: Recorded<i64>,
    pub own: Recorded<i64>,
    pub control_refusal: Vec<u8>,
    pub share_refusal: Vec<u8>,
}

impl NavalRegionControlRow {
    /// The viewer of a region's row every viewer without a row of its own holds.
    pub const EVERY_VIEWER: u32 = u32::MAX;
}

/// Derived by the recorder's NukeTotals writer from Nukes and Countries, keyed by raw country id:
/// a country's nuclear stockpile as the top bar adds it. `total` is every type's whole bombs
/// ([`NukeRow::shown`]) added with signed 32-bit wrapping, `ready` every type's queued drops whose
/// delay elapsed. Every live country has a row, so one without a stockpile holds a recorded zero;
/// a cell is not recorded while one of the country's Nukes cells it adds is not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryNukeTotalRow {
    pub country: u32,
    pub total: Recorded<i64>,
    pub ready: Recorded<i64>,
}

/// Derived by the recorder's ThreatTotals writer from ThreatCountryAggregates: every country's
/// contribution to world tension added, before the game caps the sum into the tension it shows,
/// in contribution units. The singleton row is not recorded while an aggregate it adds is not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatTotalRow {
    pub sum: Recorded<i64>,
}

/// Derived by the recorder's FactionTotals writer from GroupSpans, Groups, Countries,
/// TerritoryTotals and CountryFactories, keyed by the faction's recorded id: each measure
/// faction strength sums (territory's twelve, the factory counts' twenty-one), added over the
/// faction's members at the hour, as each member's own row gives it (a factory count by the
/// game's rule). A faction has a row while it has a member. A cell is not recorded while the
/// faction's roster, or a member's cell it adds, is not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionTotalRow {
    pub faction: u64,
    pub owned_states: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub occupied_states: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub controlled_provinces: Recorded<i64>,
    pub occupied_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub occupied_victory_points: Recorded<i64>,
    pub owned_area: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub occupied_area: Recorded<i64>,
    pub free1: Recorded<i64>,
    pub free2: Recorded<i64>,
    pub free3: Recorded<i64>,
    pub free4: Recorded<i64>,
    pub free5: Recorded<i64>,
    pub free6: Recorded<i64>,
    pub free7: Recorded<i64>,
    pub built1: Recorded<i64>,
    pub built2: Recorded<i64>,
    pub built3: Recorded<i64>,
    pub built4: Recorded<i64>,
    pub built5: Recorded<i64>,
    pub built6: Recorded<i64>,
    pub built7: Recorded<i64>,
    pub top_bar1: Recorded<i64>,
    pub top_bar2: Recorded<i64>,
    pub top_bar3: Recorded<i64>,
    pub top_bar4: Recorded<i64>,
    pub top_bar5: Recorded<i64>,
    pub top_bar6: Recorded<i64>,
    pub top_bar7: Recorded<i64>,
}

/// Derived by the recorder's RaidEnds writer from Raids, RaidTargets, Provinces, States and
/// CountryLinks: a row living the one hour a raid's end was read, keyed by that hour and the
/// raid's id and reference type: its row first read ended ([`Self::ENDED`]) or gone without an
/// ended read ([`Self::DELETED`]). A raid has at most one in a run, none when it was already ended
/// at the writer's first read. `previous_read` is the hour Raids was read before (earlier than the
/// hour before across hours it was not complete); `first_hour` the first hour the raid's row was
/// read (none when first read after such hours), `first_exact` when that read followed one without
/// it; `launched_hour` its first read in progress or with a finite nonzero distance flown. The
/// raid's columns are its row's at the end (a deleted raid's last read; not recorded for a raid
/// gone during hours Raids was not complete, whose last read is lost), the target's its
/// RaidTargets row's, not recorded without one. `target_state` is the target's state by the game's
/// location priority and `victim` its country by the game's target rule (none when it names none),
/// each read at the end hour, so an ended raid's victim is frozen at its first ended read;
/// `victim_slot` is the victim's canonical slot ([`TagHolderRow::NONE`] when not known) and
/// `victim_unknown` why the victim or its slot is not known.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidEndRow {
    pub hour: i32,
    pub id: u32,
    pub reference_type: u32,
    pub kind: u8,
    pub previous_read: i32,
    pub first_hour: Option<i32>,
    pub first_exact: bool,
    pub launched_hour: Option<i32>,
    /// The raider's canonical slot.
    pub raider: Recorded<u32>,
    pub phase: Recorded<i32>,
    pub outcome: Recorded<i32>,
    /// Fixed5 distance flown.
    pub travelled: Recorded<Option<i64>>,
    pub preparation_hours: Recorded<i32>,
    pub province: Recorded<u32>,
    pub state: Recorded<u32>,
    pub leader_reference: Recorded<u64>,
    pub leader_province: Recorded<u32>,
    pub building_type: Recorded<u32>,
    pub building_location: Recorded<u32>,
    pub building_flags: Recorded<u32>,
    pub building_country: Recorded<i32>,
    pub leader_country: Recorded<Option<i32>>,
    /// The raid type's lexer token, zero when the game names none.
    pub raid_type: Recorded<u32>,
    pub target_state: Recorded<Option<u32>>,
    /// Raw country id.
    pub victim: Recorded<Option<u32>>,
    pub victim_slot: u32,
    pub victim_unknown: u8,
}

impl RaidEndRow {
    pub const ENDED: u8 = 0;
    pub const DELETED: u8 = 1;
    /// The victim is known (or known to be none).
    pub const VICTIM_KNOWN: u8 = 0;
    /// The raid's target is not recorded.
    pub const TARGET_UNKNOWN: u8 = 1;
    /// The targeted province's or state's row is not recorded.
    pub const LOCATION_UNKNOWN: u8 = 2;
    /// The victim's country has no link to a slot.
    pub const VICTIM_UNLINKED: u8 = 3;

    /// The key of the end at `hour` of the raid whose Raids key is `raid`.
    pub fn key_of(hour: i32, raid: u128) -> u128 {
        u128::from(hour as u32) << 64 | (raid & u128::from(u64::MAX))
    }
}

/// Derived by the recorder's AirMissions writer beside AirMissionCounts: each wing registered in
/// an owner's (canonical slot) strategic region, keyed by slot, region and wing, with its
/// registration's ordinal. A region's wings in registration order are its rows by `ordinal`. It
/// is available exactly when AirMissionCounts is, for the same reason.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionWingRow {
    pub slot: u32,
    pub region: u32,
    pub wing: u64,
    pub ordinal: u32,
}

/// Derived by the recorder's LeaderBattles writer beside LeaderBattles: each leader's (full
/// handle) LeaderBattles rows of the run through the hour, counted: participations (`fought`)
/// and results whose outcome is won or lost; none before its first row. A row is deleted in the
/// hour the writer reads the leader's UnitLeaders row deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderBattleTotalRow {
    pub leader: u64,
    pub fought: u32,
    pub won: u32,
    pub lost: u32,
}

/// Derived by the recorder's LeaderTraits writer beside LeaderTraitGains: per leader (full
/// handle) and trait token, the run's first LeaderTraitGains row (`first_hour`, `first_class`)
/// and its last gain (`gained_hour`, none before one). A trait's date through the hour is the last
/// gain, else the first row. A row is deleted in the hour the writer reads the leader's
/// UnitLeaders row deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderTraitDateRow {
    pub leader: u64,
    pub token: u32,
    pub first_hour: i32,
    pub first_class: u8,
    pub gained_hour: Option<i32>,
    pub spelling: Vec<u8>,
}

/// Derived by the recorder's LeaderCommands writer from OrdersGroups and Fleets: each formation
/// a leader commands now (`source` [`LeaderCommandRow::ORDERS_GROUP`] or
/// [`LeaderCommandRow::FLEET`], its handle), since `first_hour` (the first hour its leader column
/// named the leader since the writer last read it naming another or none), its kind and its
/// stored name then. A row lives while its formation names the leader.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderCommandSpanRow {
    pub source: u8,
    pub formation: u64,
    pub leader: u64,
    pub first_hour: i32,
    pub kind: u8,
    pub name: Recorded<Option<Vec<u8>>>,
}

/// Derived by the recorder's LeaderCommands writer: a log, each row living the hour it was
/// written, of each command span that ended (the formation named another leader or none, was
/// deleted, or its tables were not read), keyed by the leader's full handle, the hour and its
/// place among the leader's rows of the hour: a [`LeaderCommandSpanRow`]'s formation, kind and
/// name, its `first_hour` and `last_hour`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderCommandRow {
    pub leader: u64,
    pub hour: i32,
    pub ordinal: u32,
    pub source: u8,
    pub formation: u64,
    pub kind: u8,
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Recorded<Option<Vec<u8>>>,
}

impl LeaderCommandRow {
    pub const ORDERS_GROUP: u8 = 0;
    pub const FLEET: u8 = 1;
    pub const ARMY: u8 = 0;
    pub const ARMY_GROUP: u8 = 1;
    pub const NAVY: u8 = 2;
}

/// Derived by the recorder from CountryExperience and CountryExperienceGains, at each daily read of
/// CountryExperience, keyed by raw country id, in the game's experience units: each branch's banked
/// experience less the read exactly 24 hours earlier (`*_change`), and the sum of its hourly gross
/// gains over the 24 hours ending at the read (`*_gain`). A change is not recorded without a read
/// 24 hours earlier in the run's unbroken hours, a gain without 24 complete hourly intervals of the
/// country's one gains row. The read a row was written at is CountryExperience's read hour (the
/// table is read whole), which the recording holds in its pages ([`TableId::CountryExperience`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryExperienceDailyRow {
    pub country: u32,
    pub army_change: Recorded<i64>,
    pub navy_change: Recorded<i64>,
    pub air_change: Recorded<i64>,
    pub army_gain: Recorded<i64>,
    pub navy_gain: Recorded<i64>,
    pub air_gain: Recorded<i64>,
}

/// Derived by the recorder from the game-score inputs, CountryFactories and the air wings' stats:
/// each country's end-screen scores by the game's arithmetic, fixed5, keyed by raw country id.
/// Each part's reason is [`Self::KNOWN`] with its value, or names why the value is not recorded
/// ([`Self::REASONS`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryGameScoreRow {
    pub country: u32,
    pub army: Recorded<i64>,
    pub navy: Recorded<i64>,
    pub air: Recorded<i64>,
    pub industry: Recorded<i64>,
    pub land: Recorded<i64>,
    pub total: Recorded<i64>,
    pub army_reason: u8,
    pub navy_reason: u8,
    pub air_reason: u8,
    pub industry_reason: u8,
    pub land_reason: u8,
    pub total_reason: u8,
}

impl CountryGameScoreRow {
    pub const KNOWN: u8 = 0;
    pub const INPUTS_INCOMPLETE: u8 = 1;
    pub const NO_INPUT_ROW: u8 = 2;
    pub const COUNT_UNREAD: u8 = 3;
    pub const MULTIPLIER: u8 = 4;
    pub const AIR_ENTRY: u8 = 5;
    pub const BASE_ACCESS: u8 = 6;
    pub const NO_LINK: u8 = 7;
    pub const REGISTRATIONS_INCOMPLETE: u8 = 8;
    pub const NO_REGISTRATION: u8 = 9;
    pub const WINGS_INCOMPLETE: u8 = 10;
    pub const NO_WING: u8 = 11;
    pub const PLANES: u8 = 12;
    pub const STAT: u8 = 13;
    pub const TOTAL_PARTS: u8 = 14;
    pub const FACTORIES_INCOMPLETE: u8 = 15;
    pub const NO_FACTORIES_ROW: u8 = 16;
    /// Each reason's text, by code.
    pub const REASONS: [&'static str; 17] = [
        "",
        "CountryGameScoreInputs was not complete",
        "CountryGameScoreInputs held no row for this country",
        "the game's count was not read (the country's CountryGameScoreInputs row names why)",
        "the score multiplier was not read (GameScoreMultipliers names why)",
        "the strategic-air entry was not read (the country's CountryGameScoreInputs row names why)",
        "an air base's access was not read (the country's CountryGameScoreInputs row names why)",
        "no link resolves the country, so its air bases' registrations cannot be joined",
        "AirBaseRegistrations was not complete",
        "an accessible air base holds no registration for the country",
        "AirWingGameScoreStats was not complete",
        "a registered air wing has no AirWingGameScoreStats row",
        "an air wing's plane count was not read (its AirWingGameScoreStats row names why)",
        "an air wing's mission stat was not read (its AirWingGameScoreStats row names why)",
        "the total needs army, navy, air and the total multiplier",
        "CountryFactories was not complete",
        "CountryFactories held no row for this country",
    ];
}

/// Derived by the recorder from each country's CountryStabilityInputs row and CountryStabilityLines
/// at its read: each part of its displayed stability and war support, fixed5, keyed by raw country
/// id, measure ([`Self::STABILITY`], [`Self::WAR_SUPPORT`]) and the part's handle in
/// [`StabilityPartKeyRow`]. A read's parts add up to its displayed value: each nonzero formula
/// term, each source of the modifier term's lines, what the lines leave of their total, and the
/// clamp. The part of kind [`StabilityPartKeyRow::TOTAL`] is the displayed value and lives while
/// the country has parts; it is not recorded, and no other part has a row, while the read's parts
/// are not known (a term not recorded, or the read's lines not recorded with it). A part with no
/// row while the total is recorded is zero.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StabilityPartRow {
    pub country: u32,
    pub measure: u8,
    pub handle: u32,
    pub value: Recorded<i64>,
}

impl StabilityPartRow {
    pub const STABILITY: u8 = 0;
    pub const WAR_SUPPORT: u8 = 1;
}

/// Derived beside [`StabilityPartRow`]: each part source a run met, numbered, never deleted. A
/// line's source is its kind (a [`CountryStabilityLineRow`] kind), identity token, identity id and
/// secondary; for a country leader or advisor also its character as Characters held it when the
/// part was met: script token (empty for a character with none), saved name, and the template it
/// was built from (`template_present` 0 for none; its tag spelled by LexicalTokens). Each of
/// those is not recorded while not known: Characters not complete or holding no such row, a token's
/// spelling not held. An unpaired line's identity is its recorded label as plain text, as a reply
/// shows it; a label plain text cannot show is a [`Self::REFUSED_LABEL`] part, its identity the
/// recorded bytes. A term's identity is its formula term name; a remainder, clamp or total has
/// none. Fields a kind has none of are zero or empty.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StabilityPartKeyRow {
    pub handle: u32,
    pub kind: u8,
    pub identity: Vec<u8>,
    pub identity_id: u32,
    pub secondary: i64,
    pub script: Recorded<Vec<u8>>,
    pub saved: Recorded<Vec<u8>>,
    pub template_present: Recorded<u8>,
    pub template_tag: Recorded<Vec<u8>>,
    pub template_file: Recorded<Vec<u8>>,
    pub template_line: Recorded<i32>,
    pub template_generated: Recorded<u8>,
}

impl StabilityPartKeyRow {
    /// A formula term other than the modifier term.
    pub const TERM: u8 = 9;
    /// The modifier term's total less its lines.
    pub const REMAINDER: u8 = 10;
    /// The displayed value less the unclamped sum.
    pub const CLAMP: u8 = 11;
    /// The displayed value itself.
    pub const TOTAL: u8 = 12;
    /// An unpaired line whose recorded label plain text cannot show.
    pub const REFUSED_LABEL: u8 = 13;
}

/// Derived by the recorder from ThreatSources: a row living the one hour a threat source was
/// first listed or listed again ([`Self::LISTED`]), what it adds changed ([`Self::CHANGED`]), or it
/// was no longer listed ([`Self::DROPPED`]); a run's first hour lists none. A source is its country
/// and second country references, date and reason, and its place among sources equal in all four.
/// `slot` is the canonical slot its country resolved to at the hour (for a dropped source no link
/// resolves now, the one it resolved to when last listed), [`TagHolderRow::NONE`] when none did;
/// `ordinal` its place in the listing (the last listing it was in, when dropped); `final_raw` what
/// it adds once its ramp ends, as last listed; `place` its place among the sources before it in
/// that listing equal in country, second country, date and reason, so a reader follows one source
/// from its listing to its drop.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatEventRow {
    pub kind: u8,
    pub slot: u32,
    pub hour: i32,
    pub ordinal: u32,
    pub country: u32,
    pub second_country: u32,
    pub date: i32,
    pub reason: Vec<u8>,
    pub final_raw: i64,
    pub place: u32,
}

impl ThreatEventRow {
    pub const LISTED: u8 = 0;
    pub const CHANGED: u8 = 1;
    pub const DROPPED: u8 = 2;
}

/// The widest country slot or country id a 24-bit key field holds; [`TagHolderRow::NONE`] is
/// written as it.
pub const KEY_FIELD_NONE: u32 = (1 << 24) - 1;

/// A 24-bit key field: none as [`KEY_FIELD_NONE`].
pub const fn key_field(value: u32) -> u128 {
    (if value == TagHolderRow::NONE { KEY_FIELD_NONE } else { value & KEY_FIELD_NONE }) as u128
}

/// A 24-bit key field back: [`KEY_FIELD_NONE`] as none.
pub const fn field_of_key(key: u128, shift: u32) -> u32 {
    let value = (key >> shift) as u32 & KEY_FIELD_NONE;
    if value == KEY_FIELD_NONE { TagHolderRow::NONE } else { value }
}

/// Whether `value` fits a 24-bit key field: under [`KEY_FIELD_NONE`], or none.
pub const fn fits_key_field(value: u32) -> bool {
    value < KEY_FIELD_NONE || value == TagHolderRow::NONE
}

/// Derived by the recorder from CountryFuel at each of its reads, keyed by raw country id: the
/// day of fuel updates the read holds, raw. A day comes from the applied updates
/// (`CountryFuel.award_day`) when they are known for this read: then `gained` is the own gain
/// (fixed5), `burned` the consumption and `other` the total other transfer (/32768), `received_*`
/// the consumers' receipts (fixed5), and the applied-only columns hold the gain sources (fixed5),
/// the own gain converted (/32768) and the incoming, outgoing and flat transfers (/32768). Else
/// the day is the ring's sums (`ring_*`) in the first six columns, the applied-only ones not
/// recorded, when every governing hour shows the country owning a state; else it is left out
/// ([`Self::OVERLAPS`] ...) and no day column is recorded. `awards_reason` is why the applied
/// updates are not known (empty when they are); `overlapped` the earlier read whose kept day this
/// read's day leaves out, so no update is counted twice. [`Self::measures`] is the Fuel chart's
/// view of a day.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDayRow {
    pub country: u32,
    pub read_hour: i32,
    pub gained: Recorded<i64>,
    pub burned: Recorded<i64>,
    pub received_army: Recorded<i64>,
    pub received_air: Recorded<i64>,
    pub received_navy: Recorded<i64>,
    pub other: Recorded<i64>,
    pub gain_base: Recorded<i64>,
    pub gain_states: Recorded<i64>,
    pub gain_oil: Recorded<i64>,
    pub gain_negative_cost: Recorded<i64>,
    pub own_gain_quantity: Recorded<i64>,
    pub incoming: Recorded<i64>,
    pub outgoing: Recorded<i64>,
    pub flat: Recorded<i64>,
    pub left_out: u8,
    pub awards_reason: Vec<u8>,
    pub continuity_start_hour: Option<i32>,
    pub overlapped: Option<i32>,
}

impl FuelDayRow {
    pub const KEPT: u8 = 0;
    /// It overlaps a day already kept.
    pub const OVERLAPS: u8 = 1;
    /// The country held no state at some governing hour, or was another country.
    pub const NOT_UPDATED: u8 = 2;
    /// Some governing hour was not recorded, or recorded without what tells ownership.
    pub const OWNERSHIP_NOT_RECORDED: u8 = 3;
    /// The day's raw columns, in [`Self::measures`]' order.
    pub const RAW: [&'static str; 14] = [
        "gained",
        "burned",
        "received_army",
        "received_air",
        "received_navy",
        "other",
        "gain_base",
        "gain_states",
        "gain_oil",
        "gain_negative_cost",
        "own_gain_quantity",
        "incoming",
        "outgoing",
        "flat",
    ];

    /// The Fuel chart's 15 measures of a day from its [`Self::RAW`] cells: the gain strip
    /// (fixed5), the burn strip (/32768), then in /32768 x fixed5 units clamped at zero the four
    /// gain sources, the unidentified gain (a ring day's), incoming, outgoing and flat transfers,
    /// the other transfers, what conversion and tank clamps took, and the three consumers'
    /// receipts. None where the day's kind holds none, a cell is not recorded, or a value passes
    /// i64.
    pub fn measures(raw: [Option<i64>; 14]) -> [Option<i64>; 15] {
        let quantity = i128::from(engine_reference_values::native_values::fuel_status::QUANTITY_SCALE);
        let gain = i128::from(engine_reference_values::native_values::fuel_status::MODIFIER_SCALE);
        let wide = raw.map(|cell| cell.map(i128::from));
        let [gained, burned, army, air, navy, other, base, states, oil, negative, converted, incoming, outgoing, flat] = wide;
        let received = [army, air, navy];
        let received_sum = received.iter().try_fold(0_i128, |sum, value| value.map(|value| sum + value));
        let mut values: [Option<i128>; 15] = [None; 15];
        values[0] = gained;
        values[1] = burned;
        for (at, value) in received.into_iter().enumerate() {
            values[12 + at] = value.map(|value| (value * quantity).max(0));
        }
        if converted.is_some() {
            for (at, value) in [base, states, oil, negative].into_iter().enumerate() {
                values[2 + at] = value.map(|value| value.max(0));
            }
            values[7] = incoming.map(|value| value.max(0));
            values[8] = outgoing.map(|value| (value * gain).max(0));
            values[9] = flat.map(|value| (value * gain).max(0));
            values[10] = other.zip(outgoing.zip(flat)).map(|(other, (outgoing, flat))| ((other - outgoing - flat) * gain).max(0));
        } else {
            values[6] = gained.map(|value| value.max(0));
            values[10] = other.map(|value| (value * gain).max(0));
        }
        values[11] = burned.zip(other).zip(received_sum).map(|((burned, other), received)| (burned * gain - other * gain - received * quantity).max(0));
        values.map(|value| value.and_then(|value| i64::try_from(value).ok()))
    }
}

/// Derived by the recorder from each CountryFuel row's demand pack and its FuelDemandParts, keyed
/// by raw country id, by family in the Fuel consumers' order (army, air, navy): the game's daily
/// projection of its own wrapping family sum (`native_*`, not recorded with `*_reason` saying why),
/// and the widened sums of each known part's daily projection, as is and clamped at zero. While
/// the pack is not known every cell is not recorded and each reason names why.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandTotalRow {
    pub country: u32,
    pub native_army: Recorded<i64>,
    pub native_air: Recorded<i64>,
    pub native_navy: Recorded<i64>,
    pub known_army: Recorded<i64>,
    pub known_air: Recorded<i64>,
    pub known_navy: Recorded<i64>,
    pub positive_army: Recorded<i64>,
    pub positive_air: Recorded<i64>,
    pub positive_navy: Recorded<i64>,
    pub army_reason: Vec<u8>,
    pub air_reason: Vec<u8>,
    pub navy_reason: Vec<u8>,
}

/// Derived by the recorder from ResourceExchanges: the deals of one resource from one seller to
/// one buyer, each side its holder identity (canonical slot, raw id of the holder's Countries row),
/// summed; a row lives while a deal counts toward it. A side no link or holder resolves is
/// [`TagHolderRow::NONE`] in both fields. A sum past i64 is not recorded. The key's slots and ids
/// are 24-bit fields ([`key_field`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TradeFlowRow {
    pub seller_slot: u32,
    pub seller: u32,
    pub buyer_slot: u32,
    pub buyer: u32,
    pub resource: u32,
    pub delivered: Recorded<i64>,
    pub factories: Recorded<i64>,
    pub convoys_assigned: Recorded<i64>,
    pub convoys_required: Recorded<i64>,
    pub deals: i64,
}

/// Derived by the recorder from BuildingConstruction, RailwayConstruction, the building tables
/// and ConstructionLineOutput: one construction line, by its native handle, from the first read
/// that lists it (or the output read that names it alone) until the read that settles its ending,
/// written a last time there and deleted the hour after. What it was as first read (kind, queue
/// owner, target, cost, creation, queue ordinal), its current run (factories, levels left, repair
/// levels left), its counts so far (levels built, repaired, added, possible, unaccounted), how it
/// ended, and its output since observation began. A count's entries are its changes: each rise is
/// the read that counted it, dated by the rise list beside it where it has one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionProjectRow {
    pub line: u64,
    /// [`Self::BUILDING`] or [`Self::RAILWAY`].
    pub kind: u8,
    /// Named by the output alone: no queue read listed it.
    pub output_alone: bool,
    /// The queue owner's raw country id as first read.
    pub country: u32,
    pub building_type: Option<u32>,
    pub target_state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
    pub route_provinces: Option<u32>,
    pub base_level: Option<i32>,
    pub target_level: Option<i32>,
    pub cost: Option<i64>,
    pub created_hour: Option<i32>,
    pub first_read: i32,
    /// The first queue read that listed it; none while only the output names it.
    pub census_read: Option<i32>,
    pub queue_ordinal: Option<u32>,
    pub earlier_unknown: bool,
    pub factories: Recorded<i32>,
    pub remaining: Recorded<i32>,
    pub repair_levels: Recorded<i32>,
    /// New levels (railway: segments) built so far.
    pub built: i64,
    /// The rises that date the new levels the row's last queue read counted: empty when that read
    /// dated them by itself (or counted none).
    pub built_rises: Vec<ConstructionRise>,
    pub repaired: i64,
    /// The rises of the levels the row's last queue read counted repaired.
    pub repaired_rises: Vec<ConstructionRise>,
    pub added_new: i64,
    pub added_repair: i64,
    /// New levels it may have both built and had added.
    pub possible: i64,
    pub unaccounted_new: i64,
    pub unaccounted_repair: i64,
    /// [`Self::OPEN`], [`Self::FINISHED`], [`Self::REMOVED`] or [`Self::UNKNOWN_END`].
    pub end: u8,
    /// Why an unknown ending is not known ([`Self::NOT_RECORDED`] ...); 0 otherwise.
    pub end_reason: u8,
    pub end_hour: Option<i32>,
    pub end_known_at: Option<i32>,
    pub construction_raw: Recorded<i64>,
    pub repair_raw: Recorded<i64>,
    pub output_since: Option<i32>,
    /// The output row's observation began at the line's creation: none of its output before
    /// observation is missing.
    pub observed_since_creation: bool,
    /// [`Self::OUTPUT_KNOWN`] or why the output is not known.
    pub output_reason: u8,
    pub first_repair_tick: Option<i32>,
}


/// ConstructionLineEnds: a log of construction lines' ends, a row living the hour a line's
/// `ConstructionProjects` row is written a last time, carrying what the line was and did over its
/// whole life, so a run's index of its lines is one read of the log, not the projects' history.
/// Keyed by that hour and the line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionLineEndRow {
    pub hour: i32,
    pub line: u64,
    pub kind: u8,
    pub output_alone: bool,
    /// The queue owner's raw country id as first read.
    pub country: u32,
    pub building_type: Option<u32>,
    pub target_state: Option<u32>,
    pub to_state: Option<u32>,
    pub first_read: i32,
    /// It built, was given, may have built or lost new levels, had some left, or had construction output.
    pub new_work: bool,
    /// It repaired, was given repair levels, lost some unseen, had some left, had repair output or a repair tick.
    pub repair_work: bool,
    /// Its output's observation began after its creation: output before is not recorded.
    pub before_observation: bool,
    pub had_output: bool,
    pub output_reason: u8,
}

impl ConstructionLineEndRow {
    pub const fn key_of(hour: i32, line: u64) -> u128 {
        (hour as u32 as u128) << 64 | line as u128
    }

    /// The end at `hour` of the line whose final row is `row`.
    pub fn of(hour: i32, row: &ConstructionProjectRow) -> Self {
        let positive = |cell: &Recorded<i32>| matches!(cell, Recorded::Value(count) if *count > 0);
        let wide = |cell: &Recorded<i64>| matches!(cell, Recorded::Value(amount) if *amount > 0);
        Self {
            hour,
            line: row.line,
            kind: row.kind,
            output_alone: row.output_alone,
            country: row.country,
            building_type: row.building_type,
            target_state: row.target_state,
            to_state: row.to_state,
            first_read: row.first_read,
            new_work: [row.built, row.added_new, row.possible, row.unaccounted_new].into_iter().any(|count| count > 0) || positive(&row.remaining) || wide(&row.construction_raw),
            repair_work: [row.repaired, row.added_repair, row.unaccounted_repair].into_iter().any(|count| count > 0)
                || positive(&row.repair_levels)
                || wide(&row.repair_raw)
                || row.first_repair_tick.is_some(),
            before_observation: row.output_since.is_some() && !row.observed_since_creation,
            had_output: row.output_since.is_some(),
            output_reason: row.output_reason,
        }
    }
}

impl ConstructionProjectRow {
    pub const BUILDING: u8 = 0;
    pub const RAILWAY: u8 = 1;
    pub const OPEN: u8 = 0;
    pub const FINISHED: u8 = 1;
    pub const REMOVED: u8 = 2;
    pub const UNKNOWN_END: u8 = 3;
    pub const NOT_RECORDED: u8 = 1;
    pub const SHARED_TARGET: u8 = 2;
    pub const NOTHING_LEFT: u8 = 3;
    pub const COMPLETION_NOT_RECORDED: u8 = 4;
    pub const OUTPUT_KNOWN: u8 = 0;
    pub const OUTPUT_NOT_RECORDED: u8 = 1;
    pub const OUTPUT_LINKED: u8 = 2;
    pub const OUTPUT_INEXACT: u8 = 3;
    pub const OUTPUT_RESTARTED: u8 = 4;
    pub const OUTPUT_FINAL_MISSING: u8 = 5;
    pub const OUTPUT_OWNER_UNKNOWN: u8 = 6;
}

/// Derived by the recorder from ConstructionLineOutput: the civilian-factory output one country
/// identity (canonical slot, raw id of its holder) spent on one kind of work (new levels or
/// repair) of one building type at one state (a railway: from its route's first province's state
/// to its last's), summed from each line's observed advances since observation began, fixed5;
/// rows are never deleted. Output no identity resolves is [`TagHolderRow::NONE`] in both fields;
/// a total past i64 is not recorded. `unknown` counts the lines whose output the total lacks:
/// those whose current output is not exact (linked, inexact) and those that ended before an exact
/// output was read; `total` is then a lower bound. The key's slot and id are 24-bit fields
/// ([`key_field`]); its state is 32 bits, and one 32-bit field holds the building type of a building
/// line or the last state of a railway (which has no building type), told apart by the railway bit.
/// A state or type the line does not name is [`TagHolderRow::NONE`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionSpendRow {
    pub slot: u32,
    pub country: u32,
    pub repair: bool,
    pub railway: bool,
    pub building_type: u32,
    pub state: u32,
    pub to_state: u32,
    pub total: Recorded<i64>,
    pub unknown: i64,
}

/// A building line naming no last state, so the default row is one its key holds.
impl Default for ConstructionSpendRow {
    fn default() -> Self {
        Self {
            slot: 0,
            country: 0,
            repair: false,
            railway: false,
            building_type: 0,
            state: 0,
            to_state: TagHolderRow::NONE,
            total: Recorded::default(),
            unknown: 0,
        }
    }
}

/// Derived by the recorder (the division log): a row living the hour a division was first seen
/// in the run's Divisions ([`Self::CREATED`]; never at the run's first complete census, the
/// baseline), a DivisionEquipmentDeletions witness named its removal ([`Self::LOST`],
/// [`Self::TRANSFER`]), or it left the census with no witness, which is no loss
/// ([`Self::VANISHED`]), keyed with its kind, the read hour and the division's full handle.
/// `disposition` and `cause` are the witness's, none for a sighting; `owner` the raw owner
/// reference (the positive expeditionary owner, else the roster owner or the witness's country),
/// `slot` its canonical slot at the hour, [`TagHolderRow::NONE`] when no link resolves it.
/// `native_hour` is the covered death's hour when `exact`, else the read hour, an upper bound;
/// `after`, for a sighting, the run's last complete census before it, a lower bound. The
/// division's last province and template (Divisions), name operands (DivisionStatus) and its
/// template's icon operands (DivisionTemplateIcons) are copied as the hour or, for a removed
/// division, the deletion found them, each with the hour its table supplied it; a removal dated by
/// a covered death holds the death's province. Not recorded where its table held no row.
/// `template_id` is the template handle's id (its high half), the log's template facet;
/// `matched_deaths`, for a destroyed or overrun division, how many covered deaths matched it
/// (`exact` when one). The template's display-name operands at the read hour: its literal name,
/// else (literal empty) the localisation key named for the template's raw country
/// (`template_key`, not recorded when the literal names it); not recorded where
/// DivisionTemplateIcons and DivisionTemplates did not give them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEventRow {
    pub kind: u8,
    pub hour: i32,
    pub division: u64,
    pub disposition: Option<u32>,
    pub cause: Option<u32>,
    pub owner: u32,
    pub slot: u32,
    pub native_hour: i32,
    pub exact: bool,
    pub after: Option<i32>,
    pub province: Recorded<u32>,
    pub place_hour: Recorded<i32>,
    pub template: Recorded<u64>,
    pub template_hour: Recorded<i32>,
    pub name: Recorded<Option<Vec<u8>>>,
    pub naming: Recorded<Option<NameOperands>>,
    pub name_hour: Recorded<i32>,
    pub icon_unit: Recorded<u32>,
    pub icon_custom: Recorded<i32>,
    pub icon_name: Recorded<Vec<u8>>,
    pub icon_hour: Recorded<i32>,
    /// A covered death's killer: its raw country and that country's canonical slot at the hour
    /// (none when no link resolves it), and its division when the death names one.
    pub killer_country: Option<u32>,
    pub killer_slot: Option<u32>,
    pub killer_division: Option<u64>,
    pub template_id: Recorded<u32>,
    pub matched_deaths: Option<u32>,
    pub template_literal: Recorded<Vec<u8>>,
    pub template_key: Recorded<Vec<u8>>,
    pub template_country: Recorded<i32>,
}

impl DivisionEventRow {
    pub const CREATED: u8 = 0;
    pub const LOST: u8 = 1;
    pub const TRANSFER: u8 = 2;
    pub const VANISHED: u8 = 3;
}

/// Derived by the recorder from the combats' commanders and their results: a row living the hour
/// a leader was first read commanding a combat ([`Self::PARTICIPATION`]) and one the hour that
/// combat's result was read ([`Self::RESULT`]), keyed with the leader's full handle, the read
/// hour and its place among the leader's rows of the hour. `kind` is [`Self::LAND`] or
/// [`Self::NAVAL`]; `sides` the sides it commanded (bit 0 attacker, bit 1 defender).
/// A participation's `start_hour` is the hour read, or the result's date when first seen in a
/// result; `start_unknown` when the combat was not known absent the hour before. A result's
/// `winners` (bit 0 attacker, bit 1 defender, as the game finalized it; not recorded when two
/// results of the combat disagree or the game's selector is not one this reads), `result_hour`
/// (the result's own date) and `outcome` ([`Self::WON`], [`Self::LOST`]; not recorded unless one
/// side won, the leader commanded one side and the result is dated from its participation on).
/// A participation leaves the result's columns not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderBattleRow {
    pub leader: u64,
    pub hour: i32,
    pub ordinal: u32,
    pub entry: u8,
    pub kind: u8,
    pub combat: u64,
    pub sides: u8,
    pub start_hour: i32,
    pub start_unknown: bool,
    pub winners: Recorded<u8>,
    pub result_hour: Recorded<i32>,
    pub outcome: Recorded<u8>,
}

/// A land participation, so the default row is one its codec holds.
impl Default for LeaderBattleRow {
    fn default() -> Self {
        Self {
            leader: 0,
            hour: 0,
            ordinal: 0,
            entry: Self::PARTICIPATION,
            kind: Self::LAND,
            combat: 0,
            sides: 0,
            start_hour: 0,
            start_unknown: false,
            winners: Recorded::default(),
            result_hour: Recorded::default(),
            outcome: Recorded::default(),
        }
    }
}

impl LeaderBattleRow {
    pub const PARTICIPATION: u8 = 0;
    pub const RESULT: u8 = 1;
    pub const LAND: u8 = 1;
    pub const NAVAL: u8 = 2;
    pub const WON: u8 = 1;
    pub const LOST: u8 = 2;
}

/// Derived by the recorder from UnitLeaderTraits: a log, each row living the hour it was read,
/// of a trait (its token id) a leader was read gaining: [`Self::HELD_BEFORE`] for one held at the
/// run's first hour, [`Self::GAINED`] when the leader's traits were read the hour before without
/// it, [`Self::FIRST_OBSERVED`] when they were not (a new leader, or every trait held when the
/// traits are read after hours they were not: one the run saw before is no gain). `spelling` is
/// the trait's token as UnitLeaderTraits spelled it then.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderTraitGainRow {
    pub leader: u64,
    pub hour: i32,
    pub token: u32,
    pub class: u8,
    pub spelling: Vec<u8>,
}

impl LeaderTraitGainRow {
    pub const HELD_BEFORE: u8 = 0;
    pub const GAINED: u8 = 1;
    pub const FIRST_OBSERVED: u8 = 2;
}

/// Derived by the recorder's Research writer from ResearchInputs, ResearchTechnologies,
/// SliceReads, Countries and CountryLinks: a holder's (a country whose link resolves to its own
/// canonical slot) research assignments by slot. Lane 0 holds the slot's latest assignment; a
/// higher lane holds an earlier one still awaiting completion evidence, deleted when that wait
/// ends. A row changes only at the holder's reads: an assignment observed, ended with its
/// outcome, completed late, or ended unknown at a break in continuity; the history of a slot's
/// assignments is the rows' change history. `database_index` is the technology's template
/// occurrence, `level` its level when first observed, `last_observed_hour` not recorded (none)
/// while the assignment is active (its last read is the holder's), `awaiting` 1 while a later
/// read may still prove an ended assignment completed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchAssignmentRow {
    pub holder_slot: u32,
    pub holder_id: u32,
    pub index: u32,
    pub lane: u16,
    pub technology_handle: u32,
    pub database_index: i32,
    pub token: Vec<u8>,
    pub level: i32,
    pub continuity_start_hour: i32,
    pub first_observed_hour: i32,
    pub started_after_hour: Option<i32>,
    pub last_observed_hour: Option<i32>,
    pub ended_by_hour: Option<i32>,
    pub outcome: u8,
    pub outcome_observed_hour: Option<i32>,
    pub awaiting: u8,
}

impl ResearchAssignmentRow {
    pub const ACTIVE: u8 = 0;
    pub const COMPLETED: u8 = 1;
    pub const SWITCHED: u8 = 2;
    pub const UNKNOWN: u8 = 3;
}

/// Derived beside [`ResearchAssignmentRow`]: a row lives while a holder's research is covered by
/// a supplying read (`member` [`Self::HOLDER`]) and while each of its slots is available
/// (`member` the slot index): from a read through the hours before the next is due, while the
/// holder and its owners' read hours hold. `since` is the open span's first hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchCoverageRow {
    pub holder_slot: u32,
    pub holder_id: u32,
    pub member: u32,
    pub since: i32,
}

impl ResearchCoverageRow {
    pub const HOLDER: u32 = u32::MAX;
}

/// Derived by the recorder's FactionAccounts writer from FactionAccounts, FactionMembers,
/// Countries and CountryLinks: each faction account (its kind-0 row) whose country, under one
/// holder identity (`slot`, `id`) since the run's hours last broke, was an explicit member of a
/// faction: the last such `faction`, and `member` 1 while it still is. Deleted when the account
/// row goes, its country no longer resolves to the same holder, or its membership is ambiguous.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionAccountOwnerRow {
    pub account: u32,
    pub faction: u64,
    pub slot: u32,
    pub id: u32,
    pub member: u8,
}

/// Derived by the recorder's Peace writer: what one peace outcome changed between two complete
/// reads (`before_hour` and the key's `after_hour`), a row living the after hour (a log). A
/// sampled conference's comparison ([`Self::RECORDED`], its `conference`), or owner changes
/// between consecutive hours no sampled conference explains, `group` numbering the connected
/// pairs of one hour ([`Self::NOT_RECORDED`] when complete rosters prove no conference resident,
/// else [`Self::UNKNOWN`]). A state transfer's `from` is the former owner's holder (canonical slot
/// and country id) at the before read, `to` the new owner's at the after read; a new subject's
/// `from` is the subject, `to` the overlord; a government change's both are the country; a
/// released country's row is one per state it received, `from` that state's former owner.
/// `state` is [`Self::NONE`] for kinds naming none; `victory_points` the state's at the after
/// read, none when Provinces cannot supply it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceOutcomeRow {
    pub after_hour: i32,
    pub ordinal: u32,
    pub observation: u8,
    pub conference: Option<u64>,
    pub before_hour: i32,
    pub change: u8,
    pub group: u32,
    pub state: u32,
    pub from_slot: u32,
    pub from_id: u32,
    pub to_slot: u32,
    pub to_id: u32,
    pub victory_points: Option<i64>,
}

impl PeaceOutcomeRow {
    pub const RECORDED: u8 = 0;
    pub const NOT_RECORDED: u8 = 1;
    pub const UNKNOWN: u8 = 2;
    pub const TRANSFER: u8 = 0;
    pub const NEW_SUBJECT: u8 = 1;
    pub const GOVERNMENT: u8 = 2;
    pub const RELEASED: u8 = 3;
    pub const NONE: u32 = u32::MAX;
}

/// Derived beside [`PeaceOutcomeRow`], a log whose rows live the hour they were written: a peace
/// conference's span at each hour a roster read or its comparison moved it, its last row up to an
/// hour being its span then: the first and last hours a read listed it, its turn and finalized flag
/// as last read, the first read finding it finalized and the first finding it gone, and its
/// comparison:
/// [`Self::WAITING`] for a complete read after it ended, [`Self::COMPARED`] (its outcome rows
/// live `compared_hour`, measured from `before_hour`), [`Self::NO_BEFORE`] (no complete read
/// before it) or [`Self::OVERLAPPED`] (other sampled conferences overlap it).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceConferenceSpanRow {
    pub conference: u64,
    pub hour: i32,
    pub first_observed_hour: i32,
    pub last_observed_hour: i32,
    pub current_turn: i32,
    pub finalized: u8,
    pub first_finalized_hour: Option<i32>,
    pub first_absent_hour: Option<i32>,
    pub comparison: u8,
    pub before_hour: Option<i32>,
    pub compared_hour: Option<i32>,
}

impl PeaceConferenceSpanRow {
    pub const WAITING: u8 = 0;
    pub const COMPARED: u8 = 1;
    pub const NO_BEFORE: u8 = 2;
    pub const OVERLAPPED: u8 = 3;
}

/// Derived by the recorder's Casualties writer from WarRelations: one live lifetime of a war
/// relation between two country identities (canonical slot, raw id of its Countries row, that
/// row's link resolving to the slot), `a` the lesser, from the relation's start hour: the casualties
/// each side took in it and each side's unknown-attacker counter, as last read. Deleted the hour
/// WarRelations no longer lists the lifetime: an ended lifetime's counters are its row's last.
/// Each slot and id is at most [`Self::FIELD`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CasualtyLifetimeRow {
    pub slot_a: u32,
    pub id_a: u32,
    pub slot_b: u32,
    pub id_b: u32,
    pub start_hour: i32,
    pub taken_a: i64,
    pub taken_b: i64,
    pub unknown_a: i64,
    pub unknown_b: i64,
}

impl CasualtyLifetimeRow {
    /// The greatest slot or id a lifetime's key holds: four of them and the start hour fill 128 bits.
    pub const FIELD: u32 = (1 << 24) - 1;
}

/// Derived beside [`CasualtyLifetimeRow`]: each country identity's casualties over every lifetime
/// it was in, live or ended: `dealt` (what its enemies took from it), `taken`, and `unknown` (the
/// losses it took from no attacker), which is known only from a first read where all its
/// relations' unknown counters are zero, then while every relation spanning two reads agrees on
/// one increment, and never again in the run after a gap or a disagreement. `partial` is 1 from
/// the first read after a gap that no longer lists a lifetime the last read before it listed:
/// what it took in that lifetime's last hours is missing. A sum is not recorded once it leaves
/// i64, and from then for the run. A row lives from the identity's first lifetime and is never
/// deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CasualtyTotalRow {
    pub slot: u32,
    pub id: u32,
    pub dealt: Recorded<i64>,
    pub taken: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub partial: u8,
}

/// Derived by the recorder's Occupation writer from StateResistance, States and StateGarrisons:
/// each occupier identity's (canonical slot, raw id of its Countries row) states under active
/// occupation (a resistance row active with a positive occupied country), the wrapping sums of
/// their fixed5 resistance and compliance, and its held states: those it garrisons united with
/// those. `held_complete` is 1 when the garrisons are read in full, every garrison's controller
/// resolves, the occupier has a garrison and garrisons every state it occupies. While an active
/// state's controller resolves to no identity (or the state has no States row), every row's sums
/// and counts are not recorded. A row lives while its identity occupies or garrisons a state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupierTotalRow {
    pub slot: u32,
    pub id: u32,
    pub resistance_sum: Recorded<i64>,
    pub compliance_sum: Recorded<i64>,
    pub states: Recorded<i64>,
    pub held_states: Recorded<i64>,
    pub held_complete: u8,
}

/// Derived by the recorder's AirMissions writer from AirRegionalMissions and AirWings: each
/// owner's (canonical slot) registered wings in a strategic region, and per mission bit the
/// regional map's plane total: in registration order, each wing whose signed mask is positive adds
/// its planes to each of its bits, a negative total restarting from zero and the sum wrapping as
/// the game's dword does. A mission is not recorded where no registered wing of the region's
/// positive masks selects its bit. With the table complete, an (owner, region) without a row has
/// no registered wing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionCountRow {
    pub slot: u32,
    pub region: u32,
    pub wings: i64,
    pub mission_0: Recorded<i32>,
    pub mission_1: Recorded<i32>,
    pub mission_2: Recorded<i32>,
    pub mission_3: Recorded<i32>,
    pub mission_4: Recorded<i32>,
    pub mission_5: Recorded<i32>,
    pub mission_6: Recorded<i32>,
    pub mission_7: Recorded<i32>,
    pub mission_8: Recorded<i32>,
    pub mission_9: Recorded<i32>,
    pub mission_10: Recorded<i32>,
    pub mission_11: Recorded<i32>,
    pub mission_12: Recorded<i32>,
    pub mission_13: Recorded<i32>,
    pub mission_14: Recorded<i32>,
    pub mission_15: Recorded<i32>,
    pub mission_16: Recorded<i32>,
    pub mission_17: Recorded<i32>,
}

impl AirMissionCountRow {
    /// The mission bits the game's regional vectors count, named or not.
    pub const MISSIONS: usize = 18;

    /// Adds a registered wing to its region's totals (each starting at -1) as the regional map
    /// counts it: a signed non-positive mask admits no mission; each selected bit's total restarts
    /// from zero when negative and takes the wing's whole planes, zero included, wrapping.
    pub fn count(totals: &mut [i32; Self::MISSIONS], mask: u32, planes: i32) {
        if mask as i32 <= 0 {
            return;
        }
        for (bit, total) in totals.iter_mut().enumerate() {
            if mask & (1 << bit) != 0 {
                *total = (*total).max(0).wrapping_add(planes);
            }
        }
    }
}

/// Derived by the recorder's AirRegionWings writer from AirWings: the wings of every owner whose
/// mission region is the region. With the table complete, a region without a row has none.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionWingRow {
    pub region: u32,
    pub wings: i64,
}

/// Derived by the recorder's AirRegionDays writer from RaidAirScores, CountryLinks, TagHolders,
/// GroupSpans and Groups: each strategic region's air activity as the sides comparison composes it
/// at the hour (every shown side's friendly score), read in game day `day`; and that activity at the
/// last hour of each earlier day of the run (`activity_total`). A row is written only where the
/// activity changes, so the activity before a later day adds the row's once for each day between.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionDayRow {
    pub region: u32,
    pub day: i32,
    pub activity: i64,
    pub activity_total: i64,
}

/// Derived by the recorder's TerritoryRegions writer from States, Provinces and CountryLinks with
/// the strategic regions of the recording's game facts: what one holder has in one region, by two
/// rules, controlled (credited to the controller) and owned (to the owner). A holder is a canonical
/// slot, or [`Self::NO_COUNTRY`] for land no country holds; a region is a strategic region's id, or
/// [`Self::NO_REGION`] where no region, or more than one, claims a province. Provinces and their
/// victory points and land area count whole; states count as the exact sum of their parts there (a
/// state's provinces in the region over its provinces, as Provinces' `state` column groups them),
/// reduced, `states` over `states_denominator`. A row lives while the holder has a province or a
/// state's part in the region under either rule. A rule's cells are not recorded while some place's
/// country under it no link resolves; area is not recorded while the province areas are not held or
/// a province credited has none; states while their exact sum's denominator passes 63 bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TerritoryRegionTotalRow {
    pub holder: u32,
    pub region: u32,
    pub controlled_provinces: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub controlled_states_denominator: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub owned_area: Recorded<i64>,
    pub owned_states: Recorded<i64>,
    pub owned_states_denominator: Recorded<i64>,
}

impl TerritoryRegionTotalRow {
    /// The holder of land no country holds.
    pub const NO_COUNTRY: u32 = u32::MAX;
    /// The region of provinces no single strategic region claims.
    pub const NO_REGION: u32 = u32::MAX;

    /// The key of `holder`'s row of `region`.
    pub const fn key_of(holder: u32, region: u32) -> u128 {
        (holder as u128) << 32 | region as u128
    }
}

/// Derived by the recorder's ProducedValues writer from ProductionCountryTotals,
/// ProductionTypeTotals, ProductionCoverage, RocketProductionOutput and VariantDefinitions: what a
/// producer (the Production writer's country id) delivered since the observation began, lines and
/// Rocket output together, in units and in applied IC, with refits and without them. Its rows are
/// a producer's every type ([`Self::WHOLE`]), each archetype it delivered (a lexer token;
/// [`ProductionTypeTotalRow::NO_ARCHETYPE`] for designs whose type was not recorded), and what an
/// archetype it has no row of counts ([`Self::REST`]). A total is not recorded where any part of it
/// is not known, its `*_reason` saying why; an archetype's total is not known while designs whose
/// type was not recorded hold something ([`Self::TYPE_NOT_RECORDED`]). The one [`Self::CLOCK`] row
/// dates every row: `since`, the hour the totals count from (the later of the lines' and Rocket
/// output's observations), and `as_of`, production's latest read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProducedValueRow {
    pub country: u32,
    pub archetype: u32,
    pub units: Recorded<i64>,
    pub units_reason: u8,
    pub units_without_refits: Recorded<i64>,
    pub units_without_refits_reason: u8,
    pub ic: Recorded<i64>,
    pub ic_reason: u8,
    pub ic_without_refits: Recorded<i64>,
    pub ic_without_refits_reason: u8,
    pub since: Option<i32>,
    pub as_of: Option<i32>,
}

impl ProducedValueRow {
    /// A producer's row of every archetype together.
    pub const WHOLE: u32 = u32::MAX;
    /// A producer's row of what an archetype it has no row of counts.
    pub const REST: u32 = u32::MAX - 1;
    /// The country of the row dating every row.
    pub const CLOCK: u32 = u32::MAX;

    pub const KNOWN: u8 = 0;
    /// The producer's ProductionCountryTotals row is not live.
    pub const NOT_RECORDED: u8 = 1;
    /// A line's count left a delivery out, or a line went without its final count.
    pub const UNITS_INEXACT: u8 = 2;
    pub const SPENDING_UNAVAILABLE: u8 = 3;
    pub const LOST_OUTPUT: u8 = 4;
    pub const MISSED_PUBLICATION: u8 = 5;
    /// Designs whose type was not recorded hold something: they could be of this archetype.
    pub const TYPE_NOT_RECORDED: u8 = 6;
    pub const ROCKET_TABLE_UNAVAILABLE: u8 = 7;
    pub const ROCKET_INPUT_INEXACT: u8 = 8;
    pub const ROCKET_OWNER_UNREADABLE: u8 = 9;
    /// Past an exact 64-bit integer.
    pub const OUT_OF_RANGE: u8 = 10;

    /// The key of `country`'s row of `archetype`.
    pub const fn key_of(country: u32, archetype: u32) -> u128 {
        (country as u128) << 32 | archetype as u128
    }

    /// The key of the row dating every row.
    pub const fn clock() -> u128 {
        Self::key_of(Self::CLOCK, 0)
    }
}

/// Derived by the recorder's AirLossTotals writer from AirLossEvents, CountryLinks and TagHolders:
/// the run's aircraft losses through the hour, by the holder an event's country resolved to at the
/// read that supplied it (its canonical slot and raw id, as TagHolders names it; [`Self::NONE`] for
/// none), strategic region and native cause, each side split by whether the opposite country
/// resolved. Only reads both AirLossEvents and CountryLinks were available count: a read either was
/// not adds nothing. `late` is the most hours any event counted in the row was read after it
/// happened. A total past 64 bits is not recorded. Rows live while the run does.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirLossTotalRow {
    pub slot: u32,
    pub id: u32,
    pub region: u32,
    pub cause: u32,
    pub dealt_known: Recorded<i64>,
    pub dealt_unknown: Recorded<i64>,
    pub taken_known: Recorded<i64>,
    pub taken_unknown: Recorded<i64>,
    pub late: i32,
}

impl AirLossTotalRow {
    /// The slot and id of losses no holder was resolved for.
    pub const NONE: u32 = u32::MAX;

    pub const fn key_of(slot: u32, id: u32, region: u32, cause: u32) -> u128 {
        (slot as u128) << 96 | (id as u128) << 64 | (region as u128) << 32 | cause as u128
    }
}

/// Derived by the recorder's BombingTotals writer from BuildingBombingDamage, CountryLinks and
/// TagHolders: the run's building damage by aircraft (the air-mission and air-naval callers) through
/// the hour, by the holder each side resolved to at the read that supplied it ([`Self::NONE`] for
/// none) and caller, dealt by the attacker and taken by the victim. Only reads both
/// BuildingBombingDamage and CountryLinks were available count. `late` as [`AirLossTotalRow`]'s. A
/// total past 64 bits is not recorded. Rows live while the run does.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BombingTotalRow {
    pub slot: u32,
    pub id: u32,
    pub cause: u8,
    pub dealt: Recorded<i64>,
    pub taken: Recorded<i64>,
    pub late: i32,
}

impl BombingTotalRow {
    pub const NONE: u32 = u32::MAX;

    pub const fn key_of(slot: u32, id: u32, cause: u8) -> u128 {
        (slot as u128) << 40 | (id as u128) << 8 | cause as u128
    }
}

/// Derived by the recorder's GarrisonStrength writer from StateGarrisons, StateAssignedManpower and
/// CountryLinks with the content's garrison strength powers (the recording's facts): each
/// garrison's displayed strength, a signed fixed5 fraction (100000 is full), keyed as its
/// StateGarrisons row. Not recorded where an input of it is not known, `reason` saying which.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GarrisonStrengthRow {
    pub controller: u32,
    pub state: u32,
    pub strength: Recorded<i64>,
    pub reason: u8,
}

impl GarrisonStrengthRow {
    /// The strength is recorded.
    pub const KNOWN: u8 = 0;
    /// The garrison's controller or occupied country resolves to no link.
    pub const UNRESOLVED: u8 = 1;
    /// The state's assigned manpower is not read whole, or one of its rows resolves to no link.
    pub const ASSIGNED: u8 = 2;
    /// The recording lacks the garrison's equipment operands.
    pub const OPERANDS: u8 = 3;
    /// The equipment operands' capture failed; its reason is the StateGarrisons row's.
    pub const FAILED: u8 = 4;

    pub const fn key_of(controller: u32, state: u32) -> u128 {
        (controller as u128) << 32 | state as u128
    }
}

/// Derived by the recorder's AirOwnerDays writer from RaidAirScores: each owner's scores in each
/// strategic region (raw country id above region, as RaidAirScores keys them), a negative score as
/// zero and an entry gone as zero, read in game day `day`; and those at the last hour of each earlier
/// day of the run (`friendly_total`, `enemy_total`). A row is written only where its scores change,
/// so the totals before a later day add the row's scores once for each day between. A row lives
/// while the run does: its totals are what a span's days sum to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirOwnerDayRow {
    pub country: u32,
    pub region: u32,
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

/// Derived by the recorder's AirTheatreRegions writer from AirTheatreMembers, AirTheatreGroups and
/// AirWings: the wings of each air theatre whose mission region is the region. A theatre's regions
/// are those with a row; with the table complete, a region without one is outside the theatre.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTheatreRegionRow {
    pub theatre: u64,
    pub region: u32,
    pub wings: i64,
}

/// Derived by the recorder's AirRegionActivity writer from RaidAirScores: in each strategic region,
/// every owner's scores summed at the hour, a negative score as zero (`friendly`, `enemy`), read in
/// game day `day`; and those sums at the last hour of each earlier day of the run
/// (`friendly_total`, `enemy_total`). A row is written only where its sums change, so the totals
/// before a later day add the row's sums once for each day between. Rows live while the run does.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionActivityRow {
    pub region: u32,
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

row_codec!(TagHolderRow, 128, |row| (row.number as u128) << 96 | (row.epoch as u128) << 64 | (row.slot as u128) << 32 | row.id as u128, |key| { number: (key >> 96) as u32, epoch: (key >> 64) as u32, slot: (key >> 32) as u32, id: key as u32 }, {0 => tag: Bytes}, |_row| Ok(()));
row_codec!(GroupSpanRow, 104, |row| (row.kind as u128) << 96 | (row.group as u128) << 64 | (row.value as u128) << 32 | row.epoch as u128, |key| { kind: (key >> 96) as u8, group: (key >> 64) as u32, value: (key >> 32) as u32, epoch: key as u32 }, {0 => ordinal: U32}, |_row| Ok(()));
row_codec!(GroupRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => faction: U64}, |_row| Ok(()));
row_codec!(GroupNameRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => literal: Bytes, 1 => template_token: OptionalU32, 2 => spelling: Bytes}, |_row| Ok(()));
row_codec!(WarEventRow, 104, |row| (row.kind as u128) << 96 | (row.hour as u32 as u128) << 64 | (row.first as u128) << 32 | row.second as u128, |key| { kind: (key >> 96) as u8, hour: (key >> 64) as u32 as i32, first: (key >> 32) as u32, second: key as u32 }, {0 => read: U8}, |_row| Ok(()));
row_codec!(LookSpanRow, 96, |row| (row.number as u128) << 64 | (row.fact as u128) << 32 | row.epoch as u128, |key| { number: (key >> 64) as u32, fact: (key >> 32) as u32, epoch: key as u32 }, {0 => read: U8}, |_row| Ok(()));
row_codec!(LookFactRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => look: Bytes}, |_row| Ok(()));
row_codec!(BalanceDropRow, 72, |row| (row.kind as u128) << 64 | (row.country as u128) << 32 | row.hour as u32 as u128, |key| { kind: (key >> 64) as u8, country: (key >> 32) as u32, hour: key as u32 as i32 }, {0 => before: I64, 1 => after: I64}, |_row| Ok(()));
row_codec!(TagNameRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => tag: Bytes}, |_row| Ok(()));
row_codec!(TerritoryTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => owned_states: I64, 1 => controlled_states: I64, 2 => occupied_states: I64, 3 => owned_provinces: I64, 4 => controlled_provinces: I64, 5 => occupied_provinces: I64, 6 => owned_victory_points: I64, 7 => controlled_victory_points: I64, 8 => occupied_victory_points: I64, 9 => owned_area: I64, 10 => controlled_area: I64, 11 => occupied_area: I64}, |_row| Ok(()));
row_codec!(CountryStabilityRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => stability: I64, 1 => war_support: I64, 2 => max_command_power: I64}, |_row| Ok(()));
row_codec!(ArmyTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => owner_divisions: I64, 1 => owner_strength_raw: I64, 2 => host_divisions: I64, 3 => host_strength_raw: I64, 4 => owner_battalions: I64, 5 => host_battalions: I64, 6 => owner_manpower: I64, 7 => host_manpower: I64}, |_row| Ok(()));
row_codec!(ArmyMixRow, 72, |row| ((row.slot as u128) << 40) | ((row.attribution as u128) << 32) | row.token as u128, |key| { slot: (key >> 40) as u32, attribution: (key >> 32) as u8, token: key as u32 }, {0 => battalions: I64}, |_row| Ok(()));
row_codec!(AirForceTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => roster_planes: I64, 1 => roster_wings: I64, 2 => effective_planes: I64, 3 => effective_wings: I64}, |_row| Ok(()));
row_codec!(AirForcePartRow, 105, |row| ((row.slot as u128) << 73) | ((row.ownership as u128) << 65) | match row.variant { Some(variant) => (1 << 64) | variant as u128, None => 0 }, |key| { slot: (key >> 73) as u32, ownership: (key >> 65) as u8, variant: if (key >> 64) & 1 == 1 { Some(key as u64) } else { None } }, {0 => planes: I64}, |_row| Ok(()));
row_codec!(AirForceWingKindRow, 72, |row| ((row.slot as u128) << 40) | ((row.ownership as u128) << 32) | row.signature as u128, |key| { slot: (key >> 40) as u32, ownership: (key >> 32) as u8, signature: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(AirForceWingSignatureRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => variants: Bytes}, |_row| Ok(()));
row_codec!(NavyTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => roster_ships: I64, 1 => roster_strength_raw: I64, 2 => roster_max_strength_raw: I64, 3 => logical_ships: I64, 4 => logical_strength_raw: I64, 5 => logical_max_strength_raw: I64}, |_row| Ok(()));
row_codec!(NavyClassRow, 72, |row| ((row.slot as u128) << 40) | ((row.ownership as u128) << 32) | row.class as u128, |key| { slot: (key >> 40) as u32, ownership: (key >> 32) as u8, class: key as u32 }, {0 => ships: I64, 1 => strength_raw: I64}, |_row| Ok(()));
row_codec!(CountryDeployedManpowerRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => army: I32, 1 => navy: I32, 2 => navy_required: I32, 3 => air: I32, 4 => exile_air: I32}, |_row| Ok(()));
row_codec!(MilitaryAidRow, 72, |row| (row.sender as u128) << 40 | (row.host as u128) << 8 | row.channel as u128, |key| { sender: (key >> 40) as u32, host: (key >> 8) as u32, channel: key as u8 }, {0 => divisions: I64}, |row| if row.channel > MilitaryAidRow::TALLIES { Err(invalid("a military aid row's channel is past tallies")) } else { Ok(()) });
row_codec!(SupplyHealthRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => no_supply: I64, 1 => strained: I64, 2 => in_supply: I64}, |_row| Ok(()));
row_codec!(CountryExperienceDailyRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army_change: I64, 1 => navy_change: I64, 2 => air_change: I64, 3 => army_gain: I64, 4 => navy_gain: I64, 5 => air_gain: I64}, |_row| Ok(()));
row_codec!(CountryGameScoreRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army: I64, 1 => navy: I64, 2 => air: I64, 3 => industry: I64, 4 => land: I64, 5 => total: I64, 6 => army_reason: U8, 7 => navy_reason: U8, 8 => air_reason: U8, 9 => industry_reason: U8, 10 => land_reason: U8, 11 => total_reason: U8}, |_row| Ok(()));
row_codec!(StabilityPartRow, 72, |row| (row.country as u128) << 40 | (row.measure as u128) << 32 | row.handle as u128, |key| { country: (key >> 40) as u32, measure: (key >> 32) as u8, handle: key as u32 }, {0 => value: I64}, |row| if row.measure > StabilityPartRow::WAR_SUPPORT { Err(invalid("a stability part's measure is past war support")) } else { Ok(()) });
row_codec!(StabilityPartKeyRow, 32, |row| row.handle as u128, |key| { handle: key as u32 }, {0 => kind: U8, 1 => identity: Bytes, 2 => identity_id: U32, 3 => secondary: I64, 4 => script: Bytes, 5 => saved: Bytes, 6 => template_present: U8, 7 => template_tag: Bytes, 8 => template_file: Bytes, 9 => template_line: I32, 10 => template_generated: U8}, |row| if row.kind > StabilityPartKeyRow::REFUSED_LABEL { Err(invalid("a stability part's kind is past a refused label")) } else { Ok(()) });
row_codec!(ThreatEventRow, 104, |row| (row.kind as u128) << 96 | (row.slot as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { kind: (key >> 96) as u8, slot: (key >> 64) as u32, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => country: U32, 1 => second_country: U32, 2 => date: I32, 3 => reason: Bytes, 4 => final_raw: I64, 5 => place: U32}, |_row| Ok(()));
row_codec!(EquipmentLossTotalRow, 105, |row| (row.slot as u128) << 73 | (row.country as u128) << 41 | (row.source as u128) << 33 | match row.variant { Some(variant) => 1 << 32 | variant as u128, None => 0 }, |key| { slot: (key >> 73) as u32, country: (key >> 41) as u32, source: (key >> 33) as u8, variant: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => total: I64, 1 => incomplete_from: OptionalI32, 2 => freed: I64}, |_row| Ok(()));
row_codec!(CombatLogDroppedRow, 120, |row| (row.country as u128) << 88 | (row.group as u128) << 56 | (row.kind as u128) << 48 | (row.month as u128) << 32 | row.variant as u128, |key| { country: (key >> 88) as u32, group: (key >> 56) as u32, kind: (key >> 48) as u8, month: (key >> 32) as u16, variant: key as u32 }, {0 => date: I32, 1 => amount: I64}, |_row| Ok(()));
row_codec!(ShipLossRow, 128, |row| (row.slot as u128) << 104 | (row.country as u128) << 80 | (row.hour as u32 as u128) << 48 | row.signature as u128, |key| { slot: (key >> 104) as u32 & ShipLossRow::FIELD, country: (key >> 80) as u32 & ShipLossRow::FIELD, hour: (key >> 48) as u32 as i32, signature: key as u64 & ShipLossRow::SIGNATURE }, {0 => sink_hour: I32, 1 => variant: U64, 2 => count: U32, 3 => check: U64}, |row| if row.slot > ShipLossRow::FIELD || row.country > ShipLossRow::FIELD || row.signature > ShipLossRow::SIGNATURE { Err(invalid("a ship loss's slot or country passes 24 bits, or its signature 48")) } else { Ok(()) });
row_codec!(EquipmentLossCoverageRow, 64, |row| (row.slot as u128) << 32 | row.country as u128, |key| { slot: (key >> 32) as u32, country: key as u32 }, {0 => land: U8, 1 => ungrouped: U32, 2 => resolved: U8, 3 => convoy_drops: U32, 4 => drain: Bytes, 5 => unread: U8, 6 => counted_from: OptionalI32, 7 => withheld_from: OptionalI32, 8 => incomplete_from: OptionalI32, 9 => convoys_from: OptionalI32, 10 => lend_from: OptionalI32, 11 => exchanges_ended: U32, 12 => ended_from: OptionalI32, 13 => unresolved: U32, 14 => unresolved_all: U32}, |_row| Ok(()));
row_codec!(StockByVariantRow, 96, |row| (row.slot as u128) << 64 | row.variant as u128, |key| { slot: (key >> 64) as u32, variant: key as u64 }, {0 => amount: I64, 1 => positions: I64}, |_row| Ok(()));
row_codec!(DivisionEquipmentCountRow, 40, |row| (row.slot as u128) << 8 | row.side as u128, |key| { slot: (key >> 8) as u32, side: key as u8 }, {0 => divisions: I64, 1 => short: I64, 2 => unknown: I64, 3 => unpooled: I64, 4 => unclassified: I64, 5 => unresolved: I64, 6 => pool_hour: OptionalI32, 7 => stock_hour: OptionalI32}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentTypeRow, 73, |row| (row.slot as u128) << 41 | (row.side as u128) << 33 | match row.archetype { Some(token) => 1 << 32 | token as u128, None => 0 }, |key| { slot: (key >> 41) as u32, side: (key >> 33) as u8, archetype: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => held: I64, 1 => needed: I64, 2 => filled: I64, 3 => missing: I64, 4 => short: I64, 5 => stock: I64}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentDesignRow, 106, |row| (row.slot as u128) << 74 | (row.side as u128) << 66 | (match row.archetype { Some(token) => 1 << 32 | token as u128, None => 0 }) << 33 | match row.variant { Some(id) => 1 << 32 | id as u128, None => 0 }, |key| { slot: (key >> 74) as u32, side: (key >> 66) as u8, archetype: if (key >> 65) & 1 == 1 { Some((key >> 33) as u32) } else { None }, variant: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => held: I64}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentTemplateRow, 104, |row| (row.slot as u128) << 72 | (row.side as u128) << 64 | row.template as u128, |key| { slot: (key >> 72) as u32, side: (key >> 64) as u8, template: key as u64 }, {0 => divisions: I64, 1 => unknown: I64, 2 => types_short: I64, 3 => manpower: I64, 4 => manpower_lacking: I64, 5 => required: I64, 6 => hp: I64, 7 => hp_max: I64, 8 => hp_max_lacking: I64, 9 => needed: I64, 10 => held: I64, 11 => need_missing: Bytes}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(FuelDayRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => read_hour: I32, 1 => gained: I64, 2 => burned: I64, 3 => received_army: I64, 4 => received_air: I64, 5 => received_navy: I64, 6 => other: I64, 7 => gain_base: I64, 8 => gain_states: I64, 9 => gain_oil: I64, 10 => gain_negative_cost: I64, 11 => own_gain_quantity: I64, 12 => incoming: I64, 13 => outgoing: I64, 14 => flat: I64, 15 => left_out: U8, 16 => awards_reason: Bytes, 17 => continuity_start_hour: OptionalI32, 18 => overlapped: OptionalI32}, |row| if row.left_out > FuelDayRow::OWNERSHIP_NOT_RECORDED { Err(invalid("a fuel day's left-out code is past ownership not recorded")) } else { Ok(()) });
row_codec!(FuelDemandTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => native_army: I64, 1 => native_air: I64, 2 => native_navy: I64, 3 => known_army: I64, 4 => known_air: I64, 5 => known_navy: I64, 6 => positive_army: I64, 7 => positive_air: I64, 8 => positive_navy: I64, 9 => army_reason: Bytes, 10 => air_reason: Bytes, 11 => navy_reason: Bytes}, |_row| Ok(()));
row_codec!(TradeFlowRow, 128, |row| key_field(row.seller_slot) << 104 | key_field(row.seller) << 80 | key_field(row.buyer_slot) << 56 | key_field(row.buyer) << 32 | row.resource as u128, |key| { seller_slot: field_of_key(key, 104), seller: field_of_key(key, 80), buyer_slot: field_of_key(key, 56), buyer: field_of_key(key, 32), resource: key as u32 }, {0 => delivered: I64, 1 => factories: I64, 2 => convoys_assigned: I64, 3 => convoys_required: I64, 4 => deals: I64}, |row| if [row.seller_slot, row.seller, row.buyer_slot, row.buyer].into_iter().all(fits_key_field) { Ok(()) } else { Err(invalid("a trade flow's country is past its 24-bit key field")) });
row_codec!(ConstructionProjectRow, 64, |row| row.line as u128, |key| { line: key as u64 }, {0 => kind: U8, 1 => output_alone: Bool, 2 => country: U32, 3 => building_type: OptionalU32, 4 => target_state: OptionalU32, 5 => to_state: OptionalU32, 6 => province: OptionalU32, 7 => route_provinces: OptionalU32, 8 => base_level: OptionalI32, 9 => target_level: OptionalI32, 10 => cost: OptionalI64, 11 => created_hour: OptionalI32, 12 => first_read: I32, 13 => census_read: OptionalI32, 14 => queue_ordinal: OptionalU32, 15 => earlier_unknown: Bool, 16 => factories: I32, 17 => remaining: I32, 18 => repair_levels: I32, 19 => built: I64, 20 => built_rises: Opaque, 21 => repaired: I64, 22 => repaired_rises: Opaque, 23 => added_new: I64, 24 => added_repair: I64, 25 => possible: I64, 26 => unaccounted_new: I64, 27 => unaccounted_repair: I64, 28 => end: U8, 29 => end_reason: U8, 30 => end_hour: OptionalI32, 31 => end_known_at: OptionalI32, 32 => construction_raw: I64, 33 => repair_raw: I64, 34 => output_since: OptionalI32, 35 => observed_since_creation: Bool, 36 => output_reason: U8, 37 => first_repair_tick: OptionalI32}, |row| if row.kind > ConstructionProjectRow::RAILWAY || row.end > ConstructionProjectRow::UNKNOWN_END { Err(invalid("a construction project's kind or ending is past its codes")) } else { Ok(()) });
row_codec!(ConstructionLineEndRow, 96, |row| ConstructionLineEndRow::key_of(row.hour, row.line), |key| { hour: (key >> 64) as u32 as i32, line: key as u64 }, {0 => kind: U8, 1 => output_alone: Bool, 2 => country: U32, 3 => building_type: OptionalU32, 4 => target_state: OptionalU32, 5 => to_state: OptionalU32, 6 => first_read: I32, 7 => new_work: Bool, 8 => repair_work: Bool, 9 => before_observation: Bool, 10 => had_output: Bool, 11 => output_reason: U8}, |row| if row.kind > ConstructionProjectRow::RAILWAY { Err(invalid("a construction line end's kind is past its codes")) } else { Ok(()) });
row_codec!(ConstructionSpendRow, 114, |row| key_field(row.slot) << 90 | key_field(row.country) << 66 | (row.repair as u128) << 65 | (row.railway as u128) << 64 | (row.state as u128) << 32 | (if row.railway { row.to_state } else { row.building_type }) as u128, |key| { slot: field_of_key(key, 90), country: field_of_key(key, 66), repair: (key >> 65) & 1 == 1, railway: (key >> 64) & 1 == 1, building_type: if (key >> 64) & 1 == 1 { TagHolderRow::NONE } else { key as u32 }, state: (key >> 32) as u32, to_state: if (key >> 64) & 1 == 1 { key as u32 } else { TagHolderRow::NONE } }, {0 => total: I64, 1 => unknown: I64}, |row| if [row.slot, row.country].into_iter().all(fits_key_field) && (row.railway || row.to_state == TagHolderRow::NONE) && (!row.railway || row.building_type == TagHolderRow::NONE) { Ok(()) } else { Err(invalid("a construction spend key holds a slot or country past 24 bits, or a line both a building type and a railway's last state")) });
row_codec!(ProductionTotalRow, 104, |row| (row.country as u128) << 72 | (row.variant as u128) << 8 | row.purpose as u128, |key| { country: (key >> 72) as u32, variant: (key >> 8) as u64, purpose: key as u8 }, {0 => units: I64, 1 => inexact_rows: U32, 2 => spent: I64, 3 => spent_reason: U8, 4 => unproven: I64, 5 => equipment_token: Bytes, 6 => archetype: U32}, |row| if OutputPurpose::from_raw(row.purpose).is_none() { Err(invalid("a production total's purpose is invalid")) } else { Ok(()) });
row_codec!(ProductionCountryTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => units: I64, 1 => units_inexact: U32, 2 => spent: I64, 3 => units_without_refits: I64, 4 => units_without_refits_inexact: U32, 5 => spent_without_refits: I64}, |_row| Ok(()));
row_codec!(ProductionTypeTotalRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| { country: (key >> 32) as u32, archetype: key as u32 }, {0 => units: I64, 1 => units_inexact: U32, 2 => spent: I64, 3 => units_without_refits: I64, 4 => units_without_refits_inexact: U32, 5 => spent_without_refits: I64, 6 => archetype_spelling: Bytes}, |_row| Ok(()));
row_codec!(ProductionCoverageRow, 40, |row| (row.country as u128) << 8 | row.source as u128, |key| { country: (key >> 8) as u32, source: key as u8 }, {0 => observation_since: OptionalI32, 1 => units_unknown_from: OptionalI32, 2 => spending_unknown_from: OptionalI32, 3 => spending_reason: U8}, |row| if row.source > ProductionCoverageRow::ROCKET { Err(invalid("a production coverage row's source is past Rocket")) } else { Ok(()) });
row_codec!(DockyardRepairRateRow, 104, |row| (row.country as u128) << 72 | (row.part as u128) << 64 | row.design as u128, |key| { country: (key >> 72) as u32, part: (key >> 64) as u8, design: key as u64 }, {0 => rate: I64, 1 => used_dockyards: I32, 2 => output: I64, 3 => split_known: Bool}, |row| if row.part > DockyardRepairRateRow::DESIGN { Err(invalid("a dockyard repair rate's part is past a design")) } else { Ok(()) });
row_codec!(LandBattleRow, 64, |row| row.combat as u128, |key| { combat: key as u64 }, {0 => start_hour: I32, 1 => start_exact: Bool, 2 => counter_base: I32, 3 => peak_total: U32, 4 => peak_total_hour: I32, 5 => peak_attackers: U32, 6 => peak_attackers_hour: I32, 7 => peak_defenders: U32, 8 => peak_defenders_hour: I32}, |_row| Ok(()));
row_codec!(LandBattleEndRow, 97, |row| (row.combat as u128) << 33 | (row.result_only as u128) << 32 | row.start_hour as u32 as u128, |key| { combat: (key >> 33) as u64, result_only: (key >> 32) & 1 == 1, start_hour: key as u32 as i32 }, {0 => kind: U8, 1 => end_hour: I32, 2 => start_exact: Bool, 3 => counter_base: I32, 4 => province: U32, 5 => last_read: OptionalI32, 6 => last_duration: I32, 7 => peak_total: U32, 8 => peak_total_hour: I32, 9 => peak_attackers: U32, 10 => peak_attackers_hour: I32, 11 => peak_defenders: U32, 12 => peak_defenders_hour: I32, 13 => attacker_manpower: I32, 14 => defender_manpower: I32, 15 => attackers: U32List, 16 => defenders: U32List, 17 => countries_unknown: U8, 18 => result: OptionalU64, 19 => result_read: Bool, 20 => winners: U8, 21 => result_duration: I32, 22 => attacker_terminal: I32, 23 => defender_terminal: I32, 24 => result_attackers: U32List, 25 => result_defenders: U32List, 26 => result_unknown: U8, 27 => attacker_terminal_equipment: Opaque, 28 => defender_terminal_equipment: Opaque}, |row| if row.kind > LandBattleEndRow::RESULT_ONLY { Err(invalid("land battle end kind invalid")) } else { Ok(()) });
row_codec!(NavalBattleRow, 64, |row| row.combat as u128, |key| { combat: key as u64 }, {0 => first_hour: I32, 1 => peak_ships: U32, 2 => peak_hour: I32, 3 => attacker_sunk: U32, 4 => defender_sunk: U32, 5 => attacker_convoys_sunk: U32, 6 => defender_convoys_sunk: U32}, |_row| Ok(()));
row_codec!(NavalResultObservationRow, 64, |row| row.result as u128, |key| { result: key as u64 }, {0 => refusal: U8, 1 => combat: U64, 2 => province: U32, 3 => first_hour: I32, 4 => last_hour: I32, 5 => peak_ships: U32, 6 => peak_hour: I32, 7 => attacker_ships: U32, 8 => attacker_convoys: U32, 9 => attacker_sunk: U32, 10 => attacker_convoys_sunk: U32, 11 => defender_ships: U32, 12 => defender_convoys: U32, 13 => defender_sunk: U32, 14 => defender_convoys_sunk: U32}, |row| if row.refusal > NavalResultObservationRow::NOT_OBSERVED { Err(invalid("naval result observation refusal invalid")) } else { Ok(()) });
row_codec!(NavalLossTotalRow, 80, |row| (row.slot as u128) << 48 | (row.country as u128) << 16 | (row.attribution as u128) << 8 | row.kind as u128, |key| { slot: (key >> 48) as u32, country: (key >> 16) as u32, attribution: (key >> 8) as u8, kind: key as u8 }, {0 => total: I64, 1 => unknown: U32, 2 => counted_from: I32, 3 => known: Bool, 4 => lost_from: OptionalI32}, |row| if row.attribution > NavalLossTotalRow::SUNK_BY || row.kind > NavalLossTotalRow::CONVOYS { Err(invalid("naval loss total attribution or kind invalid")) } else { Ok(()) });
row_codec!(DivisionEventRow, 104, |row| (row.kind as u128) << 96 | (row.hour as u32 as u128) << 64 | row.division as u128, |key| { kind: (key >> 96) as u8, hour: (key >> 64) as u32 as i32, division: key as u64 }, {0 => disposition: OptionalU32, 1 => cause: OptionalU32, 2 => owner: U32, 3 => slot: U32, 4 => native_hour: I32, 5 => exact: Bool, 6 => after: OptionalI32, 7 => province: U32, 8 => place_hour: I32, 9 => template: U64, 10 => template_hour: I32, 11 => name: Opaque, 12 => naming: Opaque, 13 => name_hour: I32, 14 => icon_unit: U32, 15 => icon_custom: I32, 16 => icon_name: Bytes, 17 => icon_hour: I32, 18 => killer_country: OptionalU32, 19 => killer_slot: OptionalU32, 20 => killer_division: OptionalU64, 21 => template_id: U32, 22 => matched_deaths: OptionalU32, 23 => template_literal: Bytes, 24 => template_key: Bytes, 25 => template_country: I32}, |row| if row.kind > DivisionEventRow::VANISHED { Err(invalid("a division event's kind is past vanished")) } else { Ok(()) });
row_codec!(LeaderBattleRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => entry: U8, 1 => kind: U8, 2 => combat: U64, 3 => sides: U8, 4 => start_hour: I32, 5 => start_unknown: Bool, 6 => winners: U8, 7 => result_hour: I32, 8 => outcome: U8}, |row| if row.entry > LeaderBattleRow::RESULT || !matches!(row.kind, LeaderBattleRow::LAND | LeaderBattleRow::NAVAL) { Err(invalid("a leader battle's entry or kind is unknown")) } else { Ok(()) });
row_codec!(LeaderTraitGainRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.token as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, token: key as u32 }, {0 => class: U8, 1 => spelling: Bytes}, |row| if row.class > LeaderTraitGainRow::FIRST_OBSERVED { Err(invalid("a trait gain's class is past first observed")) } else { Ok(()) });
row_codec!(PowerSpendTotalRow, 72, |row| (row.holder as u128) << 40 | (row.currency as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 40) as u32, currency: (key >> 32) as u8, ordinal: key as u32 }, {0 => known: I64, 1 => payments: I64, 2 => entries: I64, 3 => unknown: I64, 4 => uncovered: I64, 5 => latest_hour: OptionalI32, 6 => latest_sequence: OptionalU64, 7 => latest_ordinal: OptionalU32, 8 => latest_read: OptionalI32, 9 => days: I64, 10 => day_payments: I64, 11 => day_entries: I64, 12 => day_charge: I64, 13 => day_unknown: I64, 14 => days_uncovered: I64, 15 => realized_gained: I64, 16 => realized_lost: I64, 17 => realized_unknown: I64, 18 => first_day: OptionalI32, 19 => last_day: OptionalI32, 20 => last_lag: OptionalI32, 21 => last_hour: OptionalI32, 22 => last_sequence: OptionalU64, 23 => last_ordinal: OptionalU32, 24 => last_covered: Bool, 25 => balances: I64, 26 => credits: I64, 27 => gained: I64, 28 => capped: I64, 29 => gain_days: I64, 30 => gain_unknown_days: I64, 31 => unmatched: I64}, |row| if row.currency > PowerCurrency::Command as u8 { Err(invalid("a power total's currency is past command")) } else { Ok(()) });
row_codec!(PowerItemRow, 64, |row| (row.holder as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 32) as u32, ordinal: key as u32 }, {0 => kind: U8, 1 => currency: U8, 2 => cause: U16, 3 => component: OptionalU32, 4 => charged: Bool, 5 => site: U16, 6 => action: U8, 7 => item_token: OptionalU32, 8 => item_key: Bytes, 9 => slot_key: Bytes, 10 => character: OptionalU32, 11 => object_kind: U8, 12 => object_reference: OptionalU64, 13 => target_country: OptionalU32, 14 => target_state: OptionalU32, 15 => target_region: OptionalU32, 16 => origin_kind: U8, 17 => origin_token: OptionalU32, 18 => origin_key: Bytes, 19 => first_hour: I32, 20 => first_read: I32, 21 => first_country: U32, 22 => first_epoch: U64, 23 => first_sequence: U64}, |row| if row.kind > PowerItemRow::ABILITY_UPKEEP { Err(invalid("a power item's kind is past ability upkeep")) } else { Ok(()) });
row_codec!(PowerDayPartRow, 72, |row| (row.holder as u128) << 40 | (row.currency as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 40) as u32, currency: (key >> 32) as u8, ordinal: key as u32 }, {0 => day: I32, 1 => spent: I64, 2 => payments: I64, 3 => covered: Bool, 4 => sequence: OptionalU64, 5 => cost_ordinal: OptionalU32, 6 => read: I32}, |row| if row.day.rem_euclid(24) != 0 { Err(invalid("a power day part's day is not a midnight")) } else { Ok(()) });
row_codec!(PowerHoldRow, 96, |row| (row.holder as u128) << 64 | row.hold as u128, |key| { holder: (key >> 64) as u32, hold: key as u64 }, {0 => amount: I64, 1 => check: U64, 2 => cause: U16, 3 => object_kind: U8, 4 => object_reference: OptionalU64, 5 => ability: Bool, 6 => item_token: OptionalU32, 7 => item_key: Bytes, 8 => slot_key: Bytes, 9 => character: OptionalU32, 10 => target_country: OptionalU32, 11 => target_state: OptionalU32, 12 => target_region: OptionalU32, 13 => capture_country: U32, 14 => capture_epoch: U64}, |_row| Ok(()));
row_codec!(PowerSpendCoverageRow, 32, |row| row.holder as u128, |key| { holder: key as u32 }, {0 => political: Bool, 1 => command: Bool, 2 => reason: U8, 3 => political_since: OptionalI32, 4 => command_since: OptionalI32, 5 => daily_mismatch: U8, 6 => input_reason: Bytes}, |row| if usize::from(row.reason) >= PowerSpendCoverageRow::REASONS.len() { Err(invalid("a power spend coverage reason is past the reasons")) } else { Ok(()) });
row_codec!(ResearchAssignmentRow, 112, |row| (row.holder_slot as u128) << 80 | (row.holder_id as u128) << 48 | (row.index as u128) << 16 | row.lane as u128, |key| { holder_slot: (key >> 80) as u32, holder_id: (key >> 48) as u32, index: (key >> 16) as u32, lane: key as u16 }, {0 => technology_handle: U32, 1 => database_index: I32, 2 => token: Bytes, 3 => level: I32, 4 => continuity_start_hour: I32, 5 => first_observed_hour: I32, 6 => started_after_hour: OptionalI32, 7 => last_observed_hour: OptionalI32, 8 => ended_by_hour: OptionalI32, 9 => outcome: U8, 10 => outcome_observed_hour: OptionalI32, 11 => awaiting: U8}, |row| if row.outcome > ResearchAssignmentRow::UNKNOWN { Err(invalid("a research assignment's outcome is past unknown")) } else { Ok(()) });
row_codec!(ResearchCoverageRow, 96, |row| (row.holder_slot as u128) << 64 | (row.holder_id as u128) << 32 | row.member as u128, |key| { holder_slot: (key >> 64) as u32, holder_id: (key >> 32) as u32, member: key as u32 }, {0 => since: I32}, |_row| Ok(()));
row_codec!(FactionAccountOwnerRow, 32, |row| row.account as u128, |key| { account: key as u32 }, {0 => faction: U64, 1 => slot: U32, 2 => id: U32, 3 => member: U8}, |_row| Ok(()));
row_codec!(PeaceOutcomeRow, 64, |row| (row.after_hour as u32 as u128) << 32 | row.ordinal as u128, |key| { after_hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => observation: U8, 1 => conference: OptionalU64, 2 => before_hour: I32, 3 => change: U8, 4 => group: U32, 5 => state: U32, 6 => from_slot: U32, 7 => from_id: U32, 8 => to_slot: U32, 9 => to_id: U32, 10 => victory_points: OptionalI64}, |row| if row.observation > PeaceOutcomeRow::UNKNOWN || row.change > PeaceOutcomeRow::RELEASED { Err(invalid("a peace outcome's observation or change is past the last")) } else { Ok(()) });
row_codec!(PeaceConferenceSpanRow, 96, |row| (row.conference as u128) << 32 | row.hour as u32 as u128, |key| { conference: (key >> 32) as u64, hour: key as u32 as i32 }, {0 => first_observed_hour: I32, 1 => last_observed_hour: I32, 2 => current_turn: I32, 3 => finalized: U8, 4 => first_finalized_hour: OptionalI32, 5 => first_absent_hour: OptionalI32, 6 => comparison: U8, 7 => before_hour: OptionalI32, 8 => compared_hour: OptionalI32}, |row| if row.comparison > PeaceConferenceSpanRow::OVERLAPPED { Err(invalid("a peace conference's comparison is past overlapped")) } else { Ok(()) });
row_codec!(CasualtyLifetimeRow, 128, |row| (row.slot_a as u128) << 104 | (row.id_a as u128) << 80 | (row.slot_b as u128) << 56 | (row.id_b as u128) << 32 | row.start_hour as u32 as u128, |key| { slot_a: (key >> 104) as u32 & CasualtyLifetimeRow::FIELD, id_a: (key >> 80) as u32 & CasualtyLifetimeRow::FIELD, slot_b: (key >> 56) as u32 & CasualtyLifetimeRow::FIELD, id_b: (key >> 32) as u32 & CasualtyLifetimeRow::FIELD, start_hour: key as u32 as i32 }, {0 => taken_a: I64, 1 => taken_b: I64, 2 => unknown_a: I64, 3 => unknown_b: I64}, |row| if [row.slot_a, row.id_a, row.slot_b, row.id_b].iter().any(|&field| field > CasualtyLifetimeRow::FIELD) { Err(invalid("a casualty lifetime's slot or id passes 24 bits")) } else { Ok(()) });
row_codec!(CasualtyTotalRow, 64, |row| (row.slot as u128) << 32 | row.id as u128, |key| { slot: (key >> 32) as u32, id: key as u32 }, {0 => dealt: I64, 1 => taken: I64, 2 => unknown: I64, 3 => partial: U8}, |_row| Ok(()));
row_codec!(OccupierTotalRow, 64, |row| (row.slot as u128) << 32 | row.id as u128, |key| { slot: (key >> 32) as u32, id: key as u32 }, {0 => resistance_sum: I64, 1 => compliance_sum: I64, 2 => states: I64, 3 => held_states: I64, 4 => held_complete: U8}, |_row| Ok(()));
row_codec!(AirMissionCountRow, 64, |row| (row.slot as u128) << 32 | row.region as u128, |key| { slot: (key >> 32) as u32, region: key as u32 }, {0 => wings: I64, 1 => mission_0: I32, 2 => mission_1: I32, 3 => mission_2: I32, 4 => mission_3: I32, 5 => mission_4: I32, 6 => mission_5: I32, 7 => mission_6: I32, 8 => mission_7: I32, 9 => mission_8: I32, 10 => mission_9: I32, 11 => mission_10: I32, 12 => mission_11: I32, 13 => mission_12: I32, 14 => mission_13: I32, 15 => mission_14: I32, 16 => mission_15: I32, 17 => mission_16: I32, 18 => mission_17: I32}, |_row| Ok(()));
row_codec!(AirRegionWingRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(FeedEventRow, 128, |row| (row.generation as u128) << 64 | row.sequence as u128, |key| { generation: (key >> 64) as u64, sequence: key as u64 }, {0 => hour: I32, 1 => read_hour: I32, 2 => kind: U8, 3 => actor_reference: U32, 4 => actor_id: U32, 5 => actor_instance: U32, 6 => actor_slot: U32, 7 => target_reference: U32, 8 => target_id: U32, 9 => target_instance: U32, 10 => target_slot: U32, 11 => route: U8, 12 => within: U8, 13 => conference: OptionalU64, 14 => state: OptionalU32, 15 => ally_reference: OptionalU32, 16 => ally_id: OptionalU32, 17 => ally_instance: OptionalU32, 18 => ally_slot: OptionalU32, 19 => reason: OptionalU32, 20 => flags: U8, 21 => token: Bytes, 22 => native_event_kind: OptionalU32, 23 => native_event_id: OptionalU32, 24 => instance: OptionalU32, 25 => option: OptionalI32, 26 => country_target: OptionalU32, 27 => numeric_target: OptionalI32, 28 => lifecycle: OptionalI32, 29 => days: OptionalI32, 30 => scopes: Opaque, 31 => opener: OptionalU64, 32 => opener_hour: OptionalI32, 33 => dated: OptionalI32, 34 => political_power: OptionalI64, 35 => command_power: OptionalI64, 36 => coverage: OptionalU32, 37 => outstanding: OptionalU64, 38 => refusals: OptionalU32, 39 => failure: Bytes, 40 => backlog: Bool, 41 => incomplete: OptionalU32}, |_row| Ok(()));
row_codec!(FeedOpenEventRow, 128, |row| (row.kind as u128) << 96 | (row.id as u128) << 64 | (row.instance as u128) << 32 | row.receiver as u128, |key| { kind: (key >> 96) as u32, id: (key >> 64) as u32, instance: (key >> 32) as u32, receiver: key as u32 }, {0 => generation: U64, 1 => sequence: U64, 2 => read_hour: I32}, |_row| Ok(()));
row_codec!(FeedOpenDecisionRow, 128, |row| (row.generation as u128) << 64 | row.sequence as u128, |key| { generation: (key >> 64) as u64, sequence: key as u64 }, {0 => country: U32, 1 => token: Bytes, 2 => country_target: OptionalU32, 3 => numeric_target: OptionalI32, 4 => read_hour: I32}, |_row| Ok(()));
row_codec!(FeedGuaranteeRow, 96, |row| (row.first as u128) << 64 | (row.second as u128) << 32 | row.start as u32 as u128, |key| { first: (key >> 64) as u32, second: (key >> 32) as u32, start: key as u32 as i32 }, {0 => hour: OptionalI32, 1 => rows: U32}, |_row| Ok(()));
row_codec!(NavalRegionControlRow, 64, |row| (row.viewer as u128) << 32 | row.region as u128, |key| { viewer: (key >> 32) as u32, region: key as u32 }, {0 => friendly: I64, 1 => enemy: I64, 2 => ratio_friendly: I64, 3 => ratio_enemy: I64, 4 => own: I64, 5 => control_refusal: Bytes, 6 => share_refusal: Bytes}, |_row| Ok(()));
row_codec!(CountryNukeTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => total: I64, 1 => ready: I64}, |_row| Ok(()));
row_codec!(RaidEndRow, 96, |row| (row.hour as u32 as u128) << 64 | (row.id as u128) << 32 | row.reference_type as u128, |key| { hour: (key >> 64) as u32 as i32, id: (key >> 32) as u32, reference_type: key as u32 }, {0 => kind: U8, 1 => previous_read: I32, 2 => first_hour: OptionalI32, 3 => first_exact: Bool, 4 => launched_hour: OptionalI32, 5 => raider: U32, 6 => phase: I32, 7 => outcome: I32, 8 => travelled: OptionalI64, 9 => preparation_hours: I32, 10 => province: U32, 11 => state: U32, 12 => leader_reference: U64, 13 => leader_province: U32, 14 => building_type: U32, 15 => building_location: U32, 16 => building_flags: U32, 17 => building_country: I32, 18 => leader_country: OptionalI32, 19 => raid_type: U32, 20 => target_state: OptionalU32, 21 => victim: OptionalU32, 22 => victim_slot: U32, 23 => victim_unknown: U8}, |row| if row.kind > RaidEndRow::DELETED || row.victim_unknown > RaidEndRow::VICTIM_UNLINKED { Err(invalid("a raid end's kind or victim reason is past the last")) } else { Ok(()) });
row_codec!(AirMissionWingRow, 128, |row| (row.slot as u128) << 96 | (row.region as u128) << 64 | row.wing as u128, |key| { slot: (key >> 96) as u32, region: (key >> 64) as u32, wing: key as u64 }, {0 => ordinal: U32}, |_row| Ok(()));
row_codec!(LeaderBattleTotalRow, 64, |row| row.leader as u128, |key| { leader: key as u64 }, {0 => fought: U32, 1 => won: U32, 2 => lost: U32}, |_row| Ok(()));
row_codec!(LeaderTraitDateRow, 96, |row| (row.leader as u128) << 32 | row.token as u128, |key| { leader: (key >> 32) as u64, token: key as u32 }, {0 => first_hour: I32, 1 => first_class: U8, 2 => gained_hour: OptionalI32, 3 => spelling: Bytes}, |row| if row.first_class > LeaderTraitGainRow::FIRST_OBSERVED { Err(invalid("a trait date's class is past first observed")) } else { Ok(()) });
row_codec!(LeaderCommandSpanRow, 72, |row| (row.source as u128) << 64 | row.formation as u128, |key| { source: (key >> 64) as u8, formation: key as u64 }, {0 => leader: U64, 1 => first_hour: I32, 2 => kind: U8, 3 => name: Opaque}, |row| if row.source > LeaderCommandRow::FLEET || row.kind > LeaderCommandRow::NAVY { Err(invalid("a command span's source or kind is past the last")) } else { Ok(()) });
row_codec!(LeaderCommandRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => source: U8, 1 => formation: U64, 2 => kind: U8, 3 => first_hour: I32, 4 => last_hour: I32, 5 => name: Opaque}, |row| if row.source > LeaderCommandRow::FLEET || row.kind > LeaderCommandRow::NAVY { Err(invalid("a command's source or kind is past the last")) } else { Ok(()) });
row_codec!(ThreatTotalRow, 0, |_row| 0, |_key| {  }, {0 => sum: I64}, |_row| Ok(()));
row_codec!(FactionTotalRow, 64, |row| row.faction as u128, |key| { faction: key as u64 }, {0 => owned_states: I64, 1 => controlled_states: I64, 2 => occupied_states: I64, 3 => owned_provinces: I64, 4 => controlled_provinces: I64, 5 => occupied_provinces: I64, 6 => owned_victory_points: I64, 7 => controlled_victory_points: I64, 8 => occupied_victory_points: I64, 9 => owned_area: I64, 10 => controlled_area: I64, 11 => occupied_area: I64, 12 => free1: I64, 13 => free2: I64, 14 => free3: I64, 15 => free4: I64, 16 => free5: I64, 17 => free6: I64, 18 => free7: I64, 19 => built1: I64, 20 => built2: I64, 21 => built3: I64, 22 => built4: I64, 23 => built5: I64, 24 => built6: I64, 25 => built7: I64, 26 => top_bar1: I64, 27 => top_bar2: I64, 28 => top_bar3: I64, 29 => top_bar4: I64, 30 => top_bar5: I64, 31 => top_bar6: I64, 32 => top_bar7: I64}, |_row| Ok(()));
row_codec!(AirRegionActivityRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => day: I32, 1 => friendly: I64, 2 => enemy: I64, 3 => friendly_total: I64, 4 => enemy_total: I64}, |_row| Ok(()));
row_codec!(AirTheatreRegionRow, 96, |row| (row.theatre as u128) << 32 | row.region as u128, |key| { theatre: (key >> 32) as u64, region: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(AirRegionDayRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => day: I32, 1 => activity: I64, 2 => activity_total: I64}, |_row| Ok(()));
row_codec!(AirOwnerDayRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| { country: (key >> 32) as u32, region: key as u32 }, {0 => day: I32, 1 => friendly: I64, 2 => enemy: I64, 3 => friendly_total: I64, 4 => enemy_total: I64}, |_row| Ok(()));
row_codec!(TerritoryRegionTotalRow, 64, |row| (row.holder as u128) << 32 | row.region as u128, |key| { holder: (key >> 32) as u32, region: key as u32 }, {0 => controlled_provinces: I64, 1 => controlled_victory_points: I64, 2 => controlled_area: I64, 3 => controlled_states: I64, 4 => controlled_states_denominator: I64, 5 => owned_provinces: I64, 6 => owned_victory_points: I64, 7 => owned_area: I64, 8 => owned_states: I64, 9 => owned_states_denominator: I64}, |_row| Ok(()));
row_codec!(ProducedValueRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| { country: (key >> 32) as u32, archetype: key as u32 }, {0 => units: I64, 1 => units_reason: U8, 2 => units_without_refits: I64, 3 => units_without_refits_reason: U8, 4 => ic: I64, 5 => ic_reason: U8, 6 => ic_without_refits: I64, 7 => ic_without_refits_reason: U8, 8 => since: OptionalI32, 9 => as_of: OptionalI32}, |_row| Ok(()));
row_codec!(AirLossTotalRow, 128, |row| (row.slot as u128) << 96 | (row.id as u128) << 64 | (row.region as u128) << 32 | row.cause as u128, |key| { slot: (key >> 96) as u32, id: (key >> 64) as u32, region: (key >> 32) as u32, cause: key as u32 }, {0 => dealt_known: I64, 1 => dealt_unknown: I64, 2 => taken_known: I64, 3 => taken_unknown: I64, 4 => late: I32}, |_row| Ok(()));
row_codec!(BombingTotalRow, 72, |row| (row.slot as u128) << 40 | (row.id as u128) << 8 | row.cause as u128, |key| { slot: (key >> 40) as u32, id: (key >> 8) as u32, cause: key as u8 }, {0 => dealt: I64, 1 => taken: I64, 2 => late: I32}, |_row| Ok(()));
row_codec!(GarrisonStrengthRow, 64, |row| (row.controller as u128) << 32 | row.state as u128, |key| { controller: (key >> 32) as u32, state: key as u32 }, {0 => strength: I64, 1 => reason: U8}, |_row| Ok(()));
