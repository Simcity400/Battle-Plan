pub use crate::checkpoint::endpoint::EndpointRows;
pub use replay_rows::rows::ops;
pub use replay_rows::rows::*;
pub use replay_rows_world::*;
pub use replay_rows_events::*;
pub use replay_rows_air::*;
pub use replay_rows_naval::*;
pub use replay_rows_weather::*;
pub use replay_rows_land::*;
pub use replay_rows_presentation::*;
pub use replay_rows_wars::*;
pub use replay_rows_economy::*;
pub use replay_rows_projects::*;
pub use replay_rows_intelligence::*;
pub use replay_rows_politics::*;
pub use replay_rows_derived::*;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_rows {
    pub use replay_rows::rows::test_rows::*;
    pub use replay_rows_land::test_rows::*;
}
use crate::codec::{Decoder, invalid_static as invalid, put_varint};
use crate::display::Step;
use engine_allocation as allocation;
use std::{fmt, io};
#[cfg(any(test, feature = "test-support"))]
#[expect(unused_imports, reason = "the tests and fixtures below were written against the whole scope of the rows module, now partly in replay-rows")]
use crate::{
    codec::put_bytes,
    display::Rounding,
    format::FormatVersion,
    measurement::{self, Counter},
    value::{EncodeValue, Value},
};

pub const LOGS: [TableId; 16] = [
    TableId::CombatLogBattleModifiers,
    TableId::OperationCompletions,
    TableId::PeaceSettlements,
    TableId::LandCombatResults,
    TableId::DestroyedDivisions,
    TableId::NavalResultSources,
    TableId::AirLossEvents,
    TableId::BuildingBombingDamage,
    TableId::NuclearStrikes,
    TableId::MarketEvents,
    TableId::VariantChanges,
    TableId::DivisionEvents,
    TableId::LeaderBattles,
    TableId::LeaderTraitGains,
    TableId::LeaderCommands,
    TableId::PowerSpendDays,
];

pub fn logs() -> [bool; TABLE_COUNT] {
    let mut logs = [false; TABLE_COUNT];
    for table in LOGS {
        logs[table.index()] = true;
    }
    logs
}
pub const TABLE_COUNT: usize = TableId::ALL.len();

pub trait TableSchema {
    type Row: RowCodec;
    const ID: TableId;
    fn borrow(rows: &TableRows) -> Option<&[Self::Row]>;
    fn borrow_mut(rows: &mut TableRows) -> Option<&mut Vec<Self::Row>>;
    fn take(rows: TableRows) -> Result<Vec<Self::Row>, TableRows>;
    fn wrap(rows: Vec<Self::Row>) -> TableRows;
}

pub mod compatibility;
pub mod restricted_changes;
mod words;

struct Shown<'a, T>(&'a T, fn(&T, &mut fmt::Formatter<'_>) -> fmt::Result);
impl<T> fmt::Debug for Shown<'_, T> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.1)(self.0, out)
    }
}

