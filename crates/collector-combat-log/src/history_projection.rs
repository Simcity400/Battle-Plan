use crate::history_capture::{N, Views};
use crate::history_reducer::{Group, Store};
use engine_land_inputs::{combat_log_events as event, combat_log_history as native};
use replay_data::rows::*;
use std::{collections::BTreeMap, sync::Arc};

macro_rules! held_rows {
    ($(($index:literal, $variant:ident, $row:ty)),+ $(,)?) => {
        #[derive(Clone)]
        #[expect(
            clippy::enum_variant_names,
            reason = "Variants share the replay table names used by the generated conversions."
        )]
        enum Row { $($variant($row)),+ }
        $(impl From<$row> for Row { fn from(value: $row) -> Self { Self::$variant(value) } })+
        impl Row {
            fn key(&self) -> (usize, u128) { match self { $(Self::$variant(row) => ($index, row.key())),+ } }
        }
        fn materialize(index: usize, rows: &BTreeMap<u128, Row>) -> Result<TableRows, &'static str> {
            match index { $($index => engine_allocation::collect_vec(rows.values().map(|row| {
                let Row::$variant(row) = row else { unreachable!("held Combat Log row type") };
                row.clone()
            })).map(TableRows::$variant).map_err(|_| "combat log row materialisation allocation failed")),+, _ => unreachable!("held Combat Log table") }
        }
    };
}
held_rows! {
    (0, CombatLogGroups, CombatLogGroupRow),
    (1, CombatLogMonths, CombatLogMonthRow),
    (2, CombatLogManpower, CombatLogManpowerRow),
    (3, CombatLogTemplates, CombatLogTemplateRow),
    (4, CombatLogReferences, CombatLogReferenceRow),
    (5, CombatLogBattles, CombatLogBattleRow),
    (6, CombatLogEnemyEquipment, CombatLogEnemyEquipmentRow),
    (7, CombatLogBattleSides, CombatLogBattleSideRow),
    (8, CombatLogBattleEquipment, CombatLogBattleEquipmentRow),
    (9, CombatLogBattleModifiers, CombatLogBattleModifierRow),
    (10, CombatLogBattleCountries, CombatLogBattleCountryRow),
}

