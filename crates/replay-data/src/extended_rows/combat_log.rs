//! Retained native combat history; native array ordinals preserve repeats.
use super::*;
opaque_value!([u8; 4]);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogGroupRow {
    pub country: u32,
    pub group: u64,
    pub ordinal: u32,
}
typed_codec!(CombatLogGroupRow, 96, |row| (row.country as u128) << 64 | row.group as u128, |key| Self { country: (key >> 64) as u32, group: key as u64, ..Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogMonthRow {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub ordinal: u32,
    pub reason: i32,
    pub date: i32,
    pub allow_zero: bool,
}
typed_codec!(CombatLogMonthRow, 104, |row| (row.country as u128) << 72 | (row.group as u128) << 40 | (row.kind as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 72) as u32, group: (key >> 40) as u32, kind: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (reason): I32, 1 => (date): I32, 2 => (allow_zero): Bool }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogManpowerRow {
    pub country: u32,
    pub group: u32,
    pub ordinal: u32,
    pub reason: i32,
    pub date: i32,
    pub losses_0: i32,
    pub losses_1: i32,
    pub losses_2: i32,
}
typed_codec!(CombatLogManpowerRow, 96, |row| (row.country as u128) << 64 | (row.group as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (reason): I32, 1 => (date): I32, 2 => (losses_0): I32, 3 => (losses_1): I32, 4 => (losses_2): I32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogTemplateRow {
    pub country: u32,
    pub group: u32,
    pub ordinal: u32,
    pub template: u64,
    pub date: i32,
    pub wins: i32,
    pub total: i32,
}
typed_codec!(CombatLogTemplateRow, 96, |row| (row.country as u128) << 64 | (row.group as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (template): U64, 1 => (date): I32, 2 => (wins): I32, 3 => (total): I32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogReferenceRow {
    pub country: u32,
    pub group: u32,
    pub ordinal: u32,
    pub battle: i32,
    pub attacker: bool,
}
typed_codec!(CombatLogReferenceRow, 96, |row| (row.country as u128) << 64 | (row.group as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (battle): I32, 1 => (attacker): Bool }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogBattleRow {
    pub id: u32,
    pub reference_count: i32,
    pub date: i32,
    pub province: u32,
    pub flags: [u8; 4],
}
typed_codec!(CombatLogBattleRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
 { 0 => (reference_count): I32, 1 => (date): I32, 2 => (province): U32, 3 => (flags): Opaque }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogEnemyEquipmentRow {
    pub country: u32,
    pub group: u32,
    pub month: u32,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(CombatLogEnemyEquipmentRow, 128, |row| (row.country as u128) << 96 | (row.group as u128) << 64 | (row.month as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 96) as u32, group: (key >> 64) as u32, month: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (variant): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogBattleSideRow {
    pub battle: u32,
    pub side: u8,
    pub manpower: i32,
    pub air_factor_raw: i64,
    pub leader: u64,
    pub pool_flags: u8,
}
typed_codec!(CombatLogBattleSideRow, 40, |row| (row.battle as u128) << 8 | row.side as u128, |key| Self { battle: (key >> 8) as u32, side: key as u8, ..Default::default() },
 { 0 => (manpower): I32, 1 => (air_factor_raw): I64, 2 => (leader): U64, 3 => (pool_flags): U8 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogBattleEquipmentRow {
    pub battle: u32,
    pub side: u8,
    pub kind: u8,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(CombatLogBattleEquipmentRow, 80, |row| (row.battle as u128) << 48 | (row.side as u128) << 40 | (row.kind as u128) << 32 | row.ordinal as u128, |key| Self { battle: (key >> 48) as u32, side: (key >> 40) as u8, kind: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (variant): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogBattleModifierRow {
    pub battle: u32,
    pub side: u8,
    pub ordinal: u32,
    pub modifier: i32,
    pub hours: i32,
}
typed_codec!(CombatLogBattleModifierRow, 72, |row| (row.battle as u128) << 40 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { battle: (key >> 40) as u32, side: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (modifier): I32, 1 => (hours): I32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogBattleCountryRow {
    pub battle: u32,
    pub side: u8,
    pub ordinal: u32,
    pub country: u32,
}
typed_codec!(CombatLogBattleCountryRow, 72, |row| (row.battle as u128) << 40 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { battle: (key >> 40) as u32, side: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (country): U32 }, |_row| Ok(()));

/// Ordered active observer operands. `kind` names a native stored collection;
/// `item` is that collection's ordinal (or activity ordinal for its children).
/// No field contains a native pointer. Aggregate manpower/equipment operands
/// reuse LandCombatProgress, acquired by the same census visitor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatLogObserverRow {
    pub combat: u64,
    pub attacker: bool,
    pub ordinal: u64,
    pub kind: u8,
    pub item: u64,
    pub a: u64,
    pub b: u64,
    pub c: u64,
    pub d: u64,
    pub e: u64,
}
typed_codec!(CombatLogObserverRow, 128,
 |row| (row.combat as u128) << 64 | (row.ordinal as u128) << 1 | row.attacker as u128,
 |key| Self { combat:(key >> 64) as u64, attacker:key & 1 != 0, ordinal:(key as u64) >> 1, ..Default::default() },
 {0 => (kind): U8, 1 => (item): U64, 2 => (a): U64, 3 => (b): U64, 4 => (c): U64, 5 => (d): U64, 6 => (e): U64},
 |row| if row.ordinal > u64::MAX / 2 { Err(invalid("combat log observer ordinal overflow")) } else { Ok(()) });

/// Native full equipment-reference type used by the existing own-loss keys.
pub const COMBAT_LOG_VARIANT_TYPE: u32 = engine_native::cequipment_variant::REFERENCE_TYPE;

/// Stored active-observer operand kinds. Item and payload meanings follow the
/// native collection: side, pool, equipment, modifier, country, observer state,
/// counter block, leader hours, damage attribution, and activity descendants.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatLogObserverKind {
    Side = 10,
    Pool = 11,
    Equipment = 12,
    Modifier = 13,
    Country = 14,
    State = 20,
    Counters = 21,
    Leader = 22,
    Damage = 23,
    Activity = 24,
    ActivityTemplate = 25,
    ActivityDamage = 26,
    ActivityEquipment = 27,
}