macro_rules! define_tables {
    (@acc [$($all:tt)*] [$($derived:ident)*]) => {
        define_tables! { $($all)* }
        impl TableId {
            pub const fn is_derived(self) -> bool {
                matches!(self, $(Self::$derived)|*)
            }
        }
    };
    (@acc [$($all:tt)*] [$($derived:ident)*] recorded { $($t:ident = $id:literal => $v:ident($r:ident)),+ $(,)? } $($rest:tt)*) => {
        define_tables! { @acc [$($all)* $($t = $id => $v($r),)+] [$($derived)*] $($rest)* }
    };
    (@acc [$($all:tt)*] [$($derived:ident)*] derived { $($t:ident = $id:literal => $v:ident($r:ident)),+ $(,)? } $($rest:tt)*) => {
        define_tables! { @acc [$($all)* $($t = $id => $v($r),)+] [$($derived)* $($t)+] $($rest)* }
    };
    (recorded { $($first:tt)* } $($rest:tt)*) => {
        define_tables! { @acc [] [] recorded { $($first)* } $($rest)* }
    };
    ($($table:ident = $id:literal => $variant:ident($row:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
        #[repr(u16)]
        pub enum TableId { $($table = $id),+ }
        pub enum TableRows { $($table(Vec<$row>)),+ }
        #[derive(Clone, Copy, Eq)]
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
            pub const fn index(self) -> usize {
                enum Index { $($table),+ }
                match self { $(Self::$table => Index::$table as usize),+ }
            }
            pub const fn schema(self) -> &'static [Kind] {
                match self { $(Self::$table => <$row as Fields>::SCHEMA),+ }
            }
            pub const fn key_bits(self) -> u32 {
                match self { $(Self::$table => <$row as RowCodec>::KEY_BITS),+ }
            }
            pub fn display_step(self, field: usize) -> Option<Step> {
                let steps: &[Option<Step>] = match self { $(Self::$table => <$row as Fields>::STEPS),+ };
                steps.get(field).copied().flatten()
            }
            pub fn field_name(self, field: usize) -> Option<&'static str> {
                let names: &[&str] = match self { $(Self::$table => <$row as Fields>::NAMES),+ };
                names.get(field).copied()
            }
            pub fn decode_rows(self, count: usize, input: &mut Decoder<'_>) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table((crate::ops::$table.decode)(count, input)?)),+ })
            }
            pub fn rows_from_wire<'a>(
                self,
                keys: &[u128],
                wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
            ) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table((crate::ops::$table.from_wire)(keys, wire)?)),+ })
            }
            pub fn rows_from_wire_fields<'a>(
                self,
                keys: &[u128],
                mask: u64,
                wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
            ) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table((crate::ops::$table.from_wire_fields)(keys, mask, wire)?)),+ })
            }
            pub fn decode_checkpoint_rows(self, input: &mut Decoder<'_>) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table((crate::ops::$table.checkpoint_decode)(input)?)),+ })
            }
            pub fn decode_endpoint_rows(self, input: &mut Decoder<'_>) -> io::Result<EndpointRows> {
                Ok(match self { $(Self::$table => {
                    let (rows, edits) = (crate::ops::$table.endpoint_decode)(input)?;
                    EndpointRows { rows: TableRows::$table(rows), edits }
                }),+ })
            }
        }
        impl Clone for TableRows {
            fn clone(&self) -> Self {
                match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.clone)(rows))),+ }
            }
        }
        impl PartialEq for TableRows {
            fn eq(&self, other: &Self) -> bool {
                match self { $(Self::$table(rows) => match other {
                    Self::$table(other) => (crate::ops::$table.eq)(rows, other),
                    _ => false,
                }),+ }
            }
        }
        impl Eq for TableRows {}
        impl fmt::Debug for TableRows {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$table(rows) => out
                    .debug_tuple(stringify!($table))
                    .field(&Shown(rows, crate::ops::$table.debug))
                    .finish()),+ }
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        impl replay_spool::Spooled for TableRows {
            fn spool(&self, out: &mut replay_spool::Writer<'_>) -> io::Result<()> {
                match self { $(Self::$table(rows) => {
                    replay_spool::Writer::tag(out, TableId::$table.index())?;
                    (crate::ops::fixtures::$table.spool)(rows, out)
                }),+ }
            }
            fn unspool(input: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
                let tag = replay_spool::Reader::tag(input)?;
                match TableId::ALL.get(tag) {
                    $(Some(TableId::$table) => Ok(Self::$table((crate::ops::fixtures::$table.unspool)(input)?)),)+
                    None => Err(replay_spool::invalid("A spooled variant it does not know")),
                }
            }
        }
        impl PartialEq for RowRef<'_> {
            fn eq(&self, other: &Self) -> bool {
                match self { $(Self::$variant(row) => match other {
                    Self::$variant(other) => (crate::ops::$table.row_eq)(row, other),
                    _ => false,
                }),+ }
            }
        }
        impl fmt::Debug for RowRef<'_> {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$variant(row) => out
                    .debug_tuple(stringify!($variant))
                    .field(&Shown(*row, crate::ops::$table.row_debug))
                    .finish()),+ }
            }
        }
        impl TableRows {
            pub(crate) fn range_update_fits(&self, previous: &TableRows, ranges: &[std::ops::Range<u128>]) -> bool {
                match self { $(Self::$table(fresh) => match previous {
                    Self::$table(old) => (crate::ops::$table.range_fits)(old, fresh, ranges),
                    _ => false,
                }),+ }
            }
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            pub(crate) fn encode_range_update(&self, previous: &TableRows, ranges: &[std::ops::Range<u128>], output: &mut Vec<u8>, deleted: &mut Vec<u8>, changed: Option<&mut ChangedRows>) -> io::Result<bool> {
                match self { $(Self::$table(fresh) => match previous {
                    Self::$table(old) => (crate::ops::$table.range_encode)(old, fresh, ranges, output, deleted, changed),
                    _ => Err(invalid("key range table type mismatch")),
                }),+ }
            }
            pub(crate) fn apply_range_update(self, previous: &mut TableRows, ranges: &[std::ops::Range<u128>]) -> io::Result<TableRows> {
                match self { $(Self::$table(fresh) => match previous {
                    Self::$table(old) => (crate::ops::$table.range_apply)(old, fresh, ranges).map(Self::$table),
                    _ => Err(invalid("key range table type mismatch")),
                }),+ }
            }
            pub fn visit<V: RowsVisit>(&self, visit: V) -> V::Output {
                match self { $(Self::$table(rows) => visit.rows(rows)),+ }
            }
            pub fn visit_pair<V: RowsPairVisit>(&self, other: &TableRows, visit: V) -> Result<V::Output, V> {
                match self { $(Self::$table(rows) => match other {
                    Self::$table(others) => Ok(visit.rows(rows, others)),
                    _ => Err(visit),
                }),+ }
            }
            pub fn inline_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => rows.len() * std::mem::size_of::<$row>()),+ }
            }
            pub fn allocated_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => rows.capacity() * std::mem::size_of::<$row>()),+ }
            }
            pub fn heap_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => (crate::ops::$table.heap_bytes)(rows)),+ }
            }
            pub fn truncate(&mut self, len: usize) {
                match self { $(Self::$table(rows) => (crate::ops::$table.truncate)(rows, len)),+ }
            }
            pub fn round_to_display(&mut self) {
                match self { $(Self::$table(rows) => (crate::ops::$table.round_to_display)(rows)),+ }
            }
            pub fn from_rows<T: TableSchema>(rows: Vec<T::Row>) -> Self { T::wrap(rows) }
            pub fn empty(table: TableId) -> Self {
                match table { $(TableId::$table => Self::$table(Vec::new())),+ }
            }
            #[cfg(any(test, feature = "test-support"))]
            pub fn one_default(table: TableId) -> Self {
                match table { $(TableId::$table => Self::$table((crate::ops::fixtures::$table.one_default)())),+ }
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
            pub fn contains_key(&self, key: u128) -> bool {
                match self { $(Self::$table(rows) => (crate::ops::$table.contains_key)(rows, key)),+ }
            }

            pub fn first(&self) -> Option<RowRef<'_>> { self.iter().next() }
            pub fn validate_keys(&self) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.validate)(rows)),+ }
            }
            pub fn try_clone(&self) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.try_clone)(rows)?)),+ })
            }
            pub fn try_clone_validated(&self) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.try_clone_validated)(rows)?)),+ })
            }
            pub fn recorded_rows_bytes(&self) -> io::Result<usize> {
                match self { $(Self::$table(rows) => (crate::ops::$table.recorded_bytes)(rows)),+ }
            }
            pub fn encode_rows(&self, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.encode)(rows, output)),+ }
            }
            pub fn encode_row_fields(&self, index: usize, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.encode_row_fields)(rows, index, output)),+ }
            }
            pub fn encode_row_recorded_fields(&self, index: usize, output: &mut Vec<u8>) -> io::Result<u64> {
                match self { $(Self::$table(rows) => (crate::ops::$table.encode_row_recorded_fields)(rows, index, output)),+ }
            }
            pub fn encode_changes(&self, previous: &Self, output: &mut Vec<u8>, deleted: &mut Vec<u8>) -> io::Result<bool> {
                match self { $(Self::$table(rows) => match previous {
                    Self::$table(previous) => (crate::ops::$table.encode_changes)(rows, previous, output, deleted, None),
                    _ => Err(invalid("table rows have different types")),
                }),+ }
            }
            pub fn encode_changes_recorded(&self, previous: &Self, output: &mut Vec<u8>, deleted: &mut Vec<u8>, changed: &mut ChangedRows) -> io::Result<bool> {
                match self { $(Self::$table(rows) => match previous {
                    Self::$table(previous) => (crate::ops::$table.encode_changes)(rows, previous, output, deleted, Some(changed)),
                    _ => Err(invalid("table rows have different types")),
                }),+ }
            }
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            pub fn encode_replacements(&self, previous: &Self, indices: &[usize], output: &mut Vec<u8>) -> io::Result<bool> {
                match self { $(Self::$table(rows) => match previous {
                    Self::$table(previous) => (crate::ops::$table.encode_replacements)(rows, previous, indices, output),
                    _ => Err(invalid("replacement rows have different types")),
                }),+ }
            }
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            pub fn install_replacements(&mut self, mut replacement: Self, indices: &[usize]) -> Self {
                match self { $(Self::$table(previous) => match &mut replacement {
                    Self::$table(rows) => (crate::ops::$table.install_replacements)(previous, rows, indices),
                    _ => unreachable!("validated replacement type"),
                }),+ }
                replacement
            }
            pub fn apply_delta(self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.apply_delta)(rows, changed, deleted)?)),+ })
            }
            pub fn apply_delta_validated(self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.apply_delta_validated)(rows, changed, deleted)?)),+ })
            }
            pub fn apply_delta_shared(&self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.apply_delta_shared)(rows, changed, deleted)?)),+ })
            }
            pub fn apply_delta_shared_validated(&self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table((crate::ops::$table.apply_delta_shared_validated)(rows, changed, deleted)?)),+ })
            }
            pub fn changes_since(&self, previous: &Self) -> io::Result<Option<RowChanges>> {
                Ok(match self { $(Self::$table(rows) => match previous {
                    Self::$table(previous) => {
                        let (changed, deleted) = (crate::ops::$table.changes_since)(rows, previous)?;
                        Some(RowChanges { rows: Self::$table(changed), deleted })
                    }
                    _ => None,
                }),+ })
            }
            pub fn rows_at(&self, keys: &[u128], deleted: Vec<u128>) -> io::Result<RowChanges> {
                Ok(match self { $(Self::$table(rows) => RowChanges {
                    rows: Self::$table((crate::ops::$table.rows_at)(rows, keys)?),
                    deleted,
                }),+ })
            }
            pub fn apply_changes(mut self, changes: RowChanges) -> io::Result<Self> {
                let RowChanges { rows: mut changed, deleted } = changes;
                match &mut self { $(Self::$table(rows) => {
                    let Self::$table(changed) = &mut changed else {
                        return Err(invalid("row changes of another table"));
                    };
                    *rows = (crate::ops::$table.apply_changes)(std::mem::take(rows), std::mem::take(changed), &deleted)?;
                }),+ }
                Ok(self)
            }
            pub fn patch_from_wire<'a>(
                &mut self,
                patches: &[(u128, u64)],
                wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
            ) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.patch)(rows, patches, wire)),+ }
            }
            pub fn encode_checkpoint_rows(&self, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.checkpoint_encode)(rows, output)),+ }
            }
            pub fn encode_endpoint_rows(
                &self,
                edits: &[(u128, u64, bool)],
                out: &mut Vec<u8>,
            ) -> io::Result<()> {
                match self { $(Self::$table(rows) => (crate::ops::$table.endpoint_encode)(rows, edits, out)),+ }
            }
            pub fn apply_endpoint_rows(mut self, edits: EndpointRows, deleted: &[u128]) -> io::Result<Self> {
                let EndpointRows { rows: mut edited, edits } = edits;
                match &mut self { $(Self::$table(before) => {
                    let Self::$table(rows) = &mut edited else {
                        return Err(invalid("endpoint row table mismatch"));
                    };
                    *before = (crate::ops::$table.endpoint_apply)(std::mem::take(before), std::mem::take(rows), edits, deleted)?;
                }),+ }
                Ok(self)
            }
            #[cfg(test)]
            pub(crate) fn check_endpoint_sparse_masks(&self) {
                match self { $(Self::$table(rows) => (crate::ops::fixtures::$table.sparse_masks)(rows)),+ }
            }
            #[cfg(test)]
            pub(crate) fn endpoint_field_round_trip(&self, mask: u64) -> io::Result<bool> {
                match self { $(Self::$table(rows) => (crate::ops::fixtures::$table.field_round_trip)(rows, mask)),+ }
            }
        }
        impl RowRef<'_> {
            pub fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                match self { $(Self::$variant(row) => row.visit_nested_field_gaps(visit)),+ }
            }
            pub fn visit_wire(
                &self,
                visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
            ) -> io::Result<()> {
                match self { $(Self::$variant(row) => row.visit_wire(visit)),+ }
            }
            pub fn not_recorded_fields(&self) -> u64 {
                match self { $(Self::$variant(row) => row.not_recorded_fields()),+ }
            }
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
recorded {
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
CountryTacticUnlocks = 32 => CountryTacticUnlock(CountryTacticUnlockRow),
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
WeatherRegions = 118 => WeatherRegion(WeatherRegionRow),
WeatherProvinces = 119 => WeatherProvince(WeatherProvinceRow),
OperationTokens = 120 => OperationTokens(OperationTokensRow),
LexicalTokens = 121 => LexicalToken(LexicalTokenRow),
NavalSupremacy = 122 => NavalSupremacy(NavalSupremacyRow),
RadarCoverage = 123 => RadarCoverage(RadarCoverageRow),
VolunteerHosts = 124 => VolunteerHost(VolunteerHostRow),
TheatreAreas = 125 => TheatreArea(TheatreAreaRow),
StationedUnits = 126 => StationedUnit(StationedUnitRow),
UnitIntel = 127 => UnitIntel(UnitIntelRow),
AirBaseRegistrations = 128 => AirBaseRegistration(AirBaseRegistrationRow),
ProductionLicenses = 129 => ProductionLicense(ProductionLicenseRow),
DeploymentConveyors = 130 => DeploymentConveyor(DeploymentConveyorRow),
DeploymentLines = 131 => DeploymentLine(DeploymentLineRow),
DeploymentManpower = 132 => DeploymentManpower(DeploymentManpowerRow),
DeploymentEquipment = 133 => DeploymentEquipment(DeploymentEquipmentRow),
WorldTension = 134 => WorldTension(WorldTensionRow),
ThreatSources = 135 => ThreatSource(ThreatSourceRow),
ThreatCountryAggregates = 136 => ThreatCountryAggregate(ThreatCountryAggregateRow),
ProductionLicenseRecords = 137 => ProductionLicenseRecord(ProductionLicenseRecordRow),
OrdersGroups = 138 => OrdersGroup(OrdersGroupRow),
OrdersGroupMembers = 139 => OrdersGroupMember(OrdersGroupMemberRow),
ArmyGroupMembers = 140 => ArmyGroupMember(ArmyGroupMemberRow),
IncomingDiplomaticActions = 141 => IncomingDiplomaticAction(IncomingDiplomaticActionRow),
Scientists = 142 => Scientist(ScientistRow),
ScientistSkills = 143 => ScientistSkill(ScientistSkillRow),
RelationOpinions = 144 => RelationOpinion(RelationOpinionRow),
RelationTuples = 145 => RelationTuple(RelationTupleRow),
CountryIntel = 146 => CountryIntel(CountryIntelRow),
ProvinceNames = 147 => ProvinceNames(ProvinceNameRow),
LandCombatProgress = 148 => LandCombatProgress(LandCombatProgressRow),
DivisionTemplates = 149 => DivisionTemplates(DivisionTemplateRow),
DivisionMovement = 150 => DivisionMovement(DivisionMovementRow),
DivisionStatus = 151 => DivisionStatus(DivisionStatusRow),
StateFlags = 152 => StateFlag(StateFlagRow),
ArmyOrderPlans = 153 => ArmyOrderPlan(ArmyOrderPlanRow),
OrdersGroupPlans = 154 => OrdersGroupPlan(OrdersGroupPlanRow),
Fronts = 155 => Front(FrontRow),
CountryAppearance = 156 => CountryAppearance(CountryAppearanceRow),
ArmyIconInputs = 157 => ArmyIconInput(ArmyIconInputRow),
ShipModels = 158 => ShipModel(ShipModelRow),
SupplyNodes = 159 => SupplyNode(SupplyNodeRow),
TrainInventory = 160 => TrainInventory(TrainInventoryRow),
LandCombatDetails = 161 => LandCombatDetails(LandCombatDetailsRow),
LandCombatDivisions = 162 => LandCombatDivisions(LandCombatDivisionRow),
LandCombatModifiers = 163 => LandCombatModifiers(LandCombatModifierRow),
NavalCombatDetails = 164 => NavalCombatDetails(NavalCombatDetailsRow),
NavalCombatShips = 165 => NavalCombatShips(NavalCombatShipRow),
NavalCombatResultDetails = 166 => NavalCombatResultDetails(NavalCombatResultDetailsRow),
ProvinceUnitOrder = 167 => ProvinceUnitOrder(ProvinceUnitOrderRow),
CharacterPortraits = 168 => CharacterPortrait(CharacterPortraitRow),
LeaderPortraits = 169 => LeaderPortrait(LeaderPortraitRow),
TaskForceCounterInputs = 170 => TaskForceCounterInput(TaskForceCounterInputRow),
FleetCounterInputs = 171 => FleetCounterInput(FleetCounterInputRow),
CountryCounterInputs = 172 => CountryCounterInput(CountryCounterInputRow),
OrdersGroupCounterInputs = 173 => OrdersGroupCounterInput(OrdersGroupCounterInputRow),
LeaderPortraitPaths = 174 => LeaderPortraitPath(LeaderPortraitPathRow),
DivisionEtaInputs = 175 => DivisionEtaInputs(DivisionEtaInputsRow),
RaidTargets = 176 => RaidTarget(RaidTargetRow),
NavalBases = 177 => NavalBases(NavalBaseRow),
ResearchInputs = 178 => ResearchInputs(ResearchInputRow),
ResearchAppliedBonuses = 179 => ResearchAppliedBonuses(ResearchAppliedBonusRow),
ResearchModifiers = 180 => ResearchModifiers(ResearchModifierRow),
SimulationFeatures = 181 => SimulationFeatures(SimulationFeatureRow),
CountryVisibility = 182 => CountryVisibility(CountryVisibilityRow),
StateVisibility = 183 => StateVisibility(StateVisibilityRow),
RelationLinks = 184 => RelationLinks(RelationLinkRow),
SliceReads = 185 => SliceReads(SliceReadRow),
DivisionTemplateIcons = 186 => DivisionTemplateIcons(DivisionTemplateIconRow),
FactionTheatres = 187 => FactionTheatres(FactionTheatreRow),
RailwayGunCounterInputs = 188 => RailwayGunCounterInput(RailwayGunCounterInputRow),
CountryLocalSubunits = 189 => CountryLocalSubunits(CountryLocalSubunitRow),
AvailableEquipmentVariants = 190 => AvailableEquipmentVariants(AvailableEquipmentVariantRow),
NewDivisionEquipment = 191 => NewDivisionEquipment(DivisionEquipmentRow),
ModifierDefinitions = 192 => ModifierDefinition(ModifierDefinitionRow),
HiredAdvisors = 193 => HiredAdvisor(HiredAdvisorRow),
CountryStabilityInputs = 194 => CountryStabilityInput(CountryStabilityInputRow),
CountryStabilityLines = 195 => CountryStabilityLine(CountryStabilityLineRow),
CountryFuelConsumers = 196 => CountryFuelConsumer(CountryFuelConsumerRow),
SentVolunteers = 197 => SentVolunteer(SentVolunteerRow),
RaidTargetStatuses = 198 => RaidTargetStatus(RaidTargetStatusRow),
RaidCooldowns = 199 => RaidCooldowns(RaidCooldownRow),
RaidPathDistances = 200 => RaidPathDistances(RaidPathDistanceRow),
RaidCountryInputs = 201 => RaidCountryInput(RaidCountryInputRow),
StockpileArchetypes = 202 => StockpileArchetype(StockpileArchetypeRow),
RaidAirSites = 203 => RaidAirSite(RaidAirSiteRow),
AceRangeInputs = 204 => AceRangeInput(AceRangeInputRow),
StateResourceInputs = 205 => StateResourceInput(StateResourceInputRow),
StateResources = 206 => StateResource(StateResourceRow),
StateDynamicModifiers = 207 => StateDynamicModifier(StateDynamicModifierRow),
CountryBuildingResources = 208 => CountryBuildingResource(CountryBuildingResourceRow),
CountryWars = 209 => CountryWars(CountryWarRow),
NavalRegionAccess = 210 => NavalRegionAccess(NavalRegionAccessRow),
BorderConflicts = 211 => BorderConflicts(BorderConflictRow),
RaidPlayerAllowedTypes = 212 => RaidPlayerAllowedTypes(RaidPlayerAllowedTypeRow),
CountryCommandPower = 213 => CountryCommandPower(CountryCommandPowerRow),
DivisionRetreatBans = 214 => DivisionRetreatBans(DivisionRetreatBanRow),
StateCores = 215 => StateCores(StateCoreRow),
RaidSpeedInputs = 216 => RaidSpeedInput(RaidSpeedInputRow),
RaidEquipment = 217 => RaidEquipment(RaidEquipmentRow),
InvasionPreparations = 218 => InvasionPreparation(InvasionPreparationRow),
CountryCharacters = 219 => CountryCharacter(CountryCharacterRow),
PartyLeaders = 220 => PartyLeader(PartyLeaderRow),
CountryCivilWars = 221 => CountryCivilWar(CountryCivilWarRow),
AdvisorTokens = 222 => AdvisorToken(AdvisorTokenRow),
FactionAdvisors = 223 => FactionAdvisor(FactionAdvisorRow),
LandCombatSides = 224 => LandCombatSide(LandCombatSideRow),
AirRegionalMissions = 225 => AirRegionalMissions(AirRegionalMissionRow),
AirCountryRelations = 226 => AirCountryRelations(AirCountryRelationRow),
AirReferenceWitnesses = 227 => AirReferenceWitnesses(AirReferenceWitnessRow),
NavalMineLevels = 228 => NavalMineLevels(NavalMineLevelRow),
AirMissionInputs = 229 => AirMissionInputs(AirMissionInputRow),
AirSortieBaseCountries = 230 => AirSortieBaseCountries(AirSortieBaseCountryRow),
AirSortieModifiers = 231 => AirSortieModifiers(AirSortieModifierRow),
AirSortieSpecialBases = 232 => AirSortieSpecialBases(AirSortieSpecialBaseRow),
AirSortieModifierMetadata = 233 => AirSortieModifierMetadata(AirSortieModifierMetadataRow),
LocalPlayer = 234 => LocalPlayer(LocalPlayerRow),
AirNavalPatrolCombats = 235 => AirNavalPatrolCombats(AirNavalPatrolCombatRow),
AirNavalPatrolHistories = 236 => AirNavalPatrolHistories(AirNavalPatrolHistoryRow),
RaidFacilities = 237 => RaidFacility(RaidFacilityRow),
RaidAirScores = 238 => RaidAirScore(RaidAirScoreRow),
RaidIntelInputs = 239 => RaidIntelInput(RaidIntelInputRow),
RaidTooltipOperands = 240 => RaidTooltipOperand(RaidTooltipOperandRow),
TaskForceMapMotion = 241 => TaskForceMapMotion(TaskForceMapMotionRow),
FloatingHarborDeployments = 242 => FloatingHarborDeployments(FloatingHarborDeploymentRow),
FacilityMapFlags = 243 => FacilityMapFlags(FacilityMapFlagRow),
CountryRetiredCharacters = 244 => CountryRetiredCharacter(CountryCharacterRow),
CharacterRoleTraits = 245 => CharacterRoleTrait(CharacterRoleTraitRow),
CharacterOperatives = 246 => CharacterOperative(CharacterOperativeRow),
NavalProvinceCombats = 247 => NavalProvinceCombats(NavalProvinceCombatRow),
NavalCombatTransfers = 248 => NavalCombatTransfers(NavalCombatTransferRow),
NavalTransferUnits = 249 => NavalTransferUnits(NavalTransferUnitRow),
TaskForceMissions = 250 => TaskForceMissions(TaskForceMissionRow),
NavalCombatAirBaseWingHandles = 251 => NavalCombatAirBaseWingHandles(NavalCombatAirBaseWingHandlesRow),
CountryFactories = 252 => CountryFactories(CountryFactoriesRow),
NavalMissionTaskForces = 253 => NavalMissionTaskForce(NavalMissionTaskForceRow),
NavalMissionLedgers = 254 => NavalMissionLedger(NavalMissionLedgerRow),
NavalMissionCountries = 255 => NavalMissionCountry(NavalMissionCountryRow),
NavalMissionCommanders = 256 => NavalMissionCommander(NavalMissionCommanderRow),
GlobalSavedEventTargets = 257 => GlobalSavedEventTargets(GlobalSavedEventTargetRow),
ReserveModifierBreakdowns = 258 => ReserveModifierBreakdowns(ReserveModifierBreakdownRow),
PowerBalanceModifierBreakdowns = 259 => PowerBalanceModifierBreakdowns(PowerBalanceModifierBreakdownRow),
LogisticsEquipment = 260 => LogisticsEquipment(LogisticsEquipmentRow),
IndustrialOrganisations = 261 => IndustrialOrganisation(IndustrialOrganisationRow),
IndustrialOrganisationHistory = 262 => IndustrialOrganisationHistoryEntry(IndustrialOrganisationHistoryRow),
ProductionLineOutput = 263 => ProductionLineOutput(ProductionLineOutputRow),
ConstructionLineOutput = 264 => ConstructionLineOutput(ConstructionLineOutputRow),
CombatLogEquipment = 265 => CombatLogEquipment(CombatLogEquipmentRow),
CombatLogState = 266 => CombatLogState(CombatLogStateRow),
LendLeaseEquipment = 267 => LendLeaseEquipment(LendLeaseEquipmentRow),
DivisionCensus = 268 => DivisionCensus(DivisionCensusRow),
EquipmentDestruction = 269 => EquipmentDestruction(EquipmentDestructionRow),
NavalModifierBreakdowns = 270 => NavalModifierBreakdowns(NavalModifierBreakdownRow),
AvailableAdvisors = 271 => AvailableAdvisor(AvailableAdvisorRow),
OwnedWargoals = 272 => OwnedWargoal(OwnedWargoalRow),
TimedIdeas = 273 => TimedIdea(TimedIdeaRow),
DecisionRandomItems = 274 => DecisionRandomItem(DecisionRandomItemRow),
CapturedOperativeReferences = 275 => CapturedOperativeReference(CapturedOperativeReferenceRow),
ReservedCountryNaming = 276 => ReservedCountryNaming(ReservedCountryNamingRow),
AttritionModifierBreakdowns = 277 => AttritionModifierBreakdowns(AttritionModifierBreakdownRow),
CountryProductionModifiers = 278 => CountryProductionModifiers(CountryProductionModifiersRow),
ProductionLineModifiers = 279 => ProductionLineModifiers(ProductionLineModifiersRow),
CountryExperienceGains = 280 => CountryExperienceGains(CountryExperienceGainRow),
CountryGameScoreInputs = 281 => CountryGameScoreInput(CountryGameScoreInputRow),
AirWingGameScoreStats = 282 => AirWingGameScoreStats(AirWingGameScoreStatsRow),
GameScoreMultipliers = 283 => GameScoreMultiplier(GameScoreMultiplierRow),
AgencyUpgradeLevels = 284 => AgencyUpgradeLevel(AgencyUpgradeLevelRow),
OperationCompletions = 285 => OperationCompletion(OperationCompletionRow),
CountryPowerDaily = 286 => CountryPowerDaily(CountryPowerDailyRow),
CountryPowerChanges = 287 => CountryPowerChange(CountryPowerChangeRow),
CountryPowerDailyCosts = 288 => CountryPowerDailyCost(CountryPowerDailyCostRow),
GovernmentChanges = 289 => GovernmentChange(GovernmentChangeRow),
PoliticsCaptureCoverage = 290 => PoliticsCaptureCoverage(PoliticsCaptureCoverageRow),
WarOwnedRelations = 291 => WarOwnedRelations(WarOwnedRelationRow),
WarRelationGoals = 292 => WarRelationGoals(WarRelationGoalRow),
WarOverviewCountries = 293 => WarOverviewCountries(WarOverviewCountryRow),
WarNameRules = 294 => WarNameRules(WarNameRuleRow),
PeaceSettlements = 295 => PeaceSettlements(PeaceSettlementRow),
PeaceBids = 296 => PeaceBids(PeaceSettlementRow),
DivisionEquipmentDeletions = 297 => DivisionEquipmentDeletion(DivisionEquipmentDeletionRow),
DivisionLandInputs = 298 => DivisionLandInputs(DivisionLandInputsRow),
LandCombatResults = 299 => LandCombatResults(LandCombatResultsRow),
DestroyedDivisions = 300 => DestroyedDivision(DestroyedDivisionsRow),
LandEventCoverage = 301 => LandEventCoverage(LandEventCoverageRow),
NavalResultSources = 302 => NavalResultSources(NavalResultSourceRow),
AirLossEvents = 303 => AirLossEvent(AirLossEventRow),
BuildingBombingDamage = 304 => BuildingBombingDamage(BuildingBombingDamageRow),
AirDailyRoster = 305 => AirDailyRoster(AirDailyRosterRow),
EventFeedHistory = 306 => EventFeedHistory(EventFeedHistoryRow),
RocketProductionOutput = 307 => RocketProductionOutput(RocketProductionOutputRow),
ReservedNearestNavies = 308 => ReservedNearestNavy(ReservedNearestNavyRow),
ReservedNearestBases = 309 => ReservedNearestBase(ReservedNearestBaseRow),
TransportNavies = 310 => TransportNavy(TransportNavyRow),
TransportLedgers = 311 => TransportLedger(TransportLedgerRow),
TransportReceivers = 312 => TransportReceiver(TransportReceiverRow),
TransportSearchRegions = 313 => TransportSearchRegion(TransportSearchRegionRow),
TransportSpottingOwners = 314 => TransportSpottingOwner(TransportSpottingOwnerRow),
TransportSpottingRegions = 315 => TransportSpottingRegion(TransportSpottingRegionRow),
TransportAirRoots = 316 => TransportAirRoot(TransportAirRootRow),
TransportAirRegions = 317 => TransportAirRegion(TransportAirRegionRow),
TransportRiskShips = 318 => TransportRiskShips(TransportRiskShipsRow),
NavalModifierTrees = 319 => NavalModifierTrees(NavalModifierTreeRow),
NavalModifierTreeSamples = 320 => NavalModifierTreeSamples(NavalModifierTreeSampleRow),
AirRegionCombat = 321 => AirRegionCombat(AirRegionCombatRow),
NuclearStrikes = 322 => NuclearStrike(NuclearStrikeRow),
LandCombatDivisionModifiers = 323 => LandCombatDivisionModifier(LandCombatDivisionModifierRow),
FuelDemandParts = 324 => FuelDemandPart(FuelDemandPartRow),
MapEvents = 325 => MapEvents(MapEventsRow),
OwnerTransportSpeeds = 326 => OwnerTransportSpeed(OwnerTransportSpeedRow),
ReserveModifierTrees = 327 => ReserveModifierTree(ReserveModifierTreeRow),
BattleSounds = 328 => BattleSound(BattleSoundRow),
OperativeMissions = 329 => OperativeMission(OperativeMissionRow),
StrategicOperatives = 330 => StrategicOperative(StrategicOperativeRow),
AgencyOperationBuffers = 331 => AgencyOperationBuffers(AgencyOperationBuffersRow),
ResourceOrigins = 332 => ResourceOrigin(ResourceOriginRow),
ResourceRights = 333 => ResourceRight(ResourceRightRow),
CountryOilFactors = 334 => CountryOilFactor(CountryOilFactorRow),
StateTemporaryResources = 335 => StateTemporaryResource(StateTemporaryResourceRow),
ConvoyClients = 336 => ConvoyClient(ConvoyClientRow),
TradeRoutes = 337 => TradeRoute(TradeRouteRow),
RegionalConvoyEfficiency = 338 => RegionalConvoyEfficiency(RegionalConvoyEfficiencyRow),
CountryWargoals = 339 => CountryWargoal(CountryWargoalRow),
WargoalTargets = 340 => WargoalTarget(WargoalTargetRow),
WargoalJustifications = 341 => WargoalJustification(WargoalJustificationRow),
IndustrialOrganisationDetails = 342 => IndustrialOrganisationDetails(IndustrialOrganisationDetailsRow),
IndustrialOrganisationPolicies = 343 => IndustrialOrganisationPolicies(IndustrialOrganisationPoliciesRow),
VariantChanges = 344 => VariantChange(VariantChangeRow),
VariantBonusSnapshots = 345 => VariantBonusSnapshot(VariantBonusSnapshotRow),
DesignCountryModifiers = 346 => DesignCountryModifier(DesignCountryModifierRow),
DesignCountryTemplates = 347 => DesignCountryTemplates(DesignCountryTemplatesRow),
DesignObserverContext = 348 => DesignObserverContext(DesignObserverContextRow),
DesignOrganisationLiveness = 349 => DesignOrganisationLiveness(DesignOrganisationLivenessRow),

DivisionHistory = 350 => DivisionHistory(SharedDivisionHistoryRow),
DivisionPanelSources = 351 => DivisionPanelSources(SharedDivisionPanelSourceRow),
DivisionPanelStats = 352 => DivisionPanelStats(DivisionPanelStatsRow),
DoctrineFolders = 353 => DoctrineFolder(DoctrineFolderRow),
DoctrineTracks = 354 => DoctrineTrack(DoctrineTrackRow),
DoctrineLeaderMastery = 355 => DoctrineLeaderMastery(DoctrineLeaderMasteryRow),
DoctrineCostReductions = 356 => DoctrineCostReduction(DoctrineCostReductionRow),
DoctrineMasteryGrants = 357 => DoctrineMasteryGrant(DoctrineMasteryGrantRow),
DoctrineEquipmentBonuses = 358 => DoctrineEquipmentBonus(DoctrineEquipmentBonusRow),
DoctrineCountries = 359 => DoctrineCountry(DoctrineCountryRow),
ScientistDaily = 360 => ScientistDaily(ScientistDailyRow),
FactionAccounts = 361 => FactionAccount(FactionAccountRow),
FactionResearch = 362 => FactionResearch(FactionResearchRow),
FactionMinimumUpgrades = 363 => FactionMinimumUpgrade(FactionMinimumUpgradeRow),
FactionDoctrine = 364 => FactionDoctrine(FactionDoctrineRow),
FactionProjection = 365 => FactionProjection(FactionProjectionRow),
FactionResources = 366 => FactionResource(FactionResourceRow),
FactionGoalStatus = 367 => FactionGoalStatus(FactionGoalStatusRow),
FactionGoalEntries = 368 => FactionGoalEntry(FactionGoalEntryRow),
FactionRules = 369 => FactionRule(FactionRuleRow),
FactionPrograms = 370 => FactionProgram(FactionProgramRow),
FactionManpower = 371 => FactionManpower(FactionManpowerRow),
FactionUnlocks = 372 => FactionUnlock(FactionUnlockRow),
FactionVariables = 373 => FactionVariable(FactionVariableRow),
FactionAdvisorModifiers = 374 => FactionAdvisorModifier(FactionAdvisorModifierRow),
CombatLogGroups = 375 => CombatLogGroups(CombatLogGroupRow),
CombatLogMonths = 376 => CombatLogMonths(CombatLogMonthRow),
CombatLogManpower = 377 => CombatLogManpower(CombatLogManpowerRow),
CombatLogTemplates = 378 => CombatLogTemplates(CombatLogTemplateRow),
CombatLogReferences = 379 => CombatLogReferences(CombatLogReferenceRow),
CombatLogBattles = 380 => CombatLogBattles(CombatLogBattleRow),
CombatLogEnemyEquipment = 381 => CombatLogEnemyEquipment(CombatLogEnemyEquipmentRow),
CombatLogBattleSides = 382 => CombatLogBattleSides(CombatLogBattleSideRow),
CombatLogBattleEquipment = 383 => CombatLogBattleEquipment(CombatLogBattleEquipmentRow),
CombatLogBattleModifiers = 384 => CombatLogBattleModifiers(CombatLogBattleModifierRow),
CombatLogBattleCountries = 385 => CombatLogBattleCountries(CombatLogBattleCountryRow),
CombatLogObservers = 386 => CombatLogObservers(CombatLogObserverRow),
CountryTemplateState = 387 => CountryTemplateState(CountryTemplateStateRow),
NavalRepairCountries = 388 => NavalRepairCountries(NavalRepairCountryRow),
NavalRepairFleets = 389 => NavalRepairFleets(NavalRepairFleetRow),
NavalRepairTaskForces = 390 => NavalRepairTaskForces(NavalRepairTaskForceRow),
NavalTaskForceComposition = 391 => NavalTaskForceComposition(NavalTaskForceCompositionRow),
NavalRepairBases = 392 => NavalRepairBases(NavalRepairBaseRow),
NavalRepairQueue = 393 => NavalRepairQueue(NavalRepairQueueRow),
NavalRefitShips = 394 => NavalRefitShips(NavalRefitShipRow),
NavalRefitProgress = 395 => NavalRefitProgress(NavalRefitProgressRow),
NavalRepairChildren = 396 => NavalRepairChildren(NavalRepairChildRow),
CountryMarket = 397 => CountryMarket(CountryMarketRow),
MarketContractTerms = 398 => MarketContractTerms(MarketContractTermsRow),
LendLeaseAgreements = 399 => LendLeaseAgreement(LendLeaseAgreementRow),
MarketEvents = 400 => MarketEvent(MarketEventRow),
IntelNetworkSummaries = 401 => IntelNetworkSummary(IntelNetworkSummaryRow),
PropagandaEfforts = 402 => PropagandaEffort(PropagandaEffortRow),
IntelContributions = 403 => IntelContribution(IntelContributionRow),
IntelSources = 404 => IntelSource(IntelSourceRow),
IntelNetworkAuxiliaries = 405 => IntelNetworkAuxiliary(IntelNetworkAuxiliaryRow),
MovementStateTerms = 406 => MovementStateTerm(MovementStateTermRow),
CoreSupplyAccess = 407 => CoreSupplyAccess(CoreSupplyAccessRow),
CountryUnitSpeedEntries = 408 => CountryUnitSpeedEntry(CountryUnitSpeedEntryRow),
RiverRelations = 409 => RiverRelation(RiverRelationRow),
DivisionTransportSpeeds = 410 => DivisionTransportSpeed(OwnerTransportSpeedRow),
WarParticipation = 411 => WarParticipation(WarParticipationRow),
WarCapturedProvinces = 412 => WarCapturedProvinces(WarCapturedProvinceRow),
WarLendLeaseHistories = 413 => WarLendLeaseHistories(WarLendLeaseHistoryRow),
AirGroundCrews = 414 => AirGroundCrews(AirGroundCrewRow),
AirTransferInputs = 415 => AirTransferInputs(AirTransferInputRow),
NavalRegionalMines = 416 => NavalRegionalMines(NavalRegionalMineRow),
NavalAccidentReports = 417 => NavalAccidentReports(NavalAccidentReportRow),
NavalDominanceTaskForceRanges = 418 => NavalDominanceTaskForceRanges(NavalMissionTaskForceRow),
NavalPatrolMissionInputs = 419 => NavalPatrolMissionInputs(NavalPatrolMissionInputRow),
CountryUnitBonuses = 420 => CountryUnitBonus(CountryUnitBonusRow),
DivisionModelInputs = 421 => DivisionModelInput(DivisionModelInputRow),
}
derived {
TagHolders = 422 => TagHolder(TagHolderRow),
ArmyTotals = 423 => ArmyTotal(ArmyTotalRow),
ArmyMix = 424 => ArmyMix(ArmyMixRow),
CountryStability = 425 => CountryStability(CountryStabilityRow),
TerritoryTotals = 426 => TerritoryTotal(TerritoryTotalRow),
TagNames = 427 => TagName(TagNameRow),
GroupSpans = 428 => GroupSpan(GroupSpanRow),
Groups = 429 => Group(GroupRow),
GroupNames = 430 => GroupName(GroupNameRow),
WarEvents = 431 => WarEvent(WarEventRow),
LookSpans = 432 => LookSpan(LookSpanRow),
LookFacts = 433 => LookFact(LookFactRow),
BalanceDrops = 434 => BalanceDrop(BalanceDropRow),
AirForceTotals = 435 => AirForceTotal(AirForceTotalRow),
AirForceParts = 436 => AirForcePart(AirForcePartRow),
AirForceWingKinds = 437 => AirForceWingKind(AirForceWingKindRow),
AirForceWingSignatures = 438 => AirForceWingSignature(AirForceWingSignatureRow),
NavyTotals = 439 => NavyTotal(NavyTotalRow),
NavyClasses = 440 => NavyClass(NavyClassRow),
CountryDeployedManpower = 441 => CountryDeployedManpower(CountryDeployedManpowerRow),
MilitaryAid = 442 => MilitaryAid(MilitaryAidRow),
SupplyHealth = 443 => SupplyHealth(SupplyHealthRow),
CountryExperienceDaily = 444 => CountryExperienceDaily(CountryExperienceDailyRow),
CountryGameScores = 445 => CountryGameScore(CountryGameScoreRow),
StabilityParts = 446 => StabilityPart(StabilityPartRow),
StabilityPartKeys = 447 => StabilityPartKey(StabilityPartKeyRow),
ThreatEvents = 448 => ThreatEvent(ThreatEventRow),
EquipmentLossTotals = 449 => EquipmentLossTotal(EquipmentLossTotalRow),
CombatLogDropped = 450 => CombatLogDropped(CombatLogDroppedRow),
ShipLosses = 451 => ShipLoss(ShipLossRow),
EquipmentLossCoverage = 452 => EquipmentLossCoverage(EquipmentLossCoverageRow),
StockByVariant = 453 => StockByVariant(StockByVariantRow),
DivisionEquipmentCounts = 454 => DivisionEquipmentCount(DivisionEquipmentCountRow),
DivisionEquipmentByType = 455 => DivisionEquipmentByType(DivisionEquipmentTypeRow),
DivisionEquipmentByDesign = 456 => DivisionEquipmentByDesign(DivisionEquipmentDesignRow),
DivisionEquipmentByTemplate = 457 => DivisionEquipmentByTemplate(DivisionEquipmentTemplateRow),
FuelDays = 458 => FuelDay(FuelDayRow),
FuelDemandTotals = 459 => FuelDemandTotal(FuelDemandTotalRow),
TradeFlows = 460 => TradeFlow(TradeFlowRow),
ConstructionProjects = 461 => ConstructionProject(ConstructionProjectRow),
ConstructionSpend = 462 => ConstructionSpend(ConstructionSpendRow),
ProductionTotals = 463 => ProductionTotal(ProductionTotalRow),
ProductionCountryTotals = 464 => ProductionCountryTotal(ProductionCountryTotalRow),
ProductionCoverage = 465 => ProductionCoverage(ProductionCoverageRow),
DockyardRepairRates = 466 => DockyardRepairRate(DockyardRepairRateRow),
LandBattles = 467 => LandBattle(LandBattleRow),
LandBattleEnds = 468 => LandBattleEnd(LandBattleEndRow),
NavalBattles = 469 => NavalBattle(NavalBattleRow),
NavalResultObservations = 470 => NavalResultObservation(NavalResultObservationRow),
NavalLossTotals = 471 => NavalLossTotal(NavalLossTotalRow),
DivisionEvents = 472 => DivisionEvent(DivisionEventRow),
LeaderBattles = 473 => LeaderBattle(LeaderBattleRow),
LeaderTraitGains = 474 => LeaderTraitGain(LeaderTraitGainRow),
PowerSpendTotals = 475 => PowerSpendTotal(PowerSpendTotalRow),
PowerItems = 476 => PowerItem(PowerItemRow),
PowerDayParts = 477 => PowerDayPart(PowerDayPartRow),
PowerHolds = 478 => PowerHold(PowerHoldRow),
PowerSpendCoverage = 479 => PowerSpendCoverage(PowerSpendCoverageRow),
ResearchAssignments = 480 => ResearchAssignment(ResearchAssignmentRow),
ResearchCoverage = 481 => ResearchCoverage(ResearchCoverageRow),
FactionAccountOwners = 482 => FactionAccountOwner(FactionAccountOwnerRow),
PeaceOutcomes = 483 => PeaceOutcome(PeaceOutcomeRow),
PeaceConferenceSpans = 484 => PeaceConferenceSpan(PeaceConferenceSpanRow),
CasualtyLifetimes = 485 => CasualtyLifetime(CasualtyLifetimeRow),
CasualtyTotals = 486 => CasualtyTotal(CasualtyTotalRow),
OccupierTotals = 487 => OccupierTotal(OccupierTotalRow),
AirMissionCounts = 488 => AirMissionCount(AirMissionCountRow),
AirRegionWings = 489 => AirRegionWing(AirRegionWingRow),
FeedEvents = 490 => FeedEvent(FeedEventRow),
ProductionTypeTotals = 491 => ProductionTypeTotal(ProductionTypeTotalRow),
FeedOpenEvents = 492 => FeedOpenEvent(FeedOpenEventRow),
FeedOpenDecisions = 493 => FeedOpenDecision(FeedOpenDecisionRow),
FeedGuarantees = 494 => FeedGuarantee(FeedGuaranteeRow),
NavalRegionControl = 495 => NavalRegionControl(NavalRegionControlRow),
CountryNukeTotals = 496 => CountryNukeTotal(CountryNukeTotalRow),
RaidEnds = 497 => RaidEnd(RaidEndRow),
ConstructionLineEnds = 498 => ConstructionLineEnd(ConstructionLineEndRow),
AirMissionWings = 499 => AirMissionWing(AirMissionWingRow),
LeaderBattleTotals = 500 => LeaderBattleTotal(LeaderBattleTotalRow),
LeaderTraitDates = 501 => LeaderTraitDate(LeaderTraitDateRow),
LeaderCommandSpans = 502 => LeaderCommandSpan(LeaderCommandSpanRow),
LeaderCommands = 503 => LeaderCommand(LeaderCommandRow),
ThreatTotals = 504 => ThreatTotal(ThreatTotalRow),
FactionTotals = 505 => FactionTotal(FactionTotalRow),
AirRegionActivity = 506 => AirRegionActivity(AirRegionActivityRow),
AirTheatreRegions = 507 => AirTheatreRegion(AirTheatreRegionRow),
AirRegionDays = 508 => AirRegionDay(AirRegionDayRow),
AirOwnerDays = 509 => AirOwnerDay(AirOwnerDayRow),
TerritoryRegionTotals = 510 => TerritoryRegionTotal(TerritoryRegionTotalRow),
ProducedValues = 511 => ProducedValue(ProducedValueRow),
AirLossTotals = 512 => AirLossTotal(AirLossTotalRow),
BombingTotals = 513 => BombingTotal(BombingTotalRow),
GarrisonStrengths = 514 => GarrisonStrength(GarrisonStrengthRow),
}
recorded {
LandUnitOrder = 515 => LandUnitOrder(LandUnitOrderRow),
StrategicRegionWeights = 516 => StrategicRegionWeight(StrategicRegionWeightRow),
StateBuildingInputs = 517 => StateBuildingInput(StateBuildingInputRow),
}
derived {
PowerSpendDays = 518 => PowerSpendDay(PowerSpendDayRow),
}
recorded {
CombatEngagementRefreshes = 519 => CombatEngagementRefresh(CombatEngagementRefreshRow),
LandCombatEdges = 520 => LandCombatEdge(LandCombatEdgesRow),
}
}
const VARIANT_STATS_PRE_WITNESS_SCHEMA: [Kind; 13] = [
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::I64,
    Kind::U64,
    Kind::I32,
    Kind::Opaque,
];

