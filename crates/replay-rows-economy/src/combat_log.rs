use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogGroupRow {
    pub country: u32,
    pub group: u64,
    pub ordinal: u32,
}
typed_codec!(CombatLogGroupRow, 96, |row| (row.country as u128) << 64 | row.group as u128, |key| Self { country: (key >> 64) as u32, group: key as u64, ..Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogBattleCountryRow {
    pub battle: u32,
    pub side: u8,
    pub ordinal: u32,
    pub country: u32,
}
typed_codec!(CombatLogBattleCountryRow, 72, |row| (row.battle as u128) << 40 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { battle: (key >> 40) as u32, side: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (country): U32 }, |_row| Ok(()));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogObserverRow {
    pub combat: u64,
    pub attacker: bool,
    pub ordinal: u64,
    pub kind: u8,
    pub item: u64,
    pub index: u64,
    pub identity: u64,
    pub identity_2: u64,
    pub identity_3: u64,
    pub a: i64,
    pub b: i64,
    pub c: i64,
    pub d: i64,
    pub e: i64,
}
typed_codec!(CombatLogObserverRow, 128,
 |row| (row.combat as u128) << 64 | (row.ordinal as u128) << 1 | row.attacker as u128,
 |key| Self { combat:(key >> 64) as u64, attacker:key & 1 != 0, ordinal:(key as u64) >> 1, ..Default::default() },
 {0 => (kind): U8, 1 => (item): U64, 2 => (index): U64,
  3 => (identity): U64, 4 => (identity_2): U64, 5 => (identity_3): U64,
  6 => (a): I64, 7 => (b): I64, 8 => (c): I64, 9 => (d): I64, 10 => (e): I64},
 |row| if row.ordinal > u64::MAX / 2 { Err(invalid("combat log observer ordinal overflow")) } else { Ok(()) });

impl CombatLogObserverRow {
    pub fn from_fact(
        combat: u64,
        attacker: bool,
        kind: u8,
        item: u64,
        values: [u64; 5],
    ) -> Result<Self, &'static str> {
        use CombatLogObserverKind as K;
        let [a, b, c, d, e] = values;
        let mut row = Self {
            combat,
            attacker,
            kind,
            item,
            ..Self::default()
        };
        match kind {
            k if k == K::State as u8 || k == K::Pool as u8 || k == K::Counters as u8 => {
                [row.a, row.b, row.c, row.d, row.e] = values.map(|v| v as i64);
            }
            k if k == K::Side as u8 => {
                row.a = b as i64;
                row.identity = c;
            }
            k if k == K::Equipment as u8 || k == K::ActivityDamage as u8 => {
                row.index = a;
                row.identity = b;
                row.a = c as i64;
            }
            k if k == K::Modifier as u8 => {
                row.index = a;
                row.a = b as i64;
                row.b = c as i64;
            }
            k if k == K::Country as u8 || k == K::ActivityTemplate as u8 => {
                row.index = a;
                row.identity = b;
            }
            k if k == K::Leader as u8 => {
                row.identity = a;
                row.a = b as i64;
            }
            k if k == K::Damage as u8 || k == K::Activity as u8 => {
                row.identity = a;
                row.identity_2 = b;
                row.identity_3 = c;
                row.a = d as i64;
            }
            k if k == K::ActivityEquipment as u8 => {
                row.index = a;
                if a == u64::MAX {
                    row.a = b as i64;
                    row.b = c as i64;
                } else {
                    row.identity = b;
                    row.a = c as i64;
                }
            }
            _ => return Err("combat log observer kind unsupported"),
        }
        if row.fact() != (kind, item, [a, b, c, d, e]) {
            return Err("combat log observer payload shape differs");
        }
        Ok(row)
    }

    pub fn fact(&self) -> (u8, u64, [u64; 5]) {
        use CombatLogObserverKind as K;
        let [a, b, c, d, e] = [self.a, self.b, self.c, self.d, self.e].map(|v| v as u64);
        let values = match self.kind {
            k if k == K::State as u8 || k == K::Pool as u8 || k == K::Counters as u8 => {
                [a, b, c, d, e]
            }
            k if k == K::Side as u8 => [0, a, self.identity, 0, 0],
            k if k == K::Equipment as u8 || k == K::ActivityDamage as u8 => {
                [self.index, self.identity, a, 0, 0]
            }
            k if k == K::Modifier as u8 => [self.index, a, b, 0, 0],
            k if k == K::Country as u8 || k == K::ActivityTemplate as u8 => {
                [self.index, self.identity, 0, 0, 0]
            }
            k if k == K::Leader as u8 => [self.identity, a, 0, 0, 0],
            k if k == K::Damage as u8 || k == K::Activity as u8 => {
                [self.identity, self.identity_2, self.identity_3, a, 0]
            }
            k if k == K::ActivityEquipment as u8 && self.index == u64::MAX => {
                [self.index, a, b, 0, 0]
            }
            k if k == K::ActivityEquipment as u8 => [self.index, self.identity, a, 0, 0],
            _ => [a, b, c, d, e],
        };
        (self.kind, self.item, values)
    }

    pub fn semantic_key(&self) -> (u64, bool, u8, u64, u64) {
        (self.combat, self.attacker, self.kind, self.item, self.index)
    }

    pub fn logical_order(&self) -> (u8, u64, u8, u64) {
        use CombatLogObserverKind as K;
        match self.kind {
            k if k == K::State as u8 => (0, 0, 0, 0),
            k if k == K::Counters as u8 => (1, self.item, 0, 0),
            k if k == K::Side as u8 => (2, 0, 0, 0),
            k if k == K::Pool as u8 => (3, self.item, 0, 0),
            k if k == K::Equipment as u8 => (3, self.item, 1, self.index),
            k if k == K::Modifier as u8 => (4, 0, 0, self.index),
            k if k == K::Country as u8 => (5, 0, 0, self.index),
            k if k == K::Leader as u8 => (6, self.item, 0, 0),
            k if k == K::Damage as u8 => (7, self.item, 0, 0),
            k if k == K::Activity as u8 => (8, self.item, 0, 0),
            k if k == K::ActivityTemplate as u8 => (8, self.item, 1, self.index),
            k if k == K::ActivityDamage as u8 => (8, self.item, 2, self.index),
            k if k == K::ActivityEquipment as u8 && self.index == u64::MAX => (8, self.item, 3, 0),
            k if k == K::ActivityEquipment as u8 => (8, self.item, 4, self.index),
            _ => (u8::MAX, self.item, self.kind, self.index),
        }
    }
}

pub const COMBAT_LOG_VARIANT_TYPE: u32 = engine_reference_values::native_values::equipment_variant::REFERENCE_TYPE;

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combat_log_observer_numeric_lanes_reconstruct_native_words_exactly() {
        use CombatLogObserverKind as K;
        for word in [0, 1, u64::MAX, i64::MIN as u64, i64::MAX as u64] {
            for (kind, values) in [
                (K::State, [word, word, 3, 0, 0]),
                (K::Counters, [word; 5]),
                (K::Side, [0, word, word, 0, 0]),
                (K::Pool, [1, 1, 0, 0, 0]),
                (K::Equipment, [word, word, word, 0, 0]),
                (K::Modifier, [word, word, word, 0, 0]),
                (K::Country, [word, word, 0, 0, 0]),
                (K::Leader, [word, word, 0, 0, 0]),
                (K::Damage, [word, word, word, word, 0]),
                (K::Activity, [word, word, word, 0, 0]),
                (K::ActivityTemplate, [word, word, 0, 0, 0]),
                (K::ActivityDamage, [word, word, word, 0, 0]),
                (K::ActivityEquipment, [0, word, word, 0, 0]),
                (K::ActivityEquipment, [u64::MAX, word, word, 0, 0]),
            ] {
                let row = CombatLogObserverRow::from_fact(u64::MAX, true, kind as u8, word, values)
                    .unwrap();
                assert_eq!(row.fact(), (kind as u8, word, values));
            }
        }
    }
}