pub struct Held {
    rows: [BTreeMap<u128, Row>; N],
    dirty: [bool; N],
    views: Views,
    pub edits: usize,
}
impl Held {
    pub const fn new() -> Self {
        Self {
            rows: [const { BTreeMap::new() }; N],
            dirty: [true; N],
            views: [const { Err("combat log conversion baseline unavailable") }; N],
            edits: 0,
        }
    }
    fn put(&mut self, row: impl Into<Row>) {
        let row = row.into();
        let (index, key) = row.key();
        self.rows[index].insert(key, row);
        self.dirty[index] = true;
        self.edits += 1;
    }
    fn remove(&mut self, index: usize, first: u128, last: u128) {
        while let Some((&key, _)) = self.rows[index].range(first..=last).next() {
            self.rows[index].remove(&key);
            self.edits += 1;
            self.dirty[index] = true;
        }
    }
    fn group_key(g: &Group, index: usize) -> (u128, u32) {
        let country = g.country as u128;
        let group = (g.reference >> 32) as u128;
        match index {
            0 => ((country << 64) | g.reference as u128, 0),
            1 => ((country << 72) | (group << 40), 40),
            6 => ((country << 96) | (group << 64), 64),
            _ => ((country << 64) | (group << 32), 32),
        }
    }
    fn remove_group_table(&mut self, g: &Group, index: usize) {
        let (first, bits) = Self::group_key(g, index);
        self.remove(index, first, first | ((1u128 << bits) - 1));
    }
    fn group(&mut self, g: &Group) {
        self.put(CombatLogGroupRow {
            country: g.country,
            group: g.reference,
            ordinal: g.ordinal,
        });
    }
    fn month(&mut self, g: &Group, kind: usize, ordinal: usize) {
        let m = &g.months[kind][ordinal].1;
        self.put(CombatLogMonthRow {
            country: g.country,
            group: (g.reference >> 32) as u32,
            kind: kind as u8,
            ordinal: ordinal as u32,
            reason: m.reason,
            date: m.date,
            allow_zero: m.allow_zero,
        });
    }
    fn enemy(&mut self, g: &Group, month: usize, ordinal: usize) {
        let e = &g.months[5][month].1.enemy[ordinal];
        self.put(CombatLogEnemyEquipmentRow {
            country: g.country,
            group: (g.reference >> 32) as u32,
            month: month as u32,
            ordinal: ordinal as u32,
            variant: e.variant,
            amount_raw: e.amount_raw,
        });
    }
    fn manpower(&mut self, g: &Group, ordinal: usize) {
        let m = &g.manpower[ordinal].1;
        self.put(CombatLogManpowerRow {
            country: g.country,
            group: (g.reference >> 32) as u32,
            ordinal: ordinal as u32,
            reason: m.reason,
            date: m.date,
            losses_0: m.losses[0],
            losses_1: m.losses[1],
            losses_2: m.losses[2],
        });
    }
    fn template(&mut self, g: &Group, ordinal: usize) {
        let t = &g.templates[ordinal].1;
        self.put(CombatLogTemplateRow {
            country: g.country,
            group: (g.reference >> 32) as u32,
            ordinal: ordinal as u32,
            template: t.template,
            date: t.date,
            wins: t.wins,
            total: t.total,
        });
    }
    fn reference(&mut self, g: &Group, ordinal: usize) {
        let r = &g.references[ordinal];
        self.put(CombatLogReferenceRow {
            country: g.country,
            group: (g.reference >> 32) as u32,
            ordinal: ordinal as u32,
            battle: r.id,
            attacker: r.attacker,
        });
    }
    fn collection(&mut self, g: &Group, kind: u64) {
        match kind {
            0..=5 => {
                for i in 0..g.months[kind as usize].len() {
                    self.month(g, kind as usize, i);
                    if kind == 5 {
                        for j in 0..g.months[5][i].1.enemy.len() {
                            self.enemy(g, i, j);
                        }
                    }
                }
            }
            6 => {
                for i in 0..g.manpower.len() {
                    self.manpower(g, i);
                }
            }
            7 => {
                for i in 0..g.templates.len() {
                    self.template(g, i);
                }
            }
            8 => {
                for i in 0..g.references.len() {
                    self.reference(g, i);
                }
            }
            _ => unreachable!("certified collection kind"),
        }
    }
    pub fn apply(&mut self, store: &mut Store, fact: event::Fact) -> Result<(), &'static str> {
        if fact.kind == event::RESET {
            *self = Self::new();
        }
        if fact.kind == event::GROUP_REMOVED {
            let g = store
                .groups
                .get(&fact.root)
                .ok_or("combat log destroyed receiver not seeded")?;
            let country = g.country;
            let ordinal = g.ordinal;
            for index in [0, 1, 2, 3, 4, 6] {
                self.remove_group_table(g, index);
            }
            store.apply(fact)?;
            if let Some(pointer) = store.order.get(&(country, ordinal)) {
                self.group(&store.groups[pointer]);
            }
            return Ok(());
        }
        if fact.kind == event::PREFIX_REMOVED {
            let g = store
                .groups
                .get(&fact.root)
                .ok_or("combat log prune receiver not seeded")?;
            match fact.item {
                0..=5 => {
                    let (base, _) = Self::group_key(g, 1);
                    let first = base | (fact.item as u128) << 32;
                    self.remove(1, first, first | u32::MAX as u128);
                    if fact.item == 5 {
                        self.remove_group_table(g, 6);
                    }
                }
                6..=8 => self.remove_group_table(g, (fact.item - 4) as usize),
                _ => return Err("combat log prune kind unsupported"),
            }
        }
        let changed = store.apply(fact)?;
        match fact.kind {
            event::GROUP => self.group(&store.groups[&fact.root]),
            event::MONTH => {
                let g = &store.groups[&fact.root];
                let kind = fact.values[0] as usize;
                let index = g.indices[&(kind as u8, fact.item)];
                if changed {
                    self.month(g, kind, index);
                }
                if kind == 5 {
                    let (base, _) = Self::group_key(g, 6);
                    let first = base | (index as u128) << 32;
                    self.remove(6, first, first | u32::MAX as u128);
                }
            }
            event::ENEMY_EQUIPMENT => {
                let g = &store.groups[&fact.root];
                self.enemy(g, g.indices[&(5, fact.item)], fact.values[0] as usize);
            }
            event::MANPOWER if changed => {
                let g = &store.groups[&fact.root];
                self.manpower(g, g.indices[&(6, fact.item)]);
            }
            event::TEMPLATE if changed => {
                let g = &store.groups[&fact.root];
                self.template(g, g.indices[&(7, fact.item)]);
            }
            event::REFERENCE => self.reference(&store.groups[&fact.root], fact.item as usize),
            event::PREFIX_REMOVED => self.collection(&store.groups[&fact.root], fact.item),
            _ => (),
        }
        Ok(())
    }
    pub fn battle(&mut self, id: u32, b: &native::Battle, details: bool) {
        if details || b.ref_count <= 0 {
            for (index, bits) in [(5, 0), (7, 8), (8, 48), (9, 40), (10, 40)] {
                let first = (id as u128) << bits;
                self.remove(index, first, first | ((1u128 << bits) - 1));
            }
        }
        if b.ref_count <= 0 {
            return;
        }
        self.put(CombatLogBattleRow {
            id,
            reference_count: b.ref_count,
            date: b.date,
            province: b.province,
            flags: b.flags,
        });
        if !details {
            return;
        }
        for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
            let side = side as u8;
            let pools = [&s.equipment, &s.captured_by_enemy, &s.recovered];
            let mut pool_flags = 0;
            for (i, p) in pools.iter().enumerate() {
                pool_flags |= (u8::from(p.serialized) | u8::from(p.allow_zero) << 1) << (i * 2);
            }
            self.put(CombatLogBattleSideRow {
                battle: id,
                side,
                manpower: s.manpower,
                air_factor_raw: s.air_factor_raw,
                leader: s.leader,
                pool_flags,
            });
            for (kind, p) in pools.into_iter().enumerate() {
                for (ordinal, e) in p.equipment.iter().enumerate() {
                    self.put(CombatLogBattleEquipmentRow {
                        battle: id,
                        side,
                        kind: kind as u8,
                        ordinal: ordinal as u32,
                        variant: e.variant,
                        amount_raw: e.amount_raw,
                    });
                }
            }
            for (ordinal, m) in s.modifiers.iter().enumerate() {
                self.put(CombatLogBattleModifierRow {
                    battle: id,
                    side,
                    ordinal: ordinal as u32,
                    modifier: m.modifier,
                    hours: m.hours,
                });
            }
            for (ordinal, &country) in s.countries.iter().enumerate() {
                self.put(CombatLogBattleCountryRow {
                    battle: id,
                    side,
                    ordinal: ordinal as u32,
                    country,
                });
            }
        }
    }
    pub fn snapshot(&mut self) -> Views {
        for index in 0..N {
            if self.dirty[index] {
                self.views[index] = materialize(index, &self.rows[index]).map(Arc::new);
                self.dirty[index] = self.views[index].is_err();
            }
        }
        self.views.clone()
    }
}