impl TableId {
    pub fn schema_at(&self, format: u16) -> &'static [Kind] {
        if *self == Self::VariantStats
            && format < compatibility::VARIANT_STAT_WITNESS_FORMAT
        {
            &VARIANT_STATS_PRE_WITNESS_SCHEMA
        } else {
            self.schema()
        }
    }

    pub const fn column_withheld_at(self, column: usize, format: u16) -> bool {
        matches!(self, Self::TrainInventory)
            && column == 4
            && format < compatibility::VARIANT_STAT_WITNESS_FORMAT
    }

    pub fn skip_snapshot(&self, fields: usize, input: &mut Decoder<'_>) -> io::Result<()> {
        let schema = &self.schema_at(input.format)[..fields];
        let minimum = 1 + schema
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>();
        let count = input.count(minimum)?;
        if count > 1 && self.key_bits() == 0 {
            return Err(invalid("singleton table cannot contain multiple rows"));
        }
        let mut previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("row keys did not advance"));
            }
            previous = key;
            for kind in schema {
                kind.decode(input)?;
            }
        }
        Ok(())
    }

    pub fn skip_delta(&self, fields: usize, input: &mut Decoder<'_>) -> io::Result<()> {
        let schema = &self.schema_at(input.format)[..fields];
        let width = fields.div_ceil(8);
        let count = input.count(1 + width)?;
        let mut previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("delta keys did not advance"));
            }
            previous = key;
            let mut bytes = [0; 8];
            bytes[..width].copy_from_slice(input.take(width)?);
            let mask = u64::from_le_bytes(bytes);
            if mask == 0 || mask & !full_mask(fields) != 0 {
                return Err(invalid("invalid discarded delta mask"));
            }
            for (i, kind) in schema.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    kind.decode_change(input)?;
                }
            }
        }
        let count = input.count(1)?;
        previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("deleted keys did not advance"));
            }
            previous = key;
        }
        Ok(())
    }

    pub fn byte_field(self, field: usize) -> bool {
        matches!(
            self.schema().get(field),
            Some(Kind::Bytes | Kind::U32List | Kind::U64List | Kind::Opaque)
        )
    }

    pub fn field_span<'a>(self, field: usize, input: &mut Decoder<'a>) -> io::Result<&'a [u8]> {
        let before = input.rest();
        self.schema_at(input.format)[field].decode(input)?;
        Ok(&before[..before.len() - input.remaining()])
    }

    pub fn change_span<'a>(self, field: usize, input: &mut Decoder<'a>) -> io::Result<&'a [u8]> {
        let before = input.rest();
        self.schema_at(input.format)[field].decode_change(input)?;
        Ok(&before[..before.len() - input.remaining()])
    }

    pub fn walk_update<'a>(
        self,
        bytes: &'a [u8],
        format: u16,
        visit: &mut dyn FnMut(Token, &'a [u8]),
    ) -> io::Result<()> {
        let mut input = Decoder::new(bytes);
        input.format = format;
        let fields = self.last_change().fields(self, format);
        let (status, token) = span(&mut input, |input| input.u8())?;
        visit(Token::Status, token);
        match status {
            0 | 3 => {
                if status == 3 {
                    let (missing, token) = span(&mut input, |input| input.u32())?;
                    if missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    visit(Token::Missing, token);
                }
                let minimum = self.minimum_row_bytes_at(format);
                self.walk_rows(fields, WalkRows::Snapshot, minimum, &mut input, visit)?;
            }
            1 => {
                let (_, token) = span(&mut input, |input| input.i32())?;
                visit(Token::Baseline, token);
                let minimum = self.minimum_delta_bytes_at(format);
                self.walk_rows(fields, WalkRows::Changed, minimum, &mut input, visit)?;
                self.walk_rows(fields, WalkRows::Deleted, 1, &mut input, visit)?;
            }
            2 => {
                let (_, token) = span(&mut input, |input| input.i32())?;
                visit(Token::Baseline, token);
            }
            4 => {
                let (reason, token) = span(&mut input, |input| input.string())?;
                if reason.is_empty() {
                    return Err(invalid("unavailable table needs a reason"));
                }
                visit(Token::Reason, token);
            }
            5 => {}
            _ => return Err(invalid("unknown table status")),
        }
        input.finish()
    }

    fn walk_rows<'a>(
        self,
        fields: usize,
        rows: WalkRows,
        minimum: usize,
        input: &mut Decoder<'a>,
        visit: &mut dyn FnMut(Token, &'a [u8]),
    ) -> io::Result<()> {
        let schema = &self.schema()[..fields];
        let width = fields.div_ceil(8);
        let (count, token) = span(input, |input| input.count(minimum))?;
        visit(Token::Count, token);
        let mut previous = 0;
        for index in 0..count {
            let (key, token) = span(input, |input| self.decode_key(previous, input))?;
            if index != 0 && key <= previous {
                return Err(invalid("row keys did not advance"));
            }
            previous = key;
            visit(Token::Key(key), token);
            match rows {
                WalkRows::Snapshot => {
                    for (field, kind) in schema.iter().enumerate() {
                        let (_, token) = span(input, |input| kind.decode(input).map(|_| ()))?;
                        visit(Token::Field(field), token);
                    }
                }
                WalkRows::Changed => {
                    let (mask, token) = span(input, |input| {
                        let mut bytes = [0; 8];
                        bytes[..width].copy_from_slice(input.take(width)?);
                        Ok(u64::from_le_bytes(bytes))
                    })?;
                    if mask == 0 || mask & !full_mask(fields) != 0 {
                        return Err(invalid("invalid recorded delta mask"));
                    }
                    visit(Token::Mask, token);
                    for (field, kind) in schema.iter().enumerate() {
                        if mask & (1 << field) != 0 {
                            let (_, token) =
                                span(input, |input| kind.decode_change(input).map(|_| ()))?;
                            visit(Token::Field(field), token);
                        }
                    }
                }
                WalkRows::Deleted => {}
            }
        }
        Ok(())
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn minimum_row_bytes(self) -> usize {
        self.minimum_row_bytes_at(crate::FORMAT_VERSION)
    }
    pub fn minimum_row_bytes_at(self, format: u16) -> usize {
        1 + self.schema()[..self.last_change().fields(self, format)]
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn minimum_delta_bytes(self) -> usize {
        self.minimum_delta_bytes_at(crate::FORMAT_VERSION)
    }
    pub fn minimum_delta_bytes_at(self, format: u16) -> usize {
        1 + self.last_change().fields(self, format).div_ceil(8)
            + self.schema()[..self.last_change().fields(self, format)]
                .iter()
                .map(|kind| kind.minimum_change_bytes())
                .min()
                .unwrap_or(0)
    }

    pub fn check_key(self, key: u128) -> io::Result<()> {
        if self.key_bits() < 128 && key >> self.key_bits() != 0 {
            return Err(invalid("row key exceeds native width"));
        }
        if matches!(
            self,
            Self::Clock | Self::Campaign | Self::GameScoreMultipliers
        ) && key != 0
        {
            return Err(invalid("singleton row key must be zero"));
        }
        Ok(())
    }

    pub fn encode_key(self, key: u128, previous: u128, output: &mut Vec<u8>) -> io::Result<()> {
        self.check_key(key)?;
        let difference = key
            .checked_sub(previous)
            .ok_or_else(|| invalid("row keys decreased"))?;
        allocation::reserve(output, 19)?;
        put_varint(output, difference);
        Ok(())
    }

    pub fn decode_key(self, previous: u128, decoder: &mut Decoder<'_>) -> io::Result<u128> {
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

impl TableRows {
    pub fn field_gaps(&self) -> impl Iterator<Item = (&'static str, usize)> + use<> {
        let (fields, nested) = self.field_gaps_by_level();
        fields.chain(nested)
    }

    pub fn field_gaps_by_level(
        &self,
    ) -> (
        impl Iterator<Item = (&'static str, usize)> + use<>,
        impl Iterator<Item = (&'static str, usize)> + use<>,
    ) {
        let table = self.table();
        let mut missing = [0_usize; u64::BITS as usize];
        let mut nested = std::collections::BTreeMap::<&'static str, usize>::new();
        for row in self {
            let mut fields = row.not_recorded_fields();
            while fields != 0 {
                missing[fields.trailing_zeros() as usize] += 1;
                fields &= fields - 1;
            }
            row.visit_nested_field_gaps(&mut |name| {
                *nested.entry(name).or_default() += 1;
            });
        }
        let fields = missing
            .into_iter()
            .enumerate()
            .filter(|&(_, count)| count != 0)
            .map(move |(field, count)| (table.field_name(field).unwrap_or("unnamed field"), count));
        (fields, nested.into_iter())
    }

    pub fn supports_optional_i32_fill(&self, column: usize) -> bool {
        matches!(self, Self::RailCooldowns(_) if column == 3)
    }

    pub fn fill_optional_i32(&mut self, column: usize, value: Option<i32>) -> io::Result<bool> {
        match self {
            Self::RailCooldowns(rows) if column == 3 => {
                let target = Recorded::Value(value);
                let mut changed = false;
                for row in rows {
                    if row.selection_hour != target {
                        row.selection_hour = target;
                        changed = true;
                    }
                }
                Ok(changed)
            }
            _ => Err(invalid("uniform optional i32 fill targets an unsupported column")),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TableRows {
    pub fn fixture_prefix(&self, fields: usize) -> Vec<u8> {
        let table = self.table();
        let mut current = Vec::new();
        self.encode_rows(&mut current).unwrap();
        let mut input = Decoder::new(&current);
        let mut prefix = Vec::new();
        let mut previous = 0;
        for _ in 0..self.len() {
            let start = current.len() - input.remaining();
            previous = table.decode_key(previous, &mut input).unwrap();
            for kind in &table.schema()[..fields] {
                kind.decode(&mut input).unwrap();
            }
            let end = current.len() - input.remaining();
            prefix.extend_from_slice(&current[start..end]);
            for kind in &table.schema()[fields..] {
                kind.decode(&mut input).unwrap();
            }
        }
        input.finish().unwrap();
        prefix
    }
}

impl TryFrom<u16> for TableId {
    type Error = io::Error;

    fn try_from(value: u16) -> io::Result<Self> {
        Self::ALL
            .iter()
            .find(|id| **id as u16 == value)
            .copied()
            .ok_or_else(|| invalid("unknown table id"))
    }
}

#[derive(Clone, Copy)]
enum WalkRows {
    Snapshot,
    Changed,
    Deleted,
}

fn span<'a, T>(
    input: &mut Decoder<'a>,
    read: impl FnOnce(&mut Decoder<'a>) -> io::Result<T>,
) -> io::Result<(T, &'a [u8])> {
    let before = input.rest();
    let value = read(input)?;
    Ok((value, &before[..before.len() - input.remaining()]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Status,
    Baseline,
    Count,
    Missing,
    Reason,
    Key(u128),
    Mask,
    Field(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RowChanges {
    rows: TableRows,
    deleted: Vec<u128>,
}

impl RowChanges {
    pub fn new(rows: TableRows, deleted: Vec<u128>) -> Self {
        Self { rows, deleted }
    }

    pub fn rows(&self) -> &TableRows {
        &self.rows
    }

    pub fn deleted(&self) -> &[u128] {
        &self.deleted
    }

    pub fn heap_bytes(&self) -> usize {
        self.rows.heap_bytes() + self.deleted.capacity() * size_of::<u128>()
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            rows: self.rows.try_clone_validated()?,
            deleted: allocation::copy_vec(&self.deleted)?,
        })
    }
}

pub fn field_from_wire<T: NativeValue>(
    table: TableId,
    column: usize,
    field: Field<'_>,
) -> io::Result<T> {
    from_wire_field(field, table.display_step(column))
}

pub fn encode_delta(
    table: TableId,
    delta: &DeltaRow,
    previous_key: u128,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    if delta.recorded_fields != table.field_count() || delta.format != crate::FORMAT_VERSION {
        return Err(invalid("historical fields cannot be re-encoded"));
    }
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
    allocation::reserve(
        output,
        allocation::add(table.mask_width(), delta.values.len())?,
    )?;
    output.extend_from_slice(&delta.mask.to_le_bytes()[..table.mask_width()]);
    output.extend_from_slice(&delta.values);
    Ok(())
}

pub fn decode_delta(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'_>,
) -> io::Result<DeltaRow> {
    let (key, mask, fields, values) = decode_delta_with(table, previous_key, decoder)?;
    let values = allocation::copy_vec(values)?;
    #[cfg(any(test, feature = "test-support"))]
    delta_decode_ownership::record();
    Ok(DeltaRow {
        format: decoder.format,
        key,
        mask,
        values,
        recorded_fields: fields,
    })
}

pub fn validate_delta_row(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'_>,
) -> io::Result<u128> {
    decode_delta_with(table, previous_key, decoder).map(|(key, ..)| key)
}

fn decode_delta_with<'a>(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'a>,
) -> io::Result<(u128, u64, usize, &'a [u8])> {
    let key = table.decode_key(previous_key, decoder)?;
    let mut mask_bytes = [0; 8];
    let fields = table.last_change().fields(table, decoder.format);
    let width = fields.div_ceil(8);
    mask_bytes[..width].copy_from_slice(decoder.take(width)?);
    let mask = u64::from_le_bytes(mask_bytes);
    if mask == 0 || mask & !full_mask(fields) != 0 {
        return Err(invalid("invalid recorded delta mask"));
    }
    let values = decoder.rest();
    for (index, kind) in table.schema().iter().enumerate() {
        if mask & (1_u64 << index) != 0 {
            kind.decode_change(decoder)?;
        }
    }
    Ok((
        key,
        mask,
        fields,
        &values[..values.len() - decoder.remaining()],
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub mod delta_decode_ownership {
    use std::cell::Cell;

    thread_local! {
        static OWNED_ROWS: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub(super) fn record() {
        OWNED_ROWS.with(|count| {
            if let Some(value) = count.get() {
                count.set(Some(value + 1));
            }
        });
    }

    pub fn count<T>(read: impl FnOnce() -> T) -> (T, usize) {
        struct Reset(Option<usize>);
        impl Drop for Reset {
            fn drop(&mut self) {
                OWNED_ROWS.with(|count| count.set(self.0));
            }
        }
        let _reset = Reset(OWNED_ROWS.with(|count| count.replace(Some(0))));
        let result = read();
        let count = OWNED_ROWS.with(|count| count.get().unwrap());
        (result, count)
    }
}

pub const PACKET_HOUR_HOSTS: [TableId; 10] = [
    TableId::DivisionMovement,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMapMotion,
    TableId::RailwayGunCounterInputs,
    TableId::SimulationFeatures,
    TableId::RelationLinks,
    TableId::CountryWars,
    TableId::DivisionTemplateIcons,
];
#[cfg(any(test, feature = "test-support"))]
pub fn packet_hours(rows: &TableRows) -> Vec<PacketHour> {
    fn some<T>(field: &Recorded<Option<T>>, hour: impl Fn(&T) -> PacketHour) -> Option<PacketHour> {
        match field {
            Recorded::Value(Some(packet)) => Some(hour(packet)),
            _ => None,
        }
    }
    fn value<T>(field: &Recorded<T>, hour: impl Fn(&T) -> PacketHour) -> Option<PacketHour> {
        match field {
            Recorded::Value(packet) => Some(hour(packet)),
            Recorded::NotRecorded => None,
        }
    }
    let mut hours = Vec::new();
    match rows {
        TableRows::DivisionMovement(rows) => {
            for row in rows {
                hours.extend(some(&row.combat_references, |p| p.sample_hour));
            }
        }
        TableRows::CountryCounterInputs(rows) => {
            for row in rows {
                hours.extend(some(&row.directed_wars, |p| p.sample_hour));
            }
        }
        TableRows::ReservedCountryNaming(rows) => {
            for row in rows {
                hours.extend(some(&row.launch_sources, |p| p.sample_hour));
                hours.extend(some(&row.reverse_visibility, |p| p.sample_hour));
            }
        }
        TableRows::TaskForceCounterInputs(rows) => {
            for row in rows {
                hours.extend(some(&row.port_mission, |p| p.sample_hour));
            }
        }
        TableRows::TaskForceMapMotion(rows) => {
            for row in rows {
                hours.extend(some(&row.fresh, |p| p.sample_hour));
            }
        }
        TableRows::RailwayGunCounterInputs(rows) => {
            for row in rows {
                hours.extend(some(&row.fresh, |p| p.sample_hour));
                hours.extend(some(&row.combat_references, |p| p.sample_hour));
            }
        }
        TableRows::SimulationFeatures(rows) => {
            for row in rows {
                hours.extend(value(&row.runtime_scalars, |p| p.sample_hour));
            }
        }
        TableRows::RelationLinks(rows) => {
            for row in rows {
                hours.extend(some(&row.transport_subject, |p| p.sample_hour));
            }
        }
        TableRows::CountryWars(rows) => {
            for row in rows {
                hours.extend(some(&row.ordered, |p| p.sample_hour));
            }
        }
        TableRows::DivisionTemplateIcons(rows) => {
            for row in rows {
                hours.extend(some(&row.display_name, |p| p.sample_hour));
            }
        }
        other => panic!("{:?} holds no packet hour", other.table()),
    }
    hours
}
pub const RESERVED_SPLIT_TABLES: [TableId; 9] = [
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
#[cfg(any(test, feature = "test-support"))]
fn samples<T: RowCodec + Clone>(row: T, tick: usize, key: impl Fn(&mut T, u128)) -> Vec<T> {
    let last = (1u128 << T::KEY_BITS) - 1;
    [0, 1, last]
        .into_iter()
        .take(if tick % 17 < 5 { 3 } else { 2 })
        .map(|value| {
            let mut row = row.clone();
            key(&mut row, value);
            row
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn sample_politics_table(table: TableId, tick: usize) -> Option<TableRows> {
    let epoch = tick as u64 + 1;
    let hour = (tick as i32).saturating_mul(24);
    let raw = [i64::MIN, -1, i64::MAX][tick % 3];
    let cost = [0, 1, -1][tick % 3];
    let integer = [0, 1, u32::MAX][tick % 3];
    let wide = [0, 1, u64::MAX][tick % 3];
    let maybe = (!tick.is_multiple_of(4)).then_some(integer);
    let identity = PowerIdentity {
        action: if tick.is_multiple_of(2) {
            PowerAction::Execute
        } else {
            PowerAction::Replace
        },
        item_token: maybe,
        item_key: vec![0xff, tick as u8, 0x80],
        slot_key: vec![tick as u8],
        character: maybe,
        object_kind: if tick.is_multiple_of(2) {
            PowerObjectKind::Leader
        } else {
            PowerObjectKind::Organisation
        },
        object_reference: Some(wide),
        target_country: maybe,
        target_state: maybe,
        target_region: maybe,
        origin_kind: if tick.is_multiple_of(2) {
            PowerOriginKind::Focus
        } else {
            PowerOriginKind::Event
        },
        origin_token: maybe,
        origin_key: vec![tick as u8],
    };
    let cause = [
        PowerCause::DailyGain,
        PowerCause::Diplomacy,
        PowerCause::AbilityUpkeep,
    ][tick % 3];
    let site = if tick.is_multiple_of(2) {
        PowerSite::AddPoliticalPower
    } else {
        PowerSite::AbilityUpkeep
    };
    Some(match table {
        TableId::CountryPowerDaily => TableRows::CountryPowerDaily(samples(
            CountryPowerDailyRow {
                country: 0,
                instance: integer,
                epoch,
                day_hour: hour,
                pp_gross_gain_raw: maybe.map(|_| raw),
                pp_cost_raw: maybe.map(|_| cost),
                pp_net_gain_raw: maybe.map(|_| raw.wrapping_sub(cost)),
                pp_realized_delta_raw: maybe.map(|_| raw),
                pp_sequence: maybe.map(|_| wide),
                cp_gain_raw: maybe.map(|_| raw),
                cp_realized_delta_raw: maybe.map(|_| raw),
                cp_sequence: maybe.map(|_| wide.wrapping_add(1)),
            },
            tick,
            |row, key| row.country = key as u32,
        )),
        TableId::CountryPowerChanges => TableRows::CountryPowerChanges(samples(
            CountryPowerChangeRow {
                country: 0,
                instance: integer,
                sequence: 0,
                epoch,
                hour,
                currency: if tick.is_multiple_of(2) {
                    PowerCurrency::Political
                } else {
                    PowerCurrency::Command
                },
                operation: if tick.is_multiple_of(2) {
                    PowerOperation::Subtract
                } else {
                    PowerOperation::Grant
                },
                balance_before_raw: raw.wrapping_add(1),
                balance_after_raw: raw,
                requested_delta_raw: maybe.map(|_| raw),
                cause,
                site,
                identity,
            },
            tick,
            |row, key| {
                row.country = (key >> 64) as u32;
                row.sequence = key as u64;
            },
        )),
        TableId::CountryPowerDailyCosts => TableRows::CountryPowerDailyCosts(samples(
            CountryPowerDailyCostRow {
                country: 0,
                instance: integer,
                ordinal: 0,
                epoch,
                day_hour: hour,
                pp_sequence: wide,
                component: [
                    PowerDailyCostKind::MioProduction,
                    PowerDailyCostKind::Relation,
                    PowerDailyCostKind::PoliticalPowerCostModifier,
                ][tick % 3],
                cost_raw: raw,
                cause,
                site,
                identity,
            },
            tick,
            |row, key| {
                row.country = (key >> 32) as u32;
                row.ordinal = key as u32;
            },
        )),
        TableId::GovernmentChanges => TableRows::GovernmentChanges(samples(
            GovernmentChangeRow {
                country: 0,
                instance: integer,
                sequence: 0,
                epoch,
                hour,
                before_party: maybe,
                after_party: Some(integer),
                before_ideology: maybe,
                after_ideology: Some(integer.wrapping_add(1)),
                cause: if tick.is_multiple_of(2) {
                    GovernmentCause::SetPoliticsEffect
                } else {
                    GovernmentCause::ElectionOnAction
                },
                actor_country: maybe,
                origin_kind: if tick.is_multiple_of(2) {
                    PowerOriginKind::Event
                } else {
                    PowerOriginKind::Focus
                },
                origin_token: maybe,
                origin_key: vec![0xff, tick as u8, 0x80],
                site,
            },
            tick,
            |row, key| {
                row.country = (key >> 64) as u32;
                row.sequence = key as u64;
            },
        )),
        TableId::PoliticsCaptureCoverage => TableRows::PoliticsCaptureCoverage(samples(
            PoliticsCaptureCoverageRow {
                country: 0,
                instance: integer,
                epoch,
                since_hour: hour - 1,
                through_hour: hour,
                streams: [
                    0,
                    PoliticsCaptureCoverageRow::PP,
                    PoliticsCaptureCoverageRow::ALL_STREAMS,
                ][tick % 3],
            },
            tick,
            |row, key| row.country = key as u32,
        )),
        _ => return None,
    })
}

#[cfg(test)]
#[path = "rows/border/tests.rs"]
mod border_tests;
#[cfg(test)]
#[path = "rows/validated_apply_tests.rs"]
mod validated_apply_tests;

#[cfg(test)]
#[path = "rows/display_tests.rs"]
mod display_tests;

#[cfg(test)]
#[path = "rows/supply_footer_tests.rs"]
mod supply_footer_tests;

#[cfg(test)]
#[path = "rows/loss_sum_tests.rs"]
mod loss_sum_tests;

#[cfg(test)]
#[path = "rows/land_modifier_tests.rs"]
mod land_modifier_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wing_fuel_accessor_preserves_independent_absence_and_rejects_invalid_domains() {
        let mut row = AirWingRow {
            fuel_base_raw: None.into(),
            fuel_current_type: Some(0).into(),
            fuel_effective_aircraft: None.into(),
            fuel_context_mode: None.into(),
            fuel_capacity_class: Some(-1).into(),
            fuel_override_resolved: Some(0).into(),
            ..Default::default()
        };
        let input = row.fuel_inputs().unwrap();
        assert_eq!(input.base_fuel, None);
        assert_eq!(input.current_type, Some(0));
        assert_eq!(input.capacity_class, Some(-1));
        assert_eq!(input.override_resolved, Some(false));
        row.fuel_override_resolved = Some(2).into();
        assert!(row.fuel_inputs().is_err());
        row.fuel_override_resolved = Some(1).into();
        row.fuel_context_mode = Some(256).into();
        assert!(row.fuel_inputs().is_err());
    }

    #[test]
    fn peace_operands_failures_and_omissions_round_trip() {
        let row = PeaceSettlementRow {
            sequence: u64::MAX,
            conference: Some(0x700000009),
            hour: Some(91),
            winners: vec![0xf0000021],
            losers: vec![0xa0000023],
            actions: vec![SettledPeaceAction {
                token: u32::MAX,
                taker: 0xf0000021,
                giver: 0xa0000023,
                negotiator: 0xf0000030,
                status: 2,
                cost: -7,
                state: Some(9234),
                flags: Some([1, 2, 3, 4]),
                ..Default::default()
            }],
            transfers: vec![SettledPeaceTransfer {
                state: 9234,
                taker: 0xf0000021,
                automatic: true,
            }],
            subjects: vec![SettledPeaceSubject {
                overlord: 0xf0000021,
                subject: 0xa0000023,
            }],
            failure: Some(b"first guarded fault".to_vec()),
            omitted: 8,
        };
        round_trip(TableId::PeaceSettlements, row.clone());
        round_trip(TableId::PeaceBids, row);
    }

    #[test]
    fn peace_success_and_present_empty_failure_are_distinct_on_the_wire() {
        let success = PeaceSettlementRow {
            conference: Some(7),
            hour: Some(91),
            ..Default::default()
        };
        let failure = PeaceSettlementRow {
            failure: Some(Vec::new()),
            ..success.clone()
        };
        round_trip(TableId::PeaceSettlements, success.clone());
        round_trip(TableId::PeaceSettlements, failure.clone());
        pair(TableId::PeaceSettlements, success, failure);
    }

    #[test]
    fn battle_sounds_round_trip_exact_bits_and_member_order() {
        let row = BattleSoundRow {
            ordinal: u32::MAX,
            center_x: (-0.0_f32).to_bits(),
            center_z: 0x7fc0_1234,
            radius: f32::MAX.to_bits(),
            provinces: vec![13_000, 7, 13_000, u32::MAX],
            sounding: true,
            object: 0x0000_7ff6_1234_5678,
        };
        round_trip(TableId::BattleSounds, row.clone());
        round_trip(TableId::BattleSounds, BattleSoundRow::default());
        let moved = BattleSoundRow {
            center_x: 1.5_f32.to_bits(),
            ..row.clone()
        };
        pair(TableId::BattleSounds, row.clone(), moved.clone());
        let delta = moved.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), moved);
        assert_eq!(BattleSoundRow::default().key(), 0);
        assert_eq!(row.key(), u32::MAX as u128);
    }

    #[test]
    fn nuclear_strike_outcomes_and_refusals_round_trip_and_validate() {
        let row = NuclearStrikeRow {
            id: 41,
            hour: -7,
            launcher: crate::event_feed_rows::EventFeedCountry {
                reference: 4_999,
                id: 4_998,
                instance: u32::MAX,
            },
            nuke_type: 1,
            province: 13_000,
            state: 0,
            owner: crate::event_feed_rows::EventFeedCountry {
                reference: 0,
                id: 0,
                instance: 7,
            },
            controller: 77,
            war_support: vec![NuclearWarSupport {
                country: 0,
                before_raw: i64::MIN,
                after_raw: i64::MAX,
            }],
            buildings: vec![NuclearBuildingDamage {
                state: 0,
                province: Some(13_000),
                controller: 77,
                building_type: 0xf004,
                amount_raw: -1,
            }],
            units: vec![NuclearUnitDamage {
                handle: (8 << 32) | 0x42,
                class: 13,
                owner: 5,
                strength_before: 100_000,
                strength_after: 0,
                organisation_before: None,
                organisation_after: None,
                manpower_before: 250,
                manpower_after: 0,
            }],
            equipment: vec![NuclearEquipmentLoss {
                country: 3,
                wing: true,
                variant: u64::MAX,
                amount_raw: 300_000,
            }],
            refusals: vec![NuclearStrikeRefusal {
                part: NuclearStrikePart::Units,
                reason: b"nuclear strike unit class unsupported".to_vec(),
                count: 2,
            }],
            wings: vec![NuclearWingLoss {
                country: 3,
                manpower_before: 400,
                manpower_after: 0,
            }],
        };
        assert!(row.complete(NuclearStrikePart::Buildings));
        assert!(!row.complete(NuclearStrikePart::Units));
        round_trip(TableId::NuclearStrikes, row.clone());
        round_trip(TableId::NuclearStrikes, NuclearStrikeRow::default());
        let mut uncounted = row.clone();
        uncounted.refusals[0].count = 0;
        assert!(uncounted.check_key_fields().is_err());
        let mut half = row;
        half.units[0].organisation_before = Some(1);
        assert!(half.check_key_fields().is_err());
    }

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
        let mut streamed = Vec::new();
        assert!(row.encode_change(None, 0, &mut streamed).unwrap());
        assert_eq!(streamed, bytes);
        assert!(
            !row.encode_change(Some(&row), row.key(), &mut streamed)
                .unwrap()
        );
        assert_eq!(streamed, bytes, "unchanged rows must write nothing");
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
            let mut streamed = Vec::new();
            assert!(new.encode_change(Some(old), 0, &mut streamed).unwrap());
            assert_eq!(
                streamed, bytes,
                "direct output differs from the independent delta codec"
            );
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
            border_land_doctrine_level: crate::rows::Recorded::Value(None),
            id: 7,
            canonical_slot: 1,
            tag: vec![b'G', 0xff, b'r'],
            political_power_raw: Some(i64::MIN),
            command_power_raw: -9_007_199_254_740_993,
            stability_raw: 100_001,
            war_support_raw: 99_999,
            convoys_raw: Some(i64::MAX),
            peace_conference_participant: u8::MAX,
            manpower_residual: i32::MIN,
        }
    }

    #[test]
    fn streamed_merge_matches_independent_changes_and_deletions() {
        let row = |id, owner| StateRow {
            id,
            owner,
            ..StateRow::default()
        };
        let cases = [
            (vec![], vec![]),
            (vec![], vec![row(2, 9), row(u32::MAX, 7)]),
            (vec![row(1, 9), row(4, 7)], vec![]),
            (
                vec![row(1, 9), row(3, 1), row(4, 7)],
                vec![row(2, 9), row(3, 2), row(4, 7), row(8, 6)],
            ),
            (vec![row(3, 2)], vec![row(3, 2)]),
        ];
        let mut scratch = Vec::new();
        for (old, new) in cases {
            let changed = new
                .iter()
                .filter_map(|row| {
                    row.delta(old.iter().find(|before| before.id == row.id))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            let deleted = old
                .iter()
                .filter(|before| !new.iter().any(|row| row.id == before.id))
                .map(RowCodec::key)
                .collect::<Vec<_>>();
            let mut expected = (changed.len() as u32).to_le_bytes().to_vec();
            let mut previous_key = 0;
            for row in &changed {
                encode_delta(TableId::States, row, previous_key, &mut expected).unwrap();
                previous_key = row.key;
            }
            expected.extend_from_slice(&(deleted.len() as u32).to_le_bytes());
            previous_key = 0;
            for key in &deleted {
                TableId::States
                    .encode_key(*key, previous_key, &mut expected)
                    .unwrap();
                previous_key = *key;
            }
            let mut actual = Vec::new();
            let any = !changed.is_empty() || !deleted.is_empty();
            assert_eq!(
                encode_changes_typed(&new, &old, &mut actual, &mut scratch, None).unwrap(),
                any
            );
            assert_eq!(actual, expected);
            let mut recorded = Vec::new();
            let mut rows = ChangedRows::default();
            assert_eq!(
                encode_changes_typed(&new, &old, &mut recorded, &mut scratch, Some(&mut rows))
                    .unwrap(),
                any
            );
            assert_eq!(recorded, if any { expected } else { Vec::new() });
            let pairs: Vec<(u32, Option<u32>)> = new
                .iter()
                .enumerate()
                .filter(|(_, row)| changed.iter().any(|delta| delta.key == row.key()))
                .map(|(at, row)| {
                    let before = old.iter().position(|before| before.id == row.id);
                    (at as u32, before.map(|before| before as u32))
                })
                .collect();
            assert_eq!((rows.pairs, rows.removed, rows.lost), (pairs, deleted, false));
        }
    }

    #[test]
    fn a_recorded_merge_of_an_unchanged_table_grows_nothing() {
        let rows = TableRows::States(vec![StateRow {
            id: 3,
            owner: 2,
            ..StateRow::default()
        }]);
        let mut output = vec![9_u8; 5];
        output.shrink_to_fit();
        let capacity = output.capacity();
        let mut changed = ChangedRows::default();
        assert!(
            !rows
                .encode_changes_recorded(&rows, &mut output, &mut Vec::new(), &mut changed)
                .unwrap()
        );
        assert_eq!(output.capacity(), capacity);
        assert_eq!(output, vec![9_u8; 5]);
        assert!(changed.pairs.is_empty() && changed.removed.is_empty() && !changed.lost);
    }
    fn division() -> DivisionRow {
        DivisionRow {
            id: 0x8000_0001_0000_0029,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 123,
            template: 0x8000_0002_0000_002a,
            hit_points: -9_007_199_254_740_000,
            organisation: 100_000,
            orders_group: 0x0000_0039_0000_0035,
            reveal_skip: 0,
            theatre: Recorded::Value(Some(0x0000_0011_0000_0043)),
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
    fn new_division_operands_apply_through_current_masks() {
        let previous = LandCombatDivisionRow {
            combat: 7,
            side: 0,
            ordinal: 1,
            ..LandCombatDivisionRow::default()
        };
        let next = LandCombatDivisionRow {
            direction_group: i32::MIN,
            exile: Some(crate::rows::LandCombatExileInputs {
                origin_type: 2,
                location_cores: vec![u32::MAX],
                ..Default::default()
            }),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 10) | (1 << 11));
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
        let mut bytes = Vec::new();
        encode_delta(TableId::LandCombatDivisions, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let restored = decode_delta(TableId::LandCombatDivisions, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored.apply(Some(previous)).unwrap(), next);
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
                speed: i32::MIN,
                debug_current_ref_id: Some(u32::MAX),
                unit: Some(i32::MIN),
                order_index: Some(i32::MAX),
                front_index: Some(-1),
                theatre_index: Some(0),
                theater_group_index: None,
                military_deployment_line_index: Some(7841),
                military_deployment_conveyor_index: Some(744),
                equipment_variant_index: Some(6295),
                country_leader_index: Some(1),
                navy_id: Some(8263),
                land_combat_id: Some(15_359),
                multiplayer_random_count: Some(i32::MIN),
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
                fort_positive: 1,
                move_gate: 0,
                port_sea: 0,
                attack: Default::default(),
                selected_areas: Default::default(),
                fort_bunker: Recorded::Value(Some(-25_000)),
                controller_area: Recorded::Value(Some(u32::MAX)),
                owner_area: Recorded::Value(None),
                area_node: Recorded::Value(Some(ControllerAreaNode {
                    first_land: None,
                    neighbours: Some(vec![u32::MAX, 0, u32::MAX]),
                })),
            },
        );
        round_trip(
            TableId::States,
            StateRow {
                infrastructure: Default::default(),
                manpower_locked: Default::default(),

                id: u32::MAX,
                owner: 0,
                controller: u32::MAX,
                movement_infrastructure: i64::MIN,
                railway_infrastructure: Some(i64::MAX).into(),
                manpower_available: i32::MIN,
                population: i32::MAX.into(),
                category: Recorded::Value(Some(u32::MAX)),
                custom_name: vec![0xff, 0, b'a'],
                owner_name: "Département".into(),
                continent: Recorded::Value(Some(u32::MAX)),

                best_vp_index: Default::default(),

                single_controller: Default::default(),
                marked_owners: Recorded::Value(Some(vec![u32::MAX, 0, 7])),
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
                orders_group: u64::MAX,
                reveal_skip: 0,
                theatre: Recorded::Value(Some(u64::MAX)),
            },
        );
        round_trip(
            TableId::Divisions,
            DivisionRow {
                theatre: Recorded::Value(None),
                ..division()
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
                ..FactionRow::default()
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: u64::MAX,
                name: vec![0, 0xff, b'A'],
                template_token: Some(u32::MAX),
                color_red: f32::NAN.to_bits(),
                color_green: 0,
                color_blue: u32::MAX,
                color_alpha: 1.0_f32.to_bits(),
                color_overridden: u8::MAX,
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: 7,
                name: Vec::new(),
                template_token: Some(0),
                ..FactionRow::default()
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
            fort_positive: 0,
            move_gate: 0,
            port_sea: 0,
            attack: Default::default(),
            selected_areas: Default::default(),
            fort_bunker: Recorded::Value(None),
            controller_area: Default::default(),
            owner_area: Default::default(),
            area_node: Default::default(),
        };
        assert_eq!(TableId::Provinces.full_mask(), 8191);
        for mask in 1..=127 {
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
                fort_positive: if mask & 16 != 0 {
                    1
                } else {
                    previous.fort_positive
                },
                move_gate: if mask & 32 != 0 {
                    u8::MAX
                } else {
                    previous.move_gate
                },
                port_sea: if mask & 64 != 0 {
                    i32::MIN
                } else {
                    previous.port_sea
                },
                attack: previous.attack.clone(),
                selected_areas: previous.selected_areas,
                fort_bunker: previous.fort_bunker,
                controller_area: previous.controller_area,
                owner_area: previous.owner_area,
                area_node: previous.area_node.clone(),
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
            let mut input = Decoder::new(&wire);
            assert_eq!(input.varint().unwrap(), row.key());
            let fields = wire.len() - input.remaining();
            assert_eq!(
                &wire[fields + 12..fields + 16],
                &victory_points.to_le_bytes()
            );
            round_trip(TableId::Provinces, row);
        }
    }

    #[test]
    fn province_areas_keep_labels_null_and_node_refusals_distinct() {
        let base = ProvinceRow {
            id: 17,
            controller: 701,
            owner: 702,
            controller_area: Recorded::Value(Some(17)),
            owner_area: Recorded::Value(Some(0)),
            area_node: Recorded::Value(Some(ControllerAreaNode {
                first_land: Some(u32::MAX),
                neighbours: Some(vec![29, 29, 0, u32::MAX]),
            })),
            ..Default::default()
        };
        round_trip(TableId::Provinces, base.clone());
        for change in 0..7 {
            let mut next = base.clone();
            fn node(row: &mut ProvinceRow) -> &mut ControllerAreaNode {
                match &mut row.area_node {
                    Recorded::Value(Some(node)) => node,
                    _ => unreachable!(),
                }
            }
            match change {
                0 => next.controller_area = Recorded::Value(None),
                1 => next.controller_area = Recorded::Value(Some(0)),
                2 => next.owner_area = Recorded::Value(None),
                3 => next.area_node = Recorded::Value(None),
                4 => node(&mut next).first_land = None,
                5 => node(&mut next).neighbours = None,
                _ => node(&mut next).neighbours = Some(Vec::new()),
            }
            assert_ne!(next, base, "{change}");
            let delta = next.delta(Some(&base)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << [10, 10, 11, 12, 12, 12, 12][change]);
            let mut wire = Vec::new();
            encode_delta(TableId::Provinces, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let restored = decode_delta(TableId::Provinces, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(base.clone())).unwrap(), next);
            round_trip(TableId::Provinces, next);
        }
    }

    #[test]
    fn map_rows_preserve_full_native_handles_signed_values_and_ordered_lists() {
        round_trip(
            TableId::Players,
            PlayerRow {
                ordinal: 0.into(),
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
                attacker_classes: vec![0, 13, u32::MAX],
                defender_classes: vec![1, 0],
            },
        );
        round_trip(
            TableId::Fleets,
            FleetRow {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX,
                name: Recorded::Value(Some(b"synthetic fleet".to_vec())),
                leader: Recorded::Value(Some(u64::MAX)),
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
                route_count: 0,
                combat_references: i32::MIN,
            },
        );
        round_trip(
            TableId::Ships,
            ShipRow {
                name: Default::default(),
                naming: Default::default(),
                acquired_manpower: Default::default(),
                required_manpower: Default::default(),

                id: u64::MAX,
                task_force: u64::MAX,
                strength: i64::MIN,
                organisation: i64::MAX,
                experience: -9_007_199_254_741_000,
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
                registry_ordinal: u32::MAX,
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
                acquired_manpower: Default::default(),
                exile_country: Default::default(),

                id: u64::MAX,
                owner: u32::MAX,
                air_base: u64::MAX,
                base_override: 1_u64 << 63,
                country_override: i32::MIN,
                planes: i32::MIN,
                mission_mask: u32::MAX,
                mission_region: Some(u32::MAX),
                base_range_raw: i64::MIN,
                ace: u64::MAX,
                mission_active: u8::MAX,
                eligibility_veto: 1,
                eligibility_counter: i32::MIN,
                eligibility_limit: i32::MAX,
                attached_orders_group: Some(u64::MAX),
                pool_ordinal: u32::MAX,
                name: Recorded::Value(Some(vec![0xff, 0, b'x'])),
                strategic_bombing_raw: Recorded::Value(Some(i64::MIN)),
                replacement_setting: Recorded::Value(Some(u8::MAX)),
                map_flags: Recorded::Value(Some(15)),
                fuel_base_raw: Some(i64::MIN).into(),
                fuel_current_type: Some(i32::MIN).into(),
                fuel_effective_aircraft: Some(i32::MAX).into(),
                fuel_context_mode: Some(u32::from(u8::MAX)).into(),
                fuel_capacity_class: Some(i32::MIN).into(),
                fuel_override_resolved: Some(0).into(),
                pool_subunit: Recorded::Value(Some(u32::MAX)),
                raid_reference: Recorded::Value(Some(AirWingRaidReference {
                    raw: u64::MAX,
                    resolves: false,
                })),
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
    fn one_native_scalar_tick_costs_four_bytes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 4);
        assert_eq!(delta.values, [2]);
        let mut bytes = Vec::new();
        encode_delta(TableId::Countries, &delta, 0, &mut bytes).unwrap();
        assert_eq!(bytes, [7, 0x10, 0, 2]);
        assert_eq!(delta.apply::<CountryRow>(Some(old)).unwrap(), new);
    }

    #[test]
    fn state_masks_keep_owner_and_controller_as_independent_raw_references() {
        let old = StateRow {
            id: 0x1234_5678,
            owner: 7,
            controller: 2,
            movement_infrastructure: 100_000,
            ..StateRow::default()
        };
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        for mask in 1..=TableId::States.full_mask() {
            let new = StateRow {
                infrastructure: Recorded::Value((mask & 8192 != 0).then_some(StateInfrastructure { building_type: 1, level: 2, maximum: 3 })),
                manpower_locked: Recorded::Value(if mask & 1024 != 0 { -1 } else { 0 }),

                id: old.key().try_into().unwrap(),
                manpower_available: if mask & 32 != 0 { i32::MAX } else { 0 },
                continent: Recorded::Value(if mask & 64 != 0 { Some(123) } else { None }),
                owner: if mask & 1 != 0 { 0 } else { 7 },
                controller: if mask & 2 != 0 { u32::MAX } else { 2 },
                movement_infrastructure: if mask & 4 != 0 { 100_001 } else { 100_000 },
                railway_infrastructure: (if mask & 128 != 0 { Some(0) } else { None }).into(),
                population: (if mask & 256 != 0 { -1 } else { 0 }).into(),
                category: Recorded::Value(if mask & 512 != 0 { Some(123) } else { None }),
                custom_name: if mask & 8 != 0 {
                    b"Rename".to_vec()
                } else {
                    Vec::new()
                },
                owner_name: if mask & 16 != 0 {
                    b"Owner".to_vec()
                } else {
                    Vec::new()
                },

                best_vp_index: (if mask & 2048 != 0 { Some(-1) } else { None }).into(),
                single_controller: (if mask & 4096 != 0 { Some(1) } else { None }).into(),
                marked_owners: Recorded::Value((mask & 16384 != 0).then(|| vec![9, 3])),
            };
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::States, &delta, 0, &mut bytes).unwrap();
            let scalars = 5
                + 2
                + usize::from(mask & 1 != 0)
                + 5 * usize::from(mask & 2 != 0)
                + usize::from(mask & 4 != 0)
                + 5 * usize::from(mask & 32 != 0)
                + 3 * usize::from(mask & 64 != 0)
                + 2 * usize::from(mask & 128 != 0)
                + usize::from(mask & 256 != 0)
                + 3 * usize::from(mask & 512 != 0)
                + usize::from(mask & 1024 != 0)
                + 2 * usize::from(mask & 2048 != 0)
                + 2 * usize::from(mask & 4096 != 0)
                + 17 * usize::from(mask & 8192 != 0)
                + 17 * usize::from(mask & 16384 != 0);
            let names = 6 * usize::from(mask & 8 != 0) + 5 * usize::from(mask & 16 != 0);
            if names == 0 {
                assert_eq!(bytes.len(), scalars);
            } else {
                assert!(bytes.len() > scalars + names);
            }
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
    fn division_rows_keep_full_handles_and_store_health_in_display_steps() {
        for id in [0, 1_u64 << 63, u64::MAX] {
            let row = DivisionRow { id, ..division() };
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let key_len = if id == 0 { 1 } else { 10 };
            assert_eq!(bytes.len(), key_len + 58);
            assert_eq!(
                &bytes[key_len + 16..key_len + 24],
                &division().template.to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 24..key_len + 32],
                &(division().hit_points / 10_000).to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 32..key_len + 40],
                &(division().organisation / 10_000).to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 40..key_len + 48],
                &division().orders_group.to_le_bytes()
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
            assert_eq!(bytes.len(), key_len + 2 + delta.values.len());
            assert_eq!(&bytes[key_len..key_len + 2], &[0xff, 3]);
            assert_eq!(delta.apply::<DivisionRow>(None).unwrap(), row);
        }
    }

    #[test]
    fn division_masks_preserve_every_field_subset() {
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
            if mask & 128 != 0 {
                updated.orders_group = u64::MAX;
            }
            if mask & 256 != 0 {
                updated.reveal_skip = 255;
            }
            if mask & 512 != 0 {
                updated.theatre = Recorded::Value(None);
            }
            let new = updated;
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), 10 + 2 + delta.values.len());
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
                new.political_power_raw = None;
            }
            if mask & 8 != 0 {
                new.command_power_raw = i64::MAX;
            }
            if mask & 16 != 0 {
                new.stability_raw = i64::MIN;
            }
            if mask & 32 != 0 {
                new.war_support_raw = -1;
            }
            if mask & 64 != 0 {
                new.convoys_raw = None;
            }
            if mask & 128 != 0 {
                new.peace_conference_participant = 0;
            }
            if mask & 256 != 0 {
                new.manpower_residual = i32::MAX;
            }
            if mask & 512 != 0 {
                new.border_land_doctrine_level = crate::rows::Recorded::Value(Some(i32::MIN));
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
            assert_eq!(delta.mask, 1 << 6);
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
    fn stored_air_wing_name_presence_and_rename_delta_preserve_bytes() {
        let mut previous = AirWingRow {
            id: 0x123400000045,
            ..Default::default()
        };
        for name in [
            Recorded::Value(Some(Vec::new())),
            Recorded::Value(Some(vec![0xff, 0, b'X'])),
            Recorded::Value(None),
        ] {
            let next = AirWingRow {
                name,
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 15);
            assert_eq!(delta.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            previous = next;
        }
        let history = TableId::AirWings.last_change();
        assert_eq!(history.fields(TableId::AirWings, 98), 15);
        assert_eq!(history.fields(TableId::AirWings, 99), 16);
    }

    #[test]
    fn transport_operand_deltas_preserve_null_fault_full_handle_and_resolution() {
        let mut previous = AirWingRow {
            id: 0x1234_0000_0045,
            pool_subunit: Recorded::Value(None),
            raid_reference: Recorded::Value(None),
            ..Default::default()
        };
        for pool_subunit in [Some(0), Some(u32::MAX), None] {
            let next = AirWingRow {
                pool_subunit: Recorded::Value(pool_subunit),
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 16);
            assert_eq!(delta.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            previous = next;
        }
        for raid_reference in [
            Some(AirWingRaidReference {
                raw: 0,
                resolves: false,
            }),
            Some(AirWingRaidReference {
                raw: u64::MAX,
                resolves: false,
            }),
            Some(AirWingRaidReference {
                raw: u64::MAX,
                resolves: true,
            }),
            None,
        ] {
            let next = AirWingRow {
                raid_reference: Recorded::Value(raid_reference),
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 17);
            let mut wire = Vec::new();
            encode_delta(TableId::AirWings, &delta, 0, &mut wire).unwrap();
            let decoded = decode_delta(TableId::AirWings, 0, &mut Decoder::new(&wire)).unwrap();
            assert_eq!(decoded.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            previous = next;
        }
    }

    #[test]
    fn air_scenario_operand_snapshots_and_deltas_keep_zero_fault_and_signed_words() {
        let mut wing = AirWingRow {
            id: 7,
            ..Default::default()
        };
        let mut mission = AirMissionInputRow {
            wing: 7,
            ..Default::default()
        };
        for value in [Some(0), Some(i64::MIN), Some(i64::MAX), None] {
            let next = AirWingRow {
                strategic_bombing_raw: value.into(),
                ..wing.clone()
            };
            let delta = next.delta(Some(&wing)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 18);
            assert_eq!(delta.apply::<AirWingRow>(Some(wing)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            wing = next;
            let next = AirMissionInputRow {
                cas_ratio_raw: value.into(),
                ..mission.clone()
            };
            let delta = next.delta(Some(&mission)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 6);
            assert_eq!(
                delta.apply::<AirMissionInputRow>(Some(mission)).unwrap(),
                next
            );
            round_trip(TableId::AirMissionInputs, next.clone());
            mission = next;
        }
        let mut history = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            enemy_ships_sunk_raw: 1,
            ..Default::default()
        };
        for values in [
            Some([i64::MIN, i64::MAX, -1, 0, 3, 44, 65]),
            Some([0; 7]),
            None,
        ] {
            let next = AirNavalPatrolHistoryRow {
                scenario_channels: values.into(),
                ..history.clone()
            };
            let delta = next.delta(Some(&history)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 2);
            assert_eq!(
                delta
                    .apply::<AirNavalPatrolHistoryRow>(Some(history))
                    .unwrap(),
                next
            );
            round_trip(TableId::AirNavalPatrolHistories, next.clone());
            history = next;
        }
        let witness = AirNavalPatrolHistoryRow::availability_witness(5000);
        assert!(witness.is_availability_witness());
        assert!(!witness.has_values());
        round_trip(TableId::AirNavalPatrolHistories, witness);
        let refused = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            scenario_channels: Recorded::Value(None),
            ..Default::default()
        };
        assert!(refused.has_values());
        round_trip(TableId::AirNavalPatrolHistories, refused.clone());
        let zero = AirNavalPatrolHistoryRow {
            scenario_channels: Recorded::Value(Some([0; 7])),
            ..refused
        };
        assert!(!zero.has_values());
        assert_ne!(zero, refused);
        let table = TableId::AirNavalPatrolHistories;
        let gap = table
            .last_change()
            .table_gap(table, compatibility::WIDGET_CAPTURE_FORMAT)
            .unwrap();
        assert!(gap.contains(&format!(
            "layout or meaning changed at format {}",
            compatibility::AIR_HISTORY_SLOTS_FORMAT
        )));
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
                speed: i32::MIN,
                debug_current_ref_id: Some(0),
                unit: Some(i32::MIN),
                order_index: None,
                front_index: Some(i32::MAX),
                theatre_index: Some(0),
                theater_group_index: Some(-1),
                military_deployment_line_index: None,
                military_deployment_conveyor_index: Some(0),
                equipment_variant_index: Some(0),
                country_leader_index: Some(0),
                navy_id: Some(0),
                land_combat_id: Some(0),
                multiplayer_random_count: Some(i32::MAX),
            },
            ClockRow {
                hour: i32::MAX,
                play_time: u64::MAX,
                play_session_started_at: i64::MAX,
                seed: i32::MAX,
                speed: i32::MAX,
                debug_current_ref_id: Some(u32::MAX),
                unit: Some(i32::MAX),
                order_index: Some(i32::MIN),
                front_index: None,
                theatre_index: Some(-1),
                theater_group_index: None,
                military_deployment_line_index: Some(i32::MIN),
                military_deployment_conveyor_index: Some(1),
                equipment_variant_index: Some(-1),
                country_leader_index: Some(1),
                navy_id: Some(1),
                land_combat_id: Some(1),
                multiplayer_random_count: Some(i32::MIN),
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
                format: crate::FORMAT_VERSION,
                recorded_fields: table.field_count(),
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
            4,
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
                format: crate::FORMAT_VERSION,
                recorded_fields: table.field_count(),
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
        let mut unchanged = vec![91, 92];
        assert!(invalid.encode_change(None, 0, &mut unchanged).is_err());
        let valid = NavalCombatParticipantRow::default();
        assert_eq!(
            valid
                .encode_change(Some(&invalid), 0, &mut unchanged)
                .unwrap_err()
                .to_string(),
            valid.delta(Some(&invalid)).unwrap_err().to_string(),
        );
        assert_eq!(unchanged, [91, 92]);
        let attacker = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0001,
            battle_slot: u32::MAX,
            ..NavalCombatParticipantRow::default()
        };
        let defender = NavalCombatParticipantRow {
            side: 1,
            ..attacker
        };
        let other_type = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0002,
            ..attacker
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
        for mask in [0, TableId::Countries.full_mask() + 1] {
            let bad = DeltaRow {
                format: crate::FORMAT_VERSION,
                recorded_fields: TableId::Countries.field_count(),
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

}

#[cfg(test)]
#[path = "rows/format99/tests.rs"]
pub(crate) mod format99_tests;

#[cfg(test)]
#[path = "rows/political_runtime/tests.rs"]
mod political_runtime_tests;

#[cfg(test)]
#[path = "rows/intel_inputs/tests.rs"]
pub(crate) mod intel_inputs_tests;

#[test]
fn raid_memo_table_metadata_uses_the_dispatcher_key() {
    assert_eq!(TableId::RaidPathDistances.key_bits(), 65);
    let land = RaidPathDistanceRow {
        source: u32::MAX,
        target: u32::MAX,
        ..Default::default()
    };
    let naval = RaidPathDistanceRow {
        dispatcher: RaidPathDispatcher::Naval,
        ..land
    };
    assert_eq!(land.key(), u64::MAX as u128);
    assert_eq!(naval.key(), (1u128 << 65) - 1);
}

#[cfg(test)]
#[path = "rows/ship_names_tests.rs"]
mod ship_names_tests;
#[cfg(test)]
#[path = "rows/visit_wire_tests.rs"]
mod visit_wire_tests;

#[cfg(test)]
#[path = "rows/split_tests.rs"]
mod split_tests;

#[cfg(any(test, feature = "test-support"))]
pub use fixtures::sample_extended_table;

#[cfg(any(test, feature = "test-support"))]
#[path = "extended_rows/fixtures.rs"]
mod fixtures;

#[cfg(test)]
mod registry_tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn registry_holds_every_table_as_before_the_split() {
        let mut listed = String::new();
        for table in TableId::ALL {
            let fields: Vec<String> = (table.schema().iter().enumerate())
                .map(|(field, kind)| format!("{}:{kind:?}", table.field_name(field).unwrap()))
                .collect();
            writeln!(listed, "{} {table:?} {} {}", table as u16, table.key_bits(), fields.join(",")).unwrap();
        }
        let expected = include_str!("rows/registry.golden");
        assert_eq!(listed.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>());
    }
}
