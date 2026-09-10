//! Engine-native typed tables and lossless field-mask codecs.
use crate::codec::{Decoder, invalid, put_bytes, put_varint};
use crate::value::{EncodeValue, Value};
pub use extended_rows::*;
use std::io;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClockRow {
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CampaignRow {
    pub id: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryRow {
    /// Raw CCountry link id, which need not equal its canonical array slot.
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Vec<u8>,
    pub name: Vec<u8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryLinkRow {
    /// Raw country reference used by provinces and other engine tables.
    pub id: u32,
    pub canonical_slot: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceRow {
    pub id: u32,
    pub state: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
    /// Native signed, unscaled CProvince +0x38 value.
    pub victory_points: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateRow {
    pub id: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionRow {
    /// Full native handle: type in the low 32 bits, id in the high 32 bits.
    pub id: u64,
    /// Raw country reference of the roster containing this division.
    pub roster_owner: u32,
    /// Raw CUnit owner reference.
    pub logical_owner: u32,
    /// Raw expeditionary-owner reference, including the engine's null value.
    pub expeditionary_owner: u32,
    pub province: u32,
    /// Full native standard-template handle, with the same type/id packing as id.
    pub template: u64,
    /// Native signed fixed-point hit points, with denominator 100,000.
    pub hit_points: i64,
    /// Native signed fixed-point organisation, with denominator 100,000.
    pub organisation: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarRelationRow {
    /// Ordered country identities form the row key; this is not a full war id.
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    /// Raw engine byte, retained without boolean normalization.
    pub cancel: u8,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    /// Raw engine byte, retained without boolean normalization.
    pub first_was_instigator: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionRow {
    pub id: u64,
    /// Literal engine name bytes; an empty name is valid.
    pub name: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionMemberRow {
    /// Raw country identity; a country has at most one faction membership.
    pub country: u32,
    pub faction: u64,
    /// Recorded member order; ordinal zero identifies the first member.
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerRow {
    /// Native signed session id; its raw u32 bits form the row key.
    pub id: i32,
    pub country: u32,
    /// Raw byte, including bits other than the save's country-leader bit.
    pub flags: u8,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HumanControllerRow {
    /// Raw country link id, independently authenticated from the player roster.
    pub country: u32,
    /// Native signed byte; neither zero nor negative values are normalized.
    pub count: i8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatRow {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub day: i32,
    /// Ordered native CArmy handles, including the complete type word.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FleetRow {
    pub id: u64,
    pub owner: u32,
    pub home_base: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskForceRow {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShipRow {
    pub id: u64,
    pub task_force: u64,
    pub strength: i64,
    pub organisation: i64,
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatRow {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: i64,
    pub defender_positioning: i64,
    pub attacker_anti_air: i64,
    pub defender_anti_air: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatParticipantRow {
    /// All 64 handle bits, the side bit, and all 32 slot bits form the 97-bit key.
    pub combat: u64,
    /// Attacker zero, defender one; this selects the native participant vector.
    pub side: u8,
    pub battle_slot: u32,
    /// Full native ship handle; zero is retained for convoy entries.
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirWingRow {
    pub id: u64,
    pub owner: u32,
    pub air_base: u64,
    pub base_override: u64,
    pub country_override: i32,
    pub planes: i32,
    pub effective_planes: i32,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyOrderRow {
    /// All 64 owner handle bits and all 32 instance bits form the 96-bit row key.
    pub owner_group: u64,
    pub instance: u32,
    pub country: u32,
    pub order_type: i32,
    pub active: u8,
    /// Native ordered geometry; no derived path replaces these entries.
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
    pub paradrop_target: u32,
}

pub trait RowCodec: Default + Clone + Eq {
    const KEY_BITS: u32;
    fn key(&self) -> u128;
    fn check_key_fields(&self) -> io::Result<()>;
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>>;
    fn encode_fields(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self>;
    /// Takes ownership so an unchanged string or list keeps its existing allocation.
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self>;
}

trait Fields {
    const SCHEMA: &'static [Kind];
}

pub trait TableSchema {
    type Row: RowCodec;
    const ID: TableId;
    fn borrow(rows: &TableRows) -> Option<&[Self::Row]>;
    fn borrow_mut(rows: &mut TableRows) -> Option<&mut Vec<Self::Row>>;
    fn take(rows: TableRows) -> Result<Vec<Self::Row>, TableRows>;
    fn wrap(rows: Vec<Self::Row>) -> TableRows;
}

macro_rules! define_tables {
    ($($table:ident = $id:literal => $variant:ident($row:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum TableId { $($table = $id),+ }
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum TableRows { $($table(Vec<$row>)),+ }
        /// Inspection borrows concrete rows; decoded storage never owns this enum.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum RowRef<'a> { $($variant(&'a $row)),+ }
        pub enum RowIter<'a> { $($table(std::slice::Iter<'a, $row>)),+ }
        pub mod tables { $(pub struct $table;)+ }
        $(impl TableSchema for tables::$table {
            type Row = $row;
            const ID: TableId = TableId::$table;
            fn borrow(rows: &TableRows) -> Option<&[Self::Row]> {
                match rows { TableRows::$table(rows) => Some(rows), _ => None }
            }
            fn borrow_mut(rows: &mut TableRows) -> Option<&mut Vec<Self::Row>> {
                match rows { TableRows::$table(rows) => Some(rows), _ => None }
            }
            fn take(rows: TableRows) -> Result<Vec<Self::Row>, TableRows> {
                match rows { TableRows::$table(rows) => Ok(rows), rows => Err(rows) }
            }
            fn wrap(rows: Vec<Self::Row>) -> TableRows { TableRows::$table(rows) }
        })+
        impl TableId {
            pub const ALL: [Self; [$(stringify!($table)),+].len()] = [$(Self::$table),+];
            const fn schema(self) -> &'static [Kind] {
                match self { $(Self::$table => <$row as Fields>::SCHEMA),+ }
            }
            pub(crate) const fn key_bits(self) -> u32 {
                match self { $(Self::$table => <$row as RowCodec>::KEY_BITS),+ }
            }
            pub(crate) fn decode_rows(self, count: usize, input: &mut Decoder<'_>) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table(decode_typed::<$row>(count, input)?)),+ })
            }
        }
        impl TableRows {
            pub fn truncate(&mut self, len: usize) {
                match self { $(Self::$table(rows) => rows.truncate(len)),+ }
            }
            pub fn from_rows<T: TableSchema>(rows: Vec<T::Row>) -> Self { T::wrap(rows) }
            pub fn empty(table: TableId) -> Self {
                match table { $(TableId::$table => Self::$table(Vec::new())),+ }
            }
            pub const fn table(&self) -> TableId {
                match self { $(Self::$table(_) => TableId::$table),+ }
            }
            pub fn len(&self) -> usize {
                match self { $(Self::$table(rows) => rows.len()),+ }
            }
            pub fn is_empty(&self) -> bool { self.len() == 0 }
            pub fn rows<T: TableSchema>(&self) -> Option<&[T::Row]> { T::borrow(self) }
            pub fn rows_mut<T: TableSchema>(&mut self) -> Option<&mut Vec<T::Row>> { T::borrow_mut(self) }
            pub fn iter(&self) -> RowIter<'_> {
                match self { $(Self::$table(rows) => RowIter::$table(rows.iter())),+ }
            }
            pub fn get(&self, index: usize) -> Option<RowRef<'_>> { self.iter().nth(index) }
            pub fn first(&self) -> Option<RowRef<'_>> { self.iter().next() }
            pub(crate) fn validate_keys(&self) -> io::Result<()> {
                match self { $(Self::$table(rows) => validate_typed(rows)),+ }
            }
            pub(crate) fn encode_rows(&self, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => encode_typed(rows, output)),+ }
            }
            pub(crate) fn diff(&self, previous: &Self) -> io::Result<(Vec<DeltaRow>, Vec<u128>)> {
                match (self, previous) {
                    $((Self::$table(rows), Self::$table(previous)) => diff_typed(rows, previous)),+,
                    _ => Err(invalid("table rows have different types")),
                }
            }
            pub(crate) fn apply_delta(self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table(apply_typed(rows, changed, deleted)?)),+ })
            }
        }
        impl RowRef<'_> {
            pub const fn table(&self) -> TableId {
                match self { $(Self::$variant(_) => TableId::$table),+ }
            }
            pub fn key(&self) -> u128 {
                match self { $(Self::$variant(row) => row.key()),+ }
            }
        }
        impl<'a> Iterator for RowIter<'a> {
            type Item = RowRef<'a>;
            fn next(&mut self) -> Option<Self::Item> {
                match self { $(Self::$table(rows) => rows.next().map(RowRef::$variant)),+ }
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = self.len(); (len, Some(len))
            }
            fn nth(&mut self, index: usize) -> Option<Self::Item> {
                match self { $(Self::$table(rows) => rows.nth(index).map(RowRef::$variant)),+ }
            }
        }
        impl ExactSizeIterator for RowIter<'_> {
            fn len(&self) -> usize { match self { $(Self::$table(rows) => rows.len()),+ } }
        }
        impl<'a> IntoIterator for &'a TableRows {
            type Item = RowRef<'a>;
            type IntoIter = RowIter<'a>;
            fn into_iter(self) -> Self::IntoIter { self.iter() }
        }
    };
}
define_tables! {
Clock = 0 => Clock(ClockRow),
Campaign = 1 => Campaign(CampaignRow),
Countries = 2 => Country(CountryRow),
CountryLinks = 3 => CountryLink(CountryLinkRow),
Provinces = 4 => Province(ProvinceRow),
States = 5 => State(StateRow),
Divisions = 6 => Division(DivisionRow),
WarRelations = 7 => WarRelation(WarRelationRow),
Factions = 8 => Faction(FactionRow),
FactionMembers = 9 => FactionMember(FactionMemberRow),
Players = 10 => Player(PlayerRow),
HumanControllers = 11 => HumanController(HumanControllerRow),
LandCombats = 12 => LandCombat(LandCombatRow),
Fleets = 13 => Fleet(FleetRow),
TaskForces = 14 => TaskForce(TaskForceRow),
Ships = 15 => Ship(ShipRow),
NavalCombats = 16 => NavalCombat(NavalCombatRow),
NavalCombatParticipants = 17 => NavalCombatParticipant(NavalCombatParticipantRow),
AirWings = 18 => AirWing(AirWingRow),
ArmyOrders = 19 => ArmyOrder(ArmyOrderRow),
Focus = 20 => Focus(FocusRow),
CompletedFocuses = 21 => CompletedFocus(CompletedFocusRow),
ResearchTechnologies = 22 => ResearchTechnology(ResearchTechnologyRow),
ResearchSlots = 23 => ResearchSlot(ResearchSlotRow),
TechnologyDefinitions = 24 => TechnologyDefinition(TechnologyDefinitionRow),
Politics = 25 => Politics(PoliticsRow),
PoliticalParties = 26 => PoliticalParty(PoliticalPartyRow),
ActiveIdeas = 27 => ActiveIdea(ActiveIdeaRow),
DecisionCounts = 28 => DecisionCounts(DecisionCountsRow),
Decisions = 29 => Decision(DecisionRow),
DynamicModifiers = 30 => DynamicModifier(DynamicModifierRow),
PreferredTactics = 31 => PreferredTactic(PreferredTacticRow),
PreferredTacticOperands = 32 => PreferredTacticOperands(PreferredTacticOperandsRow),
TechSharingGroups = 33 => TechSharingGroup(TechSharingGroupRow),
TechSharingMembers = 34 => TechSharingMember(TechSharingMemberRow),
PowerBalances = 35 => PowerBalance(PowerBalanceRow),
PowerBalanceMembers = 36 => PowerBalanceMember(PowerBalanceMemberRow),
PowerBalanceRanges = 37 => PowerBalanceRange(PowerBalanceRangeRow),
PowerBalanceOverrides = 38 => PowerBalanceOverride(PowerBalanceOverrideRow),
ProductionLines = 39 => ProductionLine(ProductionLineRow),
Stockpiles = 40 => Stockpile(StockpileRow),
DivisionEquipment = 41 => DivisionEquipment(DivisionEquipmentRow),
VariantDefinitions = 42 => VariantDefinition(VariantDefinitionRow),
VariantModules = 43 => VariantModule(VariantModuleRow),
VariantStats = 44 => VariantStats(VariantStatsRow),
VariantUpgrades = 45 => VariantUpgrade(VariantUpgradeRow),
ArmyUpgradeGroups = 46 => ArmyUpgradeGroup(ArmyUpgradeGroupRow),
ArmyUpgradeRequests = 47 => ArmyUpgradeRequest(ArmyUpgradeRequestRow),
ArmyUpgradeDeliveries = 48 => ArmyUpgradeDelivery(ArmyUpgradeDeliveryRow),
ArmyUpgradeProduced = 49 => ArmyUpgradeProduced(ArmyUpgradeProducedRow),
CountryFuel = 50 => CountryFuel(CountryFuelRow),
ResourceDefinitions = 51 => ResourceDefinition(ResourceDefinitionRow),
CountryResources = 52 => CountryResource(CountryResourceRow),
ResourceExchanges = 53 => ResourceExchange(ResourceExchangeRow),
PurchaseContracts = 54 => PurchaseContract(PurchaseContractRow),
ProvinceSupply = 55 => ProvinceSupply(ProvinceSupplyRow),
HubMotorization = 56 => HubMotorization(HubMotorizationRow),
DisruptedSupply = 57 => DisruptedSupply(DisruptedSupplyRow),
ForeignHomebases = 58 => ForeignHomebase(ForeignHomebaseRow),
CountryExperience = 59 => CountryExperience(CountryExperienceRow),
StateBuildings = 60 => StateBuilding(BuildingRow),
ProvinceBuildings = 61 => ProvinceBuilding(BuildingRow),
StateBaseResources = 62 => StateBaseResource(StateBaseResourceRow),
BuildingConstruction = 63 => BuildingConstruction(BuildingConstructionRow),
RailwayConstruction = 64 => RailwayConstruction(RailwayConstructionRow),
OccupationSettings = 65 => OccupationSettings(OccupationSettingsRow),
StateOccupationLaws = 66 => StateOccupationLaw(StateOccupationLawRow),
StateGarrisons = 67 => StateGarrison(StateGarrisonRow),
StateAssignedManpower = 68 => StateAssignedManpower(StateAssignedManpowerRow),
CountryOccupationReserveManpower = 69 => CountryOccupationReserveManpower(CountryOccupationReserveManpowerRow),
StateResistance = 70 => StateResistance(StateResistanceRow),
CountryCollaboration = 71 => CountryCollaboration(CountryCollaborationRow),
Characters = 72 => Character(CharacterRow),
CountryLeaderRoles = 73 => CountryLeaderRole(CountryLeaderRoleRow),
AdvisorRoles = 74 => AdvisorRole(AdvisorRoleRow),
UnitLeaders = 75 => UnitLeader(UnitLeaderRow),
UnitLeaderTraits = 76 => UnitLeaderTrait(UnitLeaderTraitRow),
UnitLeaderTraitExperience = 77 => UnitLeaderTraitExperience(UnitLeaderTraitExperienceRow),
ActiveAbilities = 78 => ActiveAbility(ActiveAbilityRow),
Capitulation = 79 => Capitulation(CapitulationRow),
Subjects = 80 => Subject(SubjectRow),
Autonomy = 81 => Autonomy(AutonomyRow),
IntelligenceAgencies = 82 => IntelligenceAgency(IntelligenceAgencyRow),
OperativeMemberships = 83 => OperativeMembership(OperativeMembershipRow),
Cryptology = 84 => Cryptology(CryptologyRow),
IntelNetworks = 85 => IntelNetwork(IntelNetworkRow),
ScriptFlags = 86 => ScriptFlag(ScriptFlagRow),
ScriptVariables = 87 => ScriptVariables(ScriptVariablesRow),
Operations = 88 => Operation(OperationRow),
OperationAssignments = 89 => OperationAssignment(OperationAssignmentRow),
WingEquipment = 90 => WingEquipment(WingEquipmentRow),
AirBases = 91 => AirBase(AirBaseRow),
WingCumulative = 92 => WingCumulative(WingCumulativeRow),
AirRegionHistory = 93 => AirRegionHistory(AirRegionHistoryRow),
AirRegionDamage = 94 => AirRegionDamage(AirRegionDamageRow),
AirAces = 95 => AirAce(AirAceRow),
AirStatistics = 96 => AirStatistics(AirStatisticsRow),
SunkConvoys = 97 => SunkConvoy(SunkConvoyRow),
RailCooldowns = 98 => RailCooldown(RailCooldownRow),
RailEdges = 99 => RailEdge(RailEdgeRow),
LandTheatres = 100 => LandTheatre(LandTheatreRow),
LandTheatreGroups = 101 => LandTheatreGroup(LandTheatreGroupRow),
LandTheatreMembers = 102 => LandTheatreMember(LandTheatreMemberRow),
AirTheatres = 103 => AirTheatre(AirTheatreRow),
AirTheatreGroups = 104 => AirTheatreGroup(AirTheatreGroupRow),
AirTheatreMembers = 105 => AirTheatreMember(AirTheatreMemberRow),
NavalResults = 106 => NavalResult(NavalResultRow),
NavalResultMembers = 107 => NavalResultMember(NavalResultMemberRow),
NavalResultCountries = 108 => NavalResultCountry(NavalResultCountryRow),
SunkShips = 109 => SunkShips(SunkShipsRow),
ProjectPrograms = 110 => ProjectProgram(ProjectProgramRow),
Projects = 111 => Project(ProjectRow),
ProjectHistory = 112 => ProjectHistory(ProjectHistoryRow),
ProjectBreakthroughs = 113 => ProjectBreakthrough(ProjectBreakthroughRow),
Nukes = 114 => Nuke(NukeRow),
RaidCountries = 115 => RaidCountry(RaidCountryRow),
Raids = 116 => Raid(RaidRow),
PeaceConferences = 117 => PeaceConference(PeaceConferenceRow),
WeatherLimits = 118 => WeatherLimits(WeatherLimitsRow),
WeatherRegions = 119 => WeatherRegion(WeatherRegionRow),
WeatherProvinces = 120 => WeatherProvince(WeatherProvinceRow),
OperationTokens = 121 => OperationTokens(OperationTokensRow),
LexicalTokens = 122 => LexicalToken(LexicalTokenRow),
NavalSupremacy = 123 => NavalSupremacy(NavalSupremacyRow),
}
impl TableId {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn field_count(self) -> usize {
        self.schema().len()
    }

    pub const fn mask_width(self) -> usize {
        self.field_count().div_ceil(8)
    }

    pub const fn full_mask(self) -> u64 {
        full_mask(self.field_count())
    }

    pub(crate) fn minimum_row_bytes(self) -> usize {
        1 + self
            .schema()
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>()
    }

    pub(crate) fn minimum_delta_bytes(self) -> usize {
        1 + self.mask_width()
            + self
                .schema()
                .iter()
                .map(|kind| kind.minimum_change_bytes())
                .min()
                .unwrap_or(0)
    }

    pub(crate) fn check_key(self, key: u128) -> io::Result<()> {
        if self.key_bits() < 128 && key >> self.key_bits() != 0 {
            return Err(invalid(format!(
                "{self:?} row key exceeds {} bits",
                self.key_bits()
            )));
        }
        if matches!(self, Self::Clock | Self::Campaign) && key != 0 {
            return Err(invalid("singleton row key must be zero"));
        }
        Ok(())
    }

    pub(crate) fn encode_key(
        self,
        key: u128,
        previous: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<()> {
        self.check_key(key)?;
        let difference = key
            .checked_sub(previous)
            .ok_or_else(|| invalid("row keys decreased"))?;
        put_varint(output, difference);
        Ok(())
    }

    pub(crate) fn decode_key(self, previous: u128, decoder: &mut Decoder<'_>) -> io::Result<u128> {
        let difference = decoder.varint()?;
        let key = previous
            .checked_add(difference)
            .ok_or_else(|| invalid("row key addition overflow"))?;
        self.check_key(key)?;
        Ok(key)
    }

    fn check_mask(self, mask: u64) -> io::Result<()> {
        if mask == 0 || mask & !self.full_mask() != 0 {
            return Err(invalid("delta field mask is empty or has unknown bits"));
        }
        Ok(())
    }
}

impl TryFrom<u8> for TableId {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Self::ALL
            .get(usize::from(value))
            .copied()
            .ok_or_else(|| invalid("unknown table id"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaRow {
    pub key: u128,
    pub mask: u64,
    /// Selected native fields in schema order; exact numeric differences use zigzag varints.
    pub values: Vec<u8>,
}
impl DeltaRow {
    pub fn apply<T: RowCodec>(&self, previous: Option<T>) -> io::Result<T> {
        T::apply_delta(self, previous)
    }
}
const fn full_mask(count: usize) -> u64 {
    if count == 64 {
        u64::MAX
    } else {
        (1_u64 << count) - 1
    }
}
fn check_native_key(key: u128, bits: u32) -> io::Result<()> {
    if bits == 0 && key != 0 {
        return Err(invalid("singleton row key must be zero"));
    }
    if bits < 128 && key >> bits != 0 {
        return Err(invalid("row key exceeds native width"));
    }
    Ok(())
}
pub fn encode_row<T: RowCodec>(row: &T, previous: u128, output: &mut Vec<u8>) -> io::Result<()> {
    row.check_key_fields()?;
    check_native_key(row.key(), T::KEY_BITS)?;
    let difference = row
        .key()
        .checked_sub(previous)
        .ok_or_else(|| invalid("row keys decreased"))?;
    put_varint(output, difference);
    row.encode_fields(output)
}
pub fn decode_row<T: RowCodec>(previous: u128, input: &mut Decoder<'_>) -> io::Result<T> {
    let key = previous
        .checked_add(input.varint()?)
        .ok_or_else(|| invalid("row key addition overflow"))?;
    check_native_key(key, T::KEY_BITS)?;
    T::decode_fields(key, input)
}
fn validate_typed<T: RowCodec>(rows: &[T]) -> io::Result<()> {
    let mut previous = None;
    for row in rows {
        let key = row.key();
        check_native_key(key, T::KEY_BITS)?;
        row.check_key_fields()?;
        if previous.is_some_and(|old| old >= key) {
            return Err(invalid(
                "table rows must have matching types and unique ascending keys",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}
fn encode_typed<T: RowCodec>(rows: &[T], output: &mut Vec<u8>) -> io::Result<()> {
    let mut previous = 0;
    for row in rows {
        encode_row(row, previous, output)?;
        previous = row.key();
    }
    Ok(())
}
fn decode_typed<T: RowCodec>(count: usize, input: &mut Decoder<'_>) -> io::Result<Vec<T>> {
    let mut rows = Vec::with_capacity(count);
    let mut previous = 0;
    for _ in 0..count {
        let row = decode_row::<T>(previous, input)?;
        previous = row.key();
        rows.push(row);
    }
    Ok(rows)
}
fn diff_typed<T: RowCodec>(rows: &[T], previous: &[T]) -> io::Result<(Vec<DeltaRow>, Vec<u128>)> {
    let mut changed = Vec::new();
    let mut deleted = Vec::new();
    let mut old = previous.iter().peekable();
    for row in rows {
        let key = row.key();
        while old.peek().is_some_and(|previous| previous.key() < key) {
            deleted.push(old.next().unwrap().key());
        }
        let baseline = if old.peek().is_some_and(|previous| previous.key() == key) {
            old.next()
        } else {
            None
        };
        // FrameEncoder validates the entire incoming table before diffing;
        // its stored baseline was validated at commit. Keep the common equality
        // check here so unchanged rows never enter the larger delta codec.
        if baseline == Some(row) {
            continue;
        }
        if let Some(delta) = row.delta(baseline)? {
            changed.push(delta);
        }
    }
    deleted.extend(old.map(RowCodec::key));
    Ok((changed, deleted))
}
fn apply_typed<T: RowCodec>(
    previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    let mut rows = Vec::with_capacity(previous.len().saturating_sub(deleted.len()));
    let mut changed = changed.iter().peekable();
    let mut deleted = deleted.iter().peekable();
    for previous in previous {
        let key = previous.key();
        if deleted.peek().is_some_and(|deleted| **deleted < key) {
            return Err(invalid("delta deletes an absent key"));
        }
        while changed.peek().is_some_and(|delta| delta.key < key) {
            rows.push(changed.next().unwrap().apply::<T>(None)?);
        }
        if deleted.peek().is_some_and(|deleted| **deleted == key) {
            deleted.next();
        } else if changed.peek().is_some_and(|delta| delta.key == key) {
            rows.push(changed.next().unwrap().apply(Some(previous))?);
        } else {
            rows.push(previous);
        }
    }
    if deleted.next().is_some() {
        return Err(invalid("delta deletes an absent key"));
    }
    for delta in changed {
        rows.push(delta.apply::<T>(None)?);
    }
    Ok(rows)
}

trait NativeValue: Sized {
    fn field(&self) -> Field<'_>;
    fn from_field(field: Field<'_>) -> io::Result<Self>;
}
macro_rules! native_scalar {
    ($ty:ty, $kind:ident) => {
        impl NativeValue for $ty {
            fn field(&self) -> Field<'_> {
                Field::$kind(*self)
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => Ok(value),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
native_scalar!(u8, U8);
native_scalar!(i8, I8);
native_scalar!(u32, U32);
native_scalar!(i32, I32);
native_scalar!(u64, U64);
native_scalar!(i64, I64);
native_scalar!(Option<i64>, OptionalI64);
native_scalar!(Option<u32>, OptionalU32);
native_scalar!(u16, U16);
native_scalar!(i16, I16);
native_scalar!(bool, Bool);
native_scalar!(Option<i32>, OptionalI32);
native_scalar!(Option<u64>, OptionalU64);
impl NativeValue for Vec<u8> {
    fn field(&self) -> Field<'_> {
        Field::Bytes(self)
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::Bytes(value) => Ok(value.to_vec()),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}
macro_rules! native_vector {
    ($ty:ty, $kind:ident) => {
        impl NativeValue for Vec<$ty> {
            fn field(&self) -> Field<'_> {
                Field::$kind(List::Native(self))
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => Ok(value.to_vec()),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
native_vector!(u32, U32List);
native_vector!(u64, U64List);
fn read_value<T: NativeValue>(kind: Kind, input: &mut Decoder<'_>) -> io::Result<T> {
    T::from_field(kind.decode(input)?)
}
fn apply_value<T: NativeValue>(
    target: &mut T,
    kind: Kind,
    input: &mut Decoder<'_>,
) -> io::Result<()> {
    let value = T::from_field(kind.decode_change(input)?.apply(kind, target.field())?)?;
    *target = value;
    Ok(())
}
macro_rules! typed_codec {
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     |$decode_key:ident, $input:ident| $decode:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident),+ $(,)? }, |$checked:ident| $check:expr) => {
        impl Fields for $ty { const SCHEMA: &'static [Kind] = &[$(Kind::$kind),+]; }
        impl $ty { pub const fn key(&self) -> u128 { let $row = self; $key } }
        impl RowCodec for $ty {
            const KEY_BITS: u32 = $bits;
            fn key(&self) -> u128 { self.key() }
            fn check_key_fields(&self) -> io::Result<()> { let $checked = self; $check }
            fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != self.key() { return Err(invalid("delta row and baseline have different keys")); }
                    if self == previous { return Ok(None); }
                }
                let mut delta = DeltaRow { key: self.key(), mask: 0, values: Vec::new() };
                $(
                    if previous.is_none_or(|old| old.$($field)+ != self.$($field)+) {
                        delta.mask |= 1_u64 << $index;
                        self.$($field)+.field().change(previous.map_or(Field::Empty, |old| old.$($field)+.field()))?
                            .encode(&mut delta.values)?;
                    }
                )+
                Ok(Some(delta))
            }
            fn encode_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
                $(self.$($field)+.field().encode(output)?;)+
                Ok(())
            }
            fn decode_fields($decode_key: u128, $input: &mut Decoder<'_>) -> io::Result<Self> {
                $decode
            }
            fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> {
                check_native_key(delta.key, Self::KEY_BITS)?;
                let mask = full_mask(Self::SCHEMA.len());
                if delta.mask == 0 || delta.mask & !mask != 0 { return Err(invalid("delta field mask is empty or has unknown bits")); }
                let mut row = if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != delta.key { return Err(invalid("delta row and baseline have different keys")); }
                    previous
                } else {
                    if delta.mask != mask { return Err(invalid("inserted row requires every field")); }
                    let $key_name = delta.key;
                    $initial
                };
                let mut input = Decoder::new(&delta.values);
                $(if delta.mask & (1_u64 << $index) != 0 { apply_value(&mut row.$($field)+, Kind::$kind, &mut input)?; })+
                input.finish()?;
                Ok(row)
            }
        }
    };
}

macro_rules! row_codec {
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| { $($key_field:ident : $key_value:expr),* $(,)? },
     { $($index:literal => $field:ident : $kind:ident),+ $(,)? }, |$checked:ident| $check:expr) => {
        typed_codec!($ty, $bits, |$row| $key,
            |$key_name| Self { $($key_field: $key_value,)* $($field: Default::default(),)+ },
            |$key_name, input| {
                $(let $field = read_value(Kind::$kind, input)?;)+
                Ok(Self { $($key_field: $key_value,)* $($field,)+ })
            },
            { $($index => ($field): $kind),+ }, |$checked| $check);
    };
}
row_codec!(ClockRow, 0, |_row| 0, |_key| {  }, {0 => hour: I32, 1 => play_time: U64, 2 => play_session_started_at: I64, 3 => seed: I32}, |_row| Ok(()));
row_codec!(CampaignRow, 0, |_row| 0, |_key| {  }, {0 => id: Bytes}, |_row| Ok(()));
row_codec!(CountryRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32, 1 => tag: Bytes, 2 => name: Bytes, 3 => political_power_raw: OptionalI64, 4 => command_power_raw: I64, 5 => stability_raw: I64, 6 => war_support_raw: I64, 7 => convoys_raw: OptionalI64}, |_row| Ok(()));
row_codec!(CountryLinkRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32}, |_row| Ok(()));
row_codec!(ProvinceRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => state: U32, 1 => owner: U32, 2 => controller: U32, 3 => victory_points: I32}, |_row| Ok(()));
row_codec!(StateRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32}, |_row| Ok(()));
row_codec!(DivisionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64, 6 => organisation: I64}, |_row| Ok(()));
row_codec!(WarRelationRow, 64, |row| (row.first_country as u128) << 32 | row.second_country as u128, |key| { first_country: (key >> 32) as u32, second_country: key as u32 }, {0 => start_hour: I32, 1 => end_hour: I32, 2 => cancel: U8, 3 => first_casualties: I64, 4 => second_casualties: I64, 5 => first_unknown_casualties: I64, 6 => second_unknown_casualties: I64, 7 => first_was_instigator: U8}, |_row| Ok(()));
row_codec!(FactionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => name: Bytes}, |_row| Ok(()));
row_codec!(FactionMemberRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => faction: U64, 1 => ordinal: U32}, |_row| Ok(()));
row_codec!(PlayerRow, 32, |row| row.id as u32 as u128, |key| { id: key as u32 as i32 }, {0 => country: U32, 1 => flags: U8, 2 => user: Bytes, 3 => name: Bytes}, |_row| Ok(()));
row_codec!(HumanControllerRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => count: I8}, |_row| Ok(()));
row_codec!(LandCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => duration: I32, 2 => day: I32, 3 => attackers: U64List, 4 => defenders: U64List}, |_row| Ok(()));
row_codec!(FleetRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => home_base: U32}, |_row| Ok(()));
row_codec!(TaskForceRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => fleet: U64, 1 => roster_owner: U32, 2 => logical_owner: U32, 3 => expeditionary_owner: U32, 4 => province: U32, 5 => mission: U32}, |_row| Ok(()));
row_codec!(ShipRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => task_force: U64, 1 => strength: I64, 2 => organisation: I64, 3 => experience: I64, 4 => category: U32, 5 => sunk_convoys: I32, 6 => class_id: I32}, |_row| Ok(()));
row_codec!(NavalCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => attacker_positioning: I64, 2 => defender_positioning: I64, 3 => attacker_anti_air: I64, 4 => defender_anti_air: I64}, |_row| Ok(()));
row_codec!(NavalCombatParticipantRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, {0 => ship: U64, 1 => owner: U32, 2 => state: U32}, |row| if row.side > 1 { Err(invalid("naval combat participant side exceeds one")) } else { Ok(()) });
row_codec!(AirWingRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => air_base: U64, 2 => base_override: U64, 3 => country_override: I32, 4 => planes: I32, 5 => effective_planes: I32, 6 => mission_mask: U32, 7 => mission_region: OptionalU32}, |_row| Ok(()));
row_codec!(ArmyOrderRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| { owner_group: (key >> 32) as u64, instance: key as u32 }, {0 => country: U32, 1 => order_type: I32, 2 => active: U8, 3 => paradrop_target: U32, 4 => path: U32List, 5 => target_states: U32List}, |_row| Ok(()));
pub fn encode_delta(
    table: TableId,
    delta: &DeltaRow,
    previous_key: u128,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    table.check_key(delta.key)?;
    table.check_mask(delta.mask)?;
    let mut decoder = Decoder::new(&delta.values);
    for (index, kind) in table.schema().iter().enumerate() {
        if delta.mask & (1_u64 << index) != 0 {
            kind.decode_change(&mut decoder)?;
        }
    }
    decoder.finish()?;
    table.encode_key(delta.key, previous_key, output)?;
    output.extend_from_slice(&delta.mask.to_le_bytes()[..table.mask_width()]);
    output.extend_from_slice(&delta.values);
    Ok(())
}

pub fn decode_delta(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'_>,
) -> io::Result<DeltaRow> {
    let key = table.decode_key(previous_key, decoder)?;
    let mut mask_bytes = [0; 8];
    mask_bytes[..table.mask_width()].copy_from_slice(decoder.take(table.mask_width())?);
    let mask = u64::from_le_bytes(mask_bytes);
    table.check_mask(mask)?;
    let mut values = Vec::new();
    for (index, kind) in table.schema().iter().enumerate() {
        if mask & (1_u64 << index) != 0 {
            kind.decode_change(decoder)?.encode(&mut values)?;
        }
    }
    Ok(DeltaRow { key, mask, values })
}

#[derive(Clone, Copy)]
enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    Bool,
    OptionalI32,
    OptionalU32,
    OptionalI64,
    OptionalU64,
    Bytes,
    U32List,
    U64List,
    Opaque,
}
impl Kind {
    fn minimum_row_bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I8 => 1,
            Self::U16 => 2,
            Self::I16 => 2,
            Self::U32 => 4,
            Self::I32 => 4,
            Self::U64 => 8,
            Self::I64 => 8,
            Self::Bool => 1,
            Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64 => 1,
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque => 4,
        }
    }
    fn minimum_change_bytes(self) -> usize {
        match self {
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque => 4,
            _ => 1,
        }
    }
    fn decode_change<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Change<'a>> {
        if matches!(
            self,
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque
        ) {
            return Ok(match self.decode(decoder)? {
                Field::Bytes(value) => Change::Bytes(value),
                Field::U32List(value) => Change::U32List(value),
                Field::U64List(value) => Change::U64List(value),
                Field::Opaque(value) => Change::Opaque(value),
                _ => unreachable!(),
            });
        }
        if matches!(
            self,
            Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64
        ) {
            match decoder.u8()? {
                0 => return Ok(Change::Optional(None)),
                1 => {}
                _ => return Err(invalid("invalid optional integer tag")),
            }
        }
        let encoded = decoder.varint()?;
        let difference = ((encoded >> 1) as i128) ^ -((encoded & 1) as i128);
        let maximum = match self {
            Self::U8 => u128::from(u8::MAX),
            Self::I8 => u128::from(u8::MAX),
            Self::U16 => u128::from(u16::MAX),
            Self::I16 => u128::from(u16::MAX),
            Self::U32 => u128::from(u32::MAX),
            Self::I32 => u128::from(u32::MAX),
            Self::U64 => u128::from(u64::MAX),
            Self::I64 => u128::from(u64::MAX),
            Self::Bool => 1,
            Self::OptionalI32 => u128::from(u32::MAX),
            Self::OptionalU32 => u128::from(u32::MAX),
            Self::OptionalI64 => u128::from(u64::MAX),
            Self::OptionalU64 => u128::from(u64::MAX),
            _ => unreachable!(),
        };
        if difference.unsigned_abs() > maximum {
            return Err(invalid("numeric difference exceeds native field range"));
        }
        Ok(
            if matches!(
                self,
                Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64
            ) {
                Change::Optional(Some(difference))
            } else {
                Change::Number(difference)
            },
        )
    }
    fn number(self, value: i128) -> io::Result<Field<'static>> {
        let range = |_| invalid("numeric delta exceeds native field range");
        Ok(match self {
            Self::U8 => Field::U8(u8::try_from(value).map_err(range)?),
            Self::I8 => Field::I8(i8::try_from(value).map_err(range)?),
            Self::U16 => Field::U16(u16::try_from(value).map_err(range)?),
            Self::I16 => Field::I16(i16::try_from(value).map_err(range)?),
            Self::U32 => Field::U32(u32::try_from(value).map_err(range)?),
            Self::I32 => Field::I32(i32::try_from(value).map_err(range)?),
            Self::U64 => Field::U64(u64::try_from(value).map_err(range)?),
            Self::I64 => Field::I64(i64::try_from(value).map_err(range)?),
            Self::Bool => Field::Bool(match value {
                0 => false,
                1 => true,
                _ => return Err(invalid("numeric delta exceeds native field range")),
            }),
            Self::OptionalI32 => Field::OptionalI32(Some(i32::try_from(value).map_err(range)?)),
            Self::OptionalU32 => Field::OptionalU32(Some(u32::try_from(value).map_err(range)?)),
            Self::OptionalI64 => Field::OptionalI64(Some(i64::try_from(value).map_err(range)?)),
            Self::OptionalU64 => Field::OptionalU64(Some(u64::try_from(value).map_err(range)?)),
            _ => return Err(invalid("numeric change applied to list field")),
        })
    }
    fn decode<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Field<'a>> {
        Ok(match self {
            Self::U8 => Field::U8(<u8 as Value>::decode(decoder)?),
            Self::I8 => Field::I8(<i8 as Value>::decode(decoder)?),
            Self::U16 => Field::U16(<u16 as Value>::decode(decoder)?),
            Self::I16 => Field::I16(<i16 as Value>::decode(decoder)?),
            Self::U32 => Field::U32(<u32 as Value>::decode(decoder)?),
            Self::I32 => Field::I32(<i32 as Value>::decode(decoder)?),
            Self::U64 => Field::U64(<u64 as Value>::decode(decoder)?),
            Self::I64 => Field::I64(<i64 as Value>::decode(decoder)?),
            Self::Bool => Field::Bool(<bool as Value>::decode(decoder)?),
            Self::OptionalI32 => Field::OptionalI32(match decoder.u8()? {
                0 => None,
                1 => Some(<i32 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalU32 => Field::OptionalU32(match decoder.u8()? {
                0 => None,
                1 => Some(<u32 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalI64 => Field::OptionalI64(match decoder.u8()? {
                0 => None,
                1 => Some(<i64 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalU64 => Field::OptionalU64(match decoder.u8()? {
                0 => None,
                1 => Some(<u64 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::Bytes | Self::Opaque => {
                let length = usize::try_from(decoder.u32()?)
                    .map_err(|_| invalid("row byte length exceeds address space"))?;
                let bytes = decoder.take(length)?;
                if matches!(self, Self::Bytes) {
                    Field::Bytes(bytes)
                } else {
                    Field::Opaque(Opaque::Encoded(bytes))
                }
            }
            Self::U32List => {
                let count = decoder.count(4)?;
                Field::U32List(List::Encoded(decoder.take(count * 4)?))
            }
            Self::U64List => {
                let count = decoder.count(8)?;
                Field::U64List(List::Encoded(decoder.take(count * 8)?))
            }
        })
    }
}
#[derive(Clone, Copy)]
enum Opaque<'a> {
    Native(&'a dyn EncodeValue),
    Encoded(&'a [u8]),
}
impl Opaque<'_> {
    fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Encoded(bytes) => put_bytes(output, bytes),
            Self::Native(value) => {
                let prefix = output.len();
                output.extend_from_slice(&0_u32.to_le_bytes());
                value.encode_value(output)?;
                let length = u32::try_from(output.len() - prefix - 4)
                    .map_err(|_| invalid("structured field exceeds u32"))?;
                output[prefix..prefix + 4].copy_from_slice(&length.to_le_bytes());
                Ok(())
            }
        }
    }
}
enum Change<'a> {
    Number(i128),
    Optional(Option<i128>),
    Bytes(&'a [u8]),
    U32List(List<'a, u32>),
    U64List(List<'a, u64>),
    Opaque(Opaque<'a>),
}
impl<'a> Change<'a> {
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let difference = match self {
            Self::Number(value) => *value,
            Self::Optional(Some(value)) => {
                output.push(1);
                *value
            }
            Self::Optional(None) => {
                output.push(0);
                return Ok(());
            }
            Self::Bytes(value) => return put_bytes(output, value),
            Self::U32List(value) => return value.encode(output),
            Self::U64List(value) => return value.encode(output),
            Self::Opaque(value) => return value.encode(output),
        };
        let zigzag = ((difference as u128) << 1) ^ ((difference >> 127) as u128);
        put_varint(output, zigzag);
        Ok(())
    }
    fn apply(self, kind: Kind, previous: Field<'a>) -> io::Result<Field<'a>> {
        match self {
            Self::Bytes(value) => Ok(Field::Bytes(value)),
            Self::U32List(value) => Ok(Field::U32List(value)),
            Self::U64List(value) => Ok(Field::U64List(value)),
            Self::Opaque(value) => Ok(Field::Opaque(value)),
            Self::Optional(None) => match kind {
                Kind::OptionalI32 => Ok(Field::OptionalI32(None)),
                Kind::OptionalU32 => Ok(Field::OptionalU32(None)),
                Kind::OptionalI64 => Ok(Field::OptionalI64(None)),
                Kind::OptionalU64 => Ok(Field::OptionalU64(None)),
                _ => Err(invalid("optional change applied to nonoptional field")),
            },
            Self::Number(difference) | Self::Optional(Some(difference)) => {
                let value = previous
                    .number()?
                    .checked_add(difference)
                    .ok_or_else(|| invalid("numeric delta addition overflow"))?;
                kind.number(value)
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Empty,
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Bytes(&'a [u8]),
    U32List(List<'a, u32>),
    U64List(List<'a, u64>),
    Opaque(Opaque<'a>),
}
impl<'a> Field<'a> {
    fn number(self) -> io::Result<i128> {
        Ok(match self {
            Self::Empty
            | Self::OptionalI32(None)
            | Self::OptionalU32(None)
            | Self::OptionalI64(None)
            | Self::OptionalU64(None) => 0,
            Self::U8(value) => i128::from(value),
            Self::I8(value) => i128::from(value),
            Self::U16(value) => i128::from(value),
            Self::I16(value) => i128::from(value),
            Self::U32(value) => i128::from(value),
            Self::I32(value) => i128::from(value),
            Self::U64(value) => i128::from(value),
            Self::I64(value) => i128::from(value),
            Self::Bool(value) => i128::from(u8::from(value)),
            Self::OptionalI32(Some(value)) => i128::from(value),
            Self::OptionalU32(Some(value)) => i128::from(value),
            Self::OptionalI64(Some(value)) => i128::from(value),
            Self::OptionalU64(Some(value)) => i128::from(value),
            _ => return Err(invalid("list field used as numeric baseline")),
        })
    }
    fn change(self, previous: Self) -> io::Result<Change<'a>> {
        match self {
            Self::Bytes(value) => return Ok(Change::Bytes(value)),
            Self::U32List(value) => return Ok(Change::U32List(value)),
            Self::U64List(value) => return Ok(Change::U64List(value)),
            Self::Opaque(value) => return Ok(Change::Opaque(value)),
            Self::OptionalI32(None)
            | Self::OptionalU32(None)
            | Self::OptionalI64(None)
            | Self::OptionalU64(None) => return Ok(Change::Optional(None)),
            _ => {}
        }
        let difference = self
            .number()?
            .checked_sub(previous.number()?)
            .ok_or_else(|| invalid("numeric delta subtraction overflow"))?;
        Ok(
            if matches!(
                self,
                Self::OptionalI32(_)
                    | Self::OptionalU32(_)
                    | Self::OptionalI64(_)
                    | Self::OptionalU64(_)
            ) {
                Change::Optional(Some(difference))
            } else {
                Change::Number(difference)
            },
        )
    }
    fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Empty => return Err(invalid("missing row field")),
            Self::U8(value) => <u8 as Value>::encode(&value, output)?,
            Self::I8(value) => <i8 as Value>::encode(&value, output)?,
            Self::U16(value) => <u16 as Value>::encode(&value, output)?,
            Self::I16(value) => <i16 as Value>::encode(&value, output)?,
            Self::U32(value) => <u32 as Value>::encode(&value, output)?,
            Self::I32(value) => <i32 as Value>::encode(&value, output)?,
            Self::U64(value) => <u64 as Value>::encode(&value, output)?,
            Self::I64(value) => <i64 as Value>::encode(&value, output)?,
            Self::Bool(value) => <bool as Value>::encode(&value, output)?,
            Self::OptionalI32(value) => <Option<i32> as Value>::encode(&value, output)?,
            Self::OptionalU32(value) => <Option<u32> as Value>::encode(&value, output)?,
            Self::OptionalI64(value) => <Option<i64> as Value>::encode(&value, output)?,
            Self::OptionalU64(value) => <Option<u64> as Value>::encode(&value, output)?,
            Self::Bytes(value) => put_bytes(output, value)?,
            Self::U32List(value) => value.encode(output)?,
            Self::U64List(value) => value.encode(output)?,
            Self::Opaque(value) => value.encode(output)?,
        }
        Ok(())
    }
}
macro_rules! opaque_value {
    ($ty:ty) => {
        impl NativeValue for $ty {
            fn field(&self) -> Field<'_> {
                Field::Opaque(Opaque::Native(self))
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                let Field::Opaque(Opaque::Encoded(bytes)) = field else {
                    return Err(invalid("structured field requires encoded native bytes"));
                };
                let mut input = Decoder::new(bytes);
                let value = <$ty as Value>::decode(&mut input)?;
                input.finish()?;
                Ok(value)
            }
        }
    };
}

/// A field borrows either decoded native values or checked wire bytes. Decoding a
/// list allocates only when constructing its owning row, never a temporary list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum List<'a, T> {
    Native(&'a [T]),
    Encoded(&'a [u8]),
}

macro_rules! native_list {
    ($ty:ty, $width:expr) => {
        impl List<'_, $ty> {
            fn to_vec(self) -> Vec<$ty> {
                match self {
                    Self::Native(values) => values.to_vec(),
                    Self::Encoded(bytes) => bytes
                        .chunks_exact($width)
                        .map(|value| <$ty>::from_le_bytes(value.try_into().unwrap()))
                        .collect(),
                }
            }

            fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
                let length = match self {
                    Self::Native(values) => values.len(),
                    Self::Encoded(bytes) => bytes.len() / $width,
                };
                let count = u32::try_from(length).map_err(|_| invalid("list field exceeds u32"))?;
                output.extend_from_slice(&count.to_le_bytes());
                match self {
                    Self::Native(values) => {
                        for value in values {
                            output.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Self::Encoded(bytes) => output.extend_from_slice(bytes),
                }
                Ok(())
            }
        }
    };
}

native_list!(u32, 4);
native_list!(u64, 8);

#[path = "extended_rows.rs"]
mod extended_rows;

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: RowCodec + std::fmt::Debug>(table: TableId, row: T) {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<T>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
        for cut in 0..bytes.len() {
            assert!(decode_row::<T>(0, &mut Decoder::new(&bytes[..cut])).is_err());
        }
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, table.full_mask());
        assert_eq!(delta.apply::<T>(None).unwrap(), row);
        bytes.clear();
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let restored = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored, delta);
        assert_eq!(restored.apply::<T>(None).unwrap(), row);
    }
    fn pair<T: RowCodec + std::fmt::Debug>(table: TableId, low: T, high: T) {
        for (old, new) in [(&low, &high), (&high, &low)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            let mut bytes = Vec::new();
            encode_delta(table, &delta, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            let restored = decode_delta(table, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(old.clone())).unwrap(), *new);
            for cut in 0..bytes.len() {
                assert!(decode_delta(table, 0, &mut Decoder::new(&bytes[..cut])).is_err());
            }
        }
    }
    fn country() -> CountryRow {
        CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            name: vec![b'G', 0xff, b'r'],
            political_power_raw: Some(i64::MIN),
            command_power_raw: -9_007_199_254_740_993,
            stability_raw: 100_001,
            war_support_raw: 99_999,
            convoys_raw: Some(i64::MAX),
        }
    }
    fn division() -> DivisionRow {
        DivisionRow {
            id: 0x8000_0001_0000_0029,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 123,
            template: 0x8000_0002_0000_002a,
            hit_points: -9_007_199_254_740_993,
            organisation: 100_001,
        }
    }
    fn war_relation() -> WarRelationRow {
        WarRelationRow {
            first_country: 7,
            second_country: 19,
            start_hour: i32::MIN,
            end_hour: i32::MAX,
            cancel: 0,
            first_casualties: i64::MIN,
            second_casualties: i64::MAX,
            first_unknown_casualties: i64::MIN,
            second_unknown_casualties: i64::MAX,
            first_was_instigator: 0,
        }
    }
    #[test]
    fn full_rows_and_insertions_preserve_native_values_and_raw_aliases() {
        round_trip(
            TableId::Clock,
            ClockRow {
                hour: i32::MIN,
                play_time: u64::MAX,
                play_session_started_at: i64::MIN,
                seed: i32::MAX,
            },
        );
        round_trip(
            TableId::Campaign,
            CampaignRow {
                id: vec![0xff, 0, b'a'],
            },
        );
        round_trip(TableId::Countries, country());
        round_trip(
            TableId::CountryLinks,
            CountryLinkRow {
                id: 2,
                canonical_slot: 1,
            },
        );
        round_trip(
            TableId::Provinces,
            ProvinceRow {
                id: 123,
                state: 456,
                owner: 7,
                controller: 2,
                victory_points: i32::MIN,
            },
        );
        round_trip(
            TableId::States,
            StateRow {
                id: u32::MAX,
                owner: 0,
                controller: u32::MAX,
            },
        );
        round_trip(
            TableId::Divisions,
            DivisionRow {
                id: u64::MAX,
                roster_owner: u32::MAX,
                logical_owner: 0,
                expeditionary_owner: u32::MAX,
                province: u32::MAX,
                template: u64::MAX,
                hit_points: i64::MIN,
                organisation: i64::MAX,
            },
        );
        round_trip(
            TableId::WarRelations,
            WarRelationRow {
                first_country: u32::MAX,
                second_country: u32::MAX,
                cancel: u8::MAX,
                first_was_instigator: 2,
                ..war_relation()
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: 0,
                name: Vec::new(),
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: u64::MAX,
                name: vec![0, 0xff, b'A'],
            },
        );
        round_trip(
            TableId::FactionMembers,
            FactionMemberRow {
                country: u32::MAX,
                faction: u64::MAX,
                ordinal: u32::MAX,
            },
        );
    }

    #[test]
    fn province_masks_keep_exact_victory_points_independent_of_control() {
        let previous = ProvinceRow {
            id: u32::MAX,
            state: 7,
            owner: 19,
            controller: 0,
            victory_points: i32::MIN,
        };
        assert_eq!(TableId::Provinces.full_mask(), 15);
        for mask in 1..=15 {
            let next = ProvinceRow {
                id: previous.id,
                state: if mask & 1 != 0 {
                    u32::MAX
                } else {
                    previous.state
                },
                owner: if mask & 2 != 0 { 0 } else { previous.owner },
                controller: if mask & 4 != 0 {
                    u32::MAX
                } else {
                    previous.controller
                },
                victory_points: if mask & 8 != 0 {
                    i32::MAX
                } else {
                    previous.victory_points
                },
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut wire = Vec::new();
            encode_delta(TableId::Provinces, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let restored = decode_delta(TableId::Provinces, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(previous.clone())).unwrap(), next);
            assert_eq!(
                previous
                    .delta(Some(&next))
                    .unwrap()
                    .unwrap()
                    .apply(Some(next.clone()))
                    .unwrap(),
                previous
            );
            round_trip(TableId::Provinces, next);
        }
        for victory_points in [i32::MIN, -1, 0, 50, i32::MAX] {
            let row = ProvinceRow {
                victory_points,
                ..previous.clone()
            };
            let mut wire = Vec::new();
            encode_row(&row, 0, &mut wire).unwrap();
            assert_eq!(&wire[wire.len() - 4..], &victory_points.to_le_bytes());
            round_trip(TableId::Provinces, row);
        }
    }

    #[test]
    fn map_rows_preserve_full_native_handles_signed_values_and_ordered_lists() {
        round_trip(
            TableId::Players,
            PlayerRow {
                id: i32::MIN,
                country: u32::MAX,
                flags: u8::MAX,
                user: vec![0xff, 0],
                name: vec![0, 0xfe],
            },
        );
        round_trip(
            TableId::HumanControllers,
            HumanControllerRow {
                country: u32::MAX,
                count: i8::MIN,
            },
        );
        round_trip(
            TableId::LandCombats,
            LandCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                duration: i32::MIN,
                day: i32::MAX,
                attackers: vec![u64::MAX, 0, 1_u64 << 63],
                defenders: vec![1, u64::MAX],
            },
        );
        round_trip(
            TableId::Fleets,
            FleetRow {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX,
            },
        );
        round_trip(
            TableId::TaskForces,
            TaskForceRow {
                id: u64::MAX,
                fleet: u64::MAX,
                roster_owner: u32::MAX,
                logical_owner: 0,
                expeditionary_owner: u32::MAX,
                province: u32::MAX,
                mission: u32::MAX,
            },
        );
        round_trip(
            TableId::Ships,
            ShipRow {
                id: u64::MAX,
                task_force: u64::MAX,
                strength: i64::MIN,
                organisation: i64::MAX,
                experience: -9_007_199_254_740_993,
                category: u32::MAX,
                sunk_convoys: i32::MIN,
                class_id: i32::MIN,
            },
        );
        round_trip(
            TableId::NavalCombats,
            NavalCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                attacker_positioning: i64::MIN,
                defender_positioning: i64::MAX,
                attacker_anti_air: -9_007_199_254_740_993,
                defender_anti_air: 9_007_199_254_740_993,
            },
        );
        round_trip(
            TableId::NavalCombatParticipants,
            NavalCombatParticipantRow {
                combat: u64::MAX,
                side: 1,
                battle_slot: u32::MAX,
                ship: u64::MAX,
                owner: u32::MAX,
                state: u32::MAX,
            },
        );
        round_trip(
            TableId::AirWings,
            AirWingRow {
                id: u64::MAX,
                owner: u32::MAX,
                air_base: u64::MAX,
                base_override: 1_u64 << 63,
                country_override: i32::MIN,
                planes: i32::MIN,
                effective_planes: i32::MAX,
                mission_mask: u32::MAX,
                mission_region: Some(u32::MAX),
            },
        );
        round_trip(
            TableId::ArmyOrders,
            ArmyOrderRow {
                owner_group: u64::MAX,
                instance: u32::MAX,
                country: u32::MAX,
                order_type: i32::MIN,
                active: u8::MAX,
                paradrop_target: u32::MAX,
                path: vec![u32::MAX, 0, 1_u32 << 31],
                target_states: vec![0, u32::MAX],
            },
        );
    }

    #[test]
    fn one_native_scalar_tick_costs_three_bytes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 5);
        assert_eq!(delta.values, [2]);
        let mut bytes = Vec::new();
        encode_delta(TableId::Countries, &delta, 0, &mut bytes).unwrap();
        assert_eq!(bytes, [7, 0x20, 2]);
        assert_eq!(delta.apply::<CountryRow>(Some(old.clone())).unwrap(), new);
    }

    #[test]
    fn state_masks_keep_owner_and_controller_as_independent_raw_references() {
        let old = StateRow {
            id: 0x1234_5678,
            owner: 7,
            controller: 2,
        };
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        for mask in 1..=TableId::States.full_mask() {
            let new = StateRow {
                id: old.key().try_into().unwrap(),
                owner: if mask & 1 != 0 { 0 } else { 7 },
                controller: if mask & 2 != 0 { u32::MAX } else { 2 },
            };
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::States, &delta, 0, &mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                5 + 1 + usize::from(mask & 1 != 0) + 5 * usize::from(mask & 2 != 0)
            );
            assert_eq!(&bytes[..5], &[0xf8, 0xac, 0xd1, 0x91, 0x01]);
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::States, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply::<StateRow>(Some(old.clone())).unwrap(), new);
            if mask != TableId::States.full_mask() {
                assert!(decoded.apply::<StateRow>(None).is_err());
            }
        }
    }

    #[test]
    fn division_rows_keep_full_handles_and_native_fixed_point_bytes() {
        for id in [0, 1_u64 << 63, u64::MAX] {
            let row = DivisionRow { id, ..division() };
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let key_len = if id == 0 { 1 } else { 10 };
            assert_eq!(bytes.len(), key_len + 40);
            assert_eq!(
                &bytes[key_len + 16..key_len + 24],
                &division().template.to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 24..key_len + 32],
                &division().hit_points.to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 32..key_len + 40],
                &division().organisation.to_le_bytes()
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<DivisionRow>(0, &mut input).unwrap(), row);
            input.finish().unwrap();
            for cut in 0..bytes.len() {
                assert!(decode_row::<DivisionRow>(0, &mut Decoder::new(&bytes[..cut])).is_err());
            }
            let delta = row.delta(None).unwrap().unwrap();
            bytes.clear();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), key_len + 1 + delta.values.len());
            assert_eq!(bytes[key_len], 0x7f);
            assert_eq!(delta.apply::<DivisionRow>(None).unwrap(), row);
        }
    }

    #[test]
    fn division_masks_preserve_every_field_subset_without_quantization() {
        let original = division();
        let old = original.clone();
        for mask in 1..=TableId::Divisions.full_mask() {
            let mut updated = original.clone();
            if mask & 1 != 0 {
                updated.roster_owner = u32::MAX;
            }
            if mask & 2 != 0 {
                updated.logical_owner = 0;
            }
            if mask & 4 != 0 {
                updated.expeditionary_owner = 7;
            }
            if mask & 8 != 0 {
                updated.province = u32::MAX;
            }
            if mask & 16 != 0 {
                updated.template = u64::MAX;
            }
            if mask & 32 != 0 {
                updated.hit_points = i64::MIN;
            }
            if mask & 64 != 0 {
                updated.organisation = i64::MAX;
            }
            let new = updated;
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), 10 + 1 + delta.values.len());
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::Divisions, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(
                decoded.apply::<DivisionRow>(Some(old.clone())).unwrap(),
                new
            );
            assert_eq!(
                old.delta(Some(&new))
                    .unwrap()
                    .unwrap()
                    .apply::<DivisionRow>(Some(new.clone()))
                    .unwrap(),
                old
            );
            if mask != TableId::Divisions.full_mask() {
                assert!(decoded.apply::<DivisionRow>(None).is_err());
            }
        }
    }

    #[test]
    fn war_relation_keys_keep_ordered_endpoints_out_of_the_payload() {
        let mut keys = Vec::new();
        for (first_country, second_country) in [
            (0, 0),
            (0, u32::MAX),
            (7, 19),
            (19, 7),
            (u32::MAX, 0),
            (u32::MAX, u32::MAX),
        ] {
            let expected = WarRelationRow {
                first_country,
                second_country,
                cancel: 2,
                first_was_instigator: 255,
                ..war_relation()
            };
            let row = expected.clone();
            assert_eq!(
                row.key(),
                (u128::from(first_country) << 32) | u128::from(second_country)
            );
            keys.push(row.key());
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let mut fields = Decoder::new(&bytes);
            assert_eq!(fields.varint().unwrap(), row.key());
            assert_eq!(fields.remaining(), 42);
            assert_eq!(fields.i32().unwrap(), expected.start_hour);
            assert_eq!(fields.i32().unwrap(), expected.end_hour);
            assert_eq!(fields.u8().unwrap(), 2);
            assert_eq!(fields.i64().unwrap(), expected.first_casualties);
            assert_eq!(fields.i64().unwrap(), expected.second_casualties);
            assert_eq!(fields.i64().unwrap(), expected.first_unknown_casualties);
            assert_eq!(fields.i64().unwrap(), expected.second_unknown_casualties);
            assert_eq!(fields.u8().unwrap(), 255);
            fields.finish().unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<WarRelationRow>(0, &mut input).unwrap(), row);
            input.finish().unwrap();
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn war_relation_masks_preserve_each_native_field_and_raw_byte_value() {
        let original = war_relation();
        let old = original.clone();
        for mask in 1..=TableId::WarRelations.full_mask() {
            let mut updated = original.clone();
            if mask & 1 != 0 {
                updated.start_hour = i32::MAX;
            }
            if mask & 2 != 0 {
                updated.end_hour = i32::MIN;
            }
            if mask & 4 != 0 {
                updated.cancel = 255;
            }
            if mask & 8 != 0 {
                updated.first_casualties = i64::MAX;
            }
            if mask & 16 != 0 {
                updated.second_casualties = i64::MIN;
            }
            if mask & 32 != 0 {
                updated.first_unknown_casualties = i64::MAX;
            }
            if mask & 64 != 0 {
                updated.second_unknown_casualties = i64::MIN;
            }
            if mask & 128 != 0 {
                updated.first_was_instigator = 2;
            }
            let new = updated;
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut wire = Vec::new();
            encode_delta(TableId::WarRelations, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let decoded = decode_delta(TableId::WarRelations, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(
                decoded.apply::<WarRelationRow>(Some(old.clone())).unwrap(),
                new
            );
            assert_eq!(
                old.delta(Some(&new))
                    .unwrap()
                    .unwrap()
                    .apply::<WarRelationRow>(Some(new.clone()))
                    .unwrap(),
                old
            );
        }
        for value in 0..=u8::MAX {
            let new = WarRelationRow {
                cancel: value,
                first_was_instigator: u8::MAX - value,
                ..original.clone()
            };
            let mut wire = Vec::new();
            encode_row(&new, 0, &mut wire).unwrap();
            assert_eq!(
                decode_row::<WarRelationRow>(0, &mut Decoder::new(&wire)).unwrap(),
                new
            );
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(
                delta.apply::<WarRelationRow>(Some(old.clone())).unwrap(),
                new
            );
        }
    }

    #[test]
    fn field_masks_handle_every_subset_and_option_transitions() {
        let old = country();
        for mask in 1_u64..=TableId::Countries.full_mask() {
            let mut new = old.clone();
            if mask & 1 != 0 {
                new.canonical_slot = 3;
            }
            if mask & 2 != 0 {
                new.tag = vec![0xfe];
            }
            if mask & 4 != 0 {
                new.name.clear();
            }
            if mask & 8 != 0 {
                new.political_power_raw = None;
            }
            if mask & 16 != 0 {
                new.command_power_raw = i64::MAX;
            }
            if mask & 32 != 0 {
                new.stability_raw = i64::MIN;
            }
            if mask & 64 != 0 {
                new.war_support_raw = -1;
            }
            if mask & 128 != 0 {
                new.convoys_raw = None;
            }
            let old = old.clone();
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            assert_eq!(delta.apply::<CountryRow>(Some(old.clone())).unwrap(), new);
            let reverse = old.delta(Some(&new)).unwrap().unwrap();
            assert_eq!(reverse.apply::<CountryRow>(Some(new.clone())).unwrap(), old);
        }
    }

    #[test]
    fn signed_byte_and_optional_u32_changes_keep_every_bit_and_presence() {
        let mut previous = HumanControllerRow {
            country: 7,
            count: i8::MIN,
        };
        for count in (i8::MIN..=i8::MAX).rev() {
            let next = HumanControllerRow { country: 7, count };
            if let Some(delta) = next.delta(Some(&previous)).unwrap() {
                assert_eq!(
                    delta
                        .apply::<HumanControllerRow>(Some(previous.clone()))
                        .unwrap(),
                    next
                );
            }
            previous = next;
        }
        let mut previous = AirWingRow::default();
        for region in [Some(u32::MAX), Some(0), None, Some(0), Some(u32::MAX), None] {
            let next = AirWingRow {
                mission_region: region,
                ..AirWingRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 7);
            assert_eq!(
                delta.apply::<AirWingRow>(Some(previous.clone())).unwrap(),
                next
            );
            previous = next;
        }
        for (kind, previous, difference) in [
            (Kind::I8, Field::I8(i8::MAX), 1),
            (Kind::I8, Field::I8(i8::MIN), -1),
            (Kind::OptionalU32, Field::OptionalU32(Some(u32::MAX)), 1),
            (Kind::OptionalU32, Field::OptionalU32(Some(0)), -1),
        ] {
            assert!(Change::Number(difference).apply(kind, previous).is_err());
        }
    }

    #[test]
    fn list_changes_replace_exact_order_and_refuse_truncated_or_hostile_lengths() {
        let mut previous = LandCombatRow::default();
        for attackers in [vec![u64::MAX, 1], vec![1, u64::MAX], vec![], vec![0, 0]] {
            let next = LandCombatRow {
                attackers,
                ..LandCombatRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 3);
            assert_eq!(
                delta
                    .apply::<LandCombatRow>(Some(previous.clone()))
                    .unwrap(),
                next
            );
            previous = next;
        }
        let mut previous = ArmyOrderRow::default();
        for path in [vec![u32::MAX, 1], vec![1, u32::MAX], vec![], vec![0, 0]] {
            let next = ArmyOrderRow {
                path,
                ..ArmyOrderRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 4);
            assert_eq!(
                delta.apply::<ArmyOrderRow>(Some(previous.clone())).unwrap(),
                next
            );
            previous = next;
        }
        for (kind, width) in [(Kind::U32List, 4), (Kind::U64List, 8)] {
            assert!(
                kind.decode(&mut Decoder::new(&u32::MAX.to_le_bytes()))
                    .is_err()
            );
            let mut bytes = 2_u32.to_le_bytes().to_vec();
            bytes.resize(4 + 2 * width - 1, 0);
            assert!(kind.decode_change(&mut Decoder::new(&bytes)).is_err());
        }
    }

    #[test]
    fn numeric_differences_preserve_the_full_signed_and_unsigned_spans() {
        pair(
            TableId::Clock,
            ClockRow {
                hour: i32::MIN,
                play_time: 0,
                play_session_started_at: i64::MIN,
                seed: i32::MIN,
            },
            ClockRow {
                hour: i32::MAX,
                play_time: u64::MAX,
                play_session_started_at: i64::MAX,
                seed: i32::MAX,
            },
        );
        pair(
            TableId::Divisions,
            DivisionRow {
                template: 0,
                hit_points: i64::MIN,
                organisation: i64::MAX,
                ..division()
            },
            DivisionRow {
                template: u64::MAX,
                hit_points: i64::MAX,
                organisation: i64::MIN,
                ..division()
            },
        );
    }

    #[test]
    fn numeric_changes_refuse_out_of_range_differences_and_reconstructed_values() {
        fn reject<T: RowCodec>(table: TableId, previous: T, mask: u64, values: Vec<u8>) {
            let change = DeltaRow {
                key: previous.key(),
                mask,
                values,
            };
            let mut bytes = Vec::new();
            encode_delta(table, &change, 0, &mut bytes).unwrap();
            assert!(change.apply(Some(previous.clone())).is_err());
            let restored = decode_delta(table, 0, &mut Decoder::new(&bytes)).unwrap();
            assert!(restored.apply(Some(previous)).is_err());
        }
        reject(
            TableId::Clock,
            ClockRow {
                play_time: u64::MAX,
                ..ClockRow::default()
            },
            2,
            vec![2],
        );
        reject(TableId::Clock, ClockRow::default(), 2, vec![1]);
        reject(
            TableId::Clock,
            ClockRow {
                seed: i32::MAX,
                ..ClockRow::default()
            },
            8,
            vec![2],
        );
        reject(
            TableId::Clock,
            ClockRow {
                play_session_started_at: i64::MIN,
                ..ClockRow::default()
            },
            4,
            vec![1],
        );
        reject(
            TableId::States,
            StateRow {
                owner: u32::MAX,
                ..StateRow::default()
            },
            1,
            vec![2],
        );
        reject(
            TableId::Countries,
            CountryRow {
                political_power_raw: Some(i64::MAX),
                ..country()
            },
            8,
            vec![1, 2],
        );
        reject(
            TableId::WarRelations,
            WarRelationRow {
                cancel: u8::MAX,
                ..war_relation()
            },
            4,
            vec![2],
        );
        reject(TableId::WarRelations, war_relation(), 128, vec![1]);
        for (table, mask, difference) in [
            (TableId::WarRelations, 4, 1_u128 << 8),
            (TableId::States, 1, 1_u128 << 32),
            (TableId::Clock, 2, 1_u128 << 64),
        ] {
            let mut values = Vec::new();
            put_varint(&mut values, difference * 2);
            let delta = DeltaRow {
                key: 0,
                mask,
                values: values.clone(),
            };
            assert!(encode_delta(table, &delta, 0, &mut Vec::new()).is_err());
            let mut wire = vec![0, mask as u8];
            wire.extend(values);
            assert!(decode_delta(table, 0, &mut Decoder::new(&wire)).is_err());
        }
    }

    #[test]
    fn native_composite_keys_preserve_full_handles_side_and_slot() {
        let order = ArmyOrderRow {
            owner_group: u64::MAX,
            instance: u32::MAX,
            ..ArmyOrderRow::default()
        };
        let participant = NavalCombatParticipantRow {
            combat: u64::MAX,
            side: 1,
            battle_slot: u32::MAX,
            ..NavalCombatParticipantRow::default()
        };
        assert_eq!(order.key(), (1_u128 << 96) - 1);
        assert_eq!(participant.key(), (1_u128 << 97) - 1);
        round_trip(TableId::ArmyOrders, order);
        round_trip(TableId::NavalCombatParticipants, participant);
        let invalid = NavalCombatParticipantRow {
            side: 2,
            ..NavalCombatParticipantRow::default()
        };
        assert!(invalid.delta(None).is_err());
        assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
        let attacker = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0001,
            battle_slot: u32::MAX,
            ..NavalCombatParticipantRow::default()
        };
        let defender = NavalCombatParticipantRow {
            side: 1,
            ..attacker.clone()
        };
        let other_type = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0002,
            ..attacker.clone()
        };
        assert_ne!(attacker.key(), defender.key());
        assert_ne!(attacker.key(), other_type.key());
    }

    #[test]
    fn native_key_widths_full_width_guard_and_masks_refuse_overflow() {
        assert!(check_native_key(u128::MAX, 128).is_ok());
        assert_eq!(full_mask(64), u64::MAX);
        for table in TableId::ALL {
            let key = if table.key_bits() == 128 {
                u128::MAX
            } else {
                (1_u128 << table.key_bits()) - 1
            };
            let mut bytes = Vec::new();
            table.encode_key(key, 0, &mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                if key == 0 {
                    1
                } else {
                    table.key_bits().div_ceil(7) as usize
                }
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(table.decode_key(0, &mut input).unwrap(), key);
            input.finish().unwrap();
            if table.key_bits() < 128 {
                for oversized in [1_u128 << table.key_bits(), u128::MAX] {
                    let before = bytes.clone();
                    assert!(table.encode_key(oversized, 0, &mut bytes).is_err());
                    assert_eq!(bytes, before);
                }
            }
            let mut one = Vec::new();
            put_varint(&mut one, 1);
            assert!(
                table
                    .decode_key(u128::MAX, &mut Decoder::new(&one))
                    .is_err()
            );
        }
    }

    #[test]
    fn malformed_deltas_cannot_invent_baselines_or_ignore_suffixes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        let valid = new.delta(Some(&old)).unwrap().unwrap();
        assert!(valid.apply::<CountryRow>(None).is_err());
        let other = CountryRow {
            id: 99,
            ..country()
        };
        assert!(valid.apply(Some(other)).is_err());
        for mask in [0, 1 << 8] {
            let bad = DeltaRow {
                mask,
                ..valid.clone()
            };
            assert!(bad.apply(Some(old.clone())).is_err());
            assert!(encode_delta(TableId::Countries, &bad, 0, &mut Vec::new()).is_err());
        }
        let mut suffix = valid.clone();
        suffix.values.push(0);
        assert!(suffix.apply(Some(old.clone())).is_err());
        assert!(encode_delta(TableId::Countries, &suffix, 0, &mut Vec::new()).is_err());
        let mut missing = valid;
        missing.values.clear();
        assert!(missing.apply(Some(old)).is_err());
        assert!(decode_row::<ClockRow>(0, &mut Decoder::new(&[1; 25])).is_err());
        assert!(decode_row::<CampaignRow>(0, &mut Decoder::new(&[1, 0, 0, 0, 0])).is_err());
    }

    #[test]
    fn scalar_deltas_move_unchanged_strings_and_ordered_lists() {
        let old = country();
        let tag = old.tag.as_ptr();
        let name = old.name.as_ptr();
        let mut next = old.clone();
        next.command_power_raw += 1;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        let restored = delta.apply(Some(old)).unwrap();
        assert_eq!(restored, next);
        assert_eq!(restored.tag.as_ptr(), tag);
        assert_eq!(restored.name.as_ptr(), name);
        let old = LandCombatRow {
            attackers: vec![u64::MAX, 0, 7],
            defenders: vec![9, 1],
            ..LandCombatRow::default()
        };
        let attackers = old.attackers.as_ptr();
        let defenders = old.defenders.as_ptr();
        let mut next = old.clone();
        next.duration = 1;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        let restored = delta.apply(Some(old)).unwrap();
        assert_eq!(restored, next);
        assert_eq!(restored.attackers.as_ptr(), attackers);
        assert_eq!(restored.defenders.as_ptr(), defenders);
    }
}