impl Default for Held {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fact(kind: u64, root: u64, item: u64, values: [u64; 5]) -> event::Fact {
        event::Fact {
            kind,
            root,
            item,
            values,
        }
    }
    #[test]
    fn combat_log_projection_carries_unchanged_own_month_and_replaces_an_enemy_pool_by_empty() {
        let mut store = Store::default();
        let mut held = Held::new();
        held.apply(
            &mut store,
            fact(event::GROUP, 1, 0, [7, 53 | 10 << 32, 0, 0, 0]),
        )
        .unwrap();
        let own = fact(event::MONTH, 1, 100, [0, 0, 60_000_001, 0, 0]);
        held.apply(&mut store, own).unwrap();
        let before = held.snapshot();
        let edits = held.edits;
        held.apply(&mut store, own).unwrap();
        let after = held.snapshot();
        assert_eq!(held.edits, edits);
        for index in [1, 6] {
            assert!(Arc::ptr_eq(
                before[index].as_ref().unwrap(),
                after[index].as_ref().unwrap()
            ));
        }
        let enemy = fact(event::MONTH, 1, 101, [5, 0, 60_000_001, 1, 0]);
        held.apply(&mut store, enemy).unwrap();
        held.apply(
            &mut store,
            fact(
                event::ENEMY_EQUIPMENT,
                1,
                101,
                [0, 7 << 32 | 70, i64::MIN as u64, 0, 0],
            ),
        )
        .unwrap();
        let before = held.snapshot();
        held.apply(&mut store, enemy).unwrap();
        let after = held.snapshot();
        assert!(Arc::ptr_eq(
            before[1].as_ref().unwrap(),
            after[1].as_ref().unwrap()
        ));
        assert!(!Arc::ptr_eq(
            before[6].as_ref().unwrap(),
            after[6].as_ref().unwrap()
        ));
        assert_eq!(after[6].as_ref().unwrap().len(), 0);
    }
    #[test]
    fn combat_log_projection_rekeys_only_pruned_collection_and_manager_swap_member() {
        let mut store = Store::default();
        let mut held = Held::new();
        for (root, group, ordinal) in [(1, 10, 0), (2, 11, 1)] {
            held.apply(
                &mut store,
                fact(event::GROUP, root, 0, [7, 53 | group << 32, ordinal, 0, 0]),
            )
            .unwrap();
        }
        held.apply(
            &mut store,
            fact(event::MONTH, 1, 100, [0, 0, 60_000_001, 0, 0]),
        )
        .unwrap();
        held.apply(
            &mut store,
            fact(event::MONTH, 1, 101, [0, 0, 60_000_721, 0, 0]),
        )
        .unwrap();
        held.apply(
            &mut store,
            fact(event::PREFIX_REMOVED, 1, 0, [1, 0, 0, 0, 0]),
        )
        .unwrap();
        assert_eq!(store.groups[&1].indices[&(0, 101)], 0);
        let views = held.snapshot();
        let TableRows::CombatLogMonths(months) = &**views[1].as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(months.len(), 1);
        assert_eq!((months[0].ordinal, months[0].date), (0, 60_000_721));
        held.apply(&mut store, fact(event::GROUP_REMOVED, 1, 0, [0; 5]))
            .unwrap();
        let views = held.snapshot();
        let TableRows::CombatLogGroups(groups) = &**views[0].as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].group, groups[0].ordinal), (53 | 11 << 32, 0));
        assert_eq!(views[1].as_ref().unwrap().len(), 0);
    }
    #[test]
    fn combat_log_projection_removes_freed_slot_payload_before_reusing_its_identity() {
        let mut held = Held::new();
        let mut battle = native::Battle {
            ref_count: 1,
            ..Default::default()
        };
        battle.attacker.countries = vec![11, 12];
        held.battle(4, &battle, true);
        battle.ref_count = 0;
        held.battle(4, &battle, false);
        battle.ref_count = 1;
        battle.attacker.countries = vec![13];
        held.battle(4, &battle, true);
        let views = held.snapshot();
        let TableRows::CombatLogBattleCountries(countries) = &**views[10].as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(countries.len(), 1);
        assert_eq!(countries[0].country, 13);
    }
}
