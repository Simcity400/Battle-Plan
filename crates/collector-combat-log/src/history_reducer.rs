use engine_land_inputs::{combat_log_events as event, combat_log_history as data};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Group {
    pub country: u32,
    pub reference: u64,
    pub ordinal: u32,
    pub months: [Vec<(u64, data::EquipmentMonth)>; 6],
    pub manpower: Vec<(u64, data::ManpowerMonth)>,
    pub templates: Vec<(u64, data::TemplateMonth)>,
    pub references: Vec<data::BattleReference>,
    pub indices: BTreeMap<(u8, u64), usize>,
}
#[derive(Default)]
pub struct Store {
    pub groups: BTreeMap<u64, Group>,
    pub battles: BTreeMap<u32, data::Battle>,
    pub order: BTreeMap<(u32, u32), u64>,
    pub members: BTreeMap<u32, usize>,
    pub touched_references: BTreeSet<u32>,
    pub(crate) changed_counts: Vec<u32>,
    pub(crate) pending_battles: BTreeMap<u64, data::Battle>,
}
fn update<'a, T: Default>(
    rows: &'a mut Vec<(u64, T)>,
    indices: &mut BTreeMap<(u8, u64), usize>,
    kind: u8,
    pointer: u64,
) -> Result<(&'a mut T, bool), &'static str> {
    if let Some(&index) = indices.get(&(kind, pointer)) {
        return Ok((&mut rows[index].1, false));
    }
    engine_allocation::reserve(rows, 1).map_err(|_| "combat log reducer allocation failed")?;
    indices.insert((kind, pointer), rows.len());
    rows.push((pointer, T::default()));
    Ok((&mut rows.last_mut().unwrap().1, true))
}
fn append<T>(rows: &mut Vec<T>, ordinal: u64, value: T) -> Result<(), &'static str> {
    if ordinal != rows.len() as u64 {
        return Err("combat log event ordinal differs");
    }
    engine_allocation::reserve(rows, 1).map_err(|_| "combat log reducer allocation failed")?;
    rows.push(value);
    Ok(())
}
fn prefix<T>(rows: &mut Vec<T>, count: u64) -> Result<(), &'static str> {
    let count = usize::try_from(count).map_err(|_| "combat log prune count overflow")?;
    if count > rows.len() {
        return Err("combat log prune exceeds retained rows");
    }
    rows.drain(..count);
    Ok(())
}
impl Store {
    pub fn clear(&mut self) {
        self.groups.clear();
        self.battles.clear();
        self.order.clear();
        self.members.clear();
        self.touched_references.clear();
        self.pending_battles.clear();
        self.changed_counts.clear();
    }
    fn group(&mut self, key: u64) -> Result<&mut Group, &'static str> {
        self.groups
            .get_mut(&key)
            .ok_or("combat log event receiver not seeded")
    }
    fn side(&mut self, token: u64, side: u64) -> Result<&mut data::SideData, &'static str> {
        let battle = self
            .pending_battles
            .get_mut(&token)
            .ok_or("combat log event battle not begun")?;
        match side {
            0 => Ok(&mut battle.attacker),
            1 => Ok(&mut battle.defender),
            _ => Err("combat log side unsupported"),
        }
    }
    fn pool(&mut self, token: u64, key: u64) -> Result<&mut data::Pool, &'static str> {
        let side = self.side(token, key / 3)?;
        match key % 3 {
            0 => Ok(&mut side.equipment),
            1 => Ok(&mut side.captured_by_enemy),
            _ => Ok(&mut side.recovered),
        }
    }
    pub fn apply(&mut self, fact: event::Fact) -> Result<bool, &'static str> {
        let mut changed = true;
        let event::Fact {
            kind,
            root,
            item,
            values: v,
        } = fact;
        match kind {
            event::GROUP => {
                if let Some(group) = self.groups.get(&root) {
                    if (group.country, group.reference) != (v[0] as u32, v[1]) {
                        return Err("combat log receiver reused without destruction");
                    }
                } else {
                    self.order.insert((v[0] as u32, v[2] as u32), root);
                    self.groups.insert(
                        root,
                        Group {
                            country: v[0] as u32,
                            reference: v[1],
                            ordinal: v[2] as u32,
                            ..Default::default()
                        },
                    );
                }
            }
            event::MONTH => {
                let group = self.group(root)?;
                let rows = group
                    .months
                    .get_mut(v[0] as usize)
                    .ok_or("combat log month kind unsupported")?;
                let (month, inserted) = update(rows, &mut group.indices, v[0] as u8, item)?;
                changed = inserted
                    || (month.kind, month.reason, month.date, month.allow_zero)
                        != (v[0] as u8, v[1] as i32, v[2] as i32, v[3] != 0);
                month.kind = v[0] as u8;
                month.reason = v[1] as i32;
                month.date = v[2] as i32;
                month.allow_zero = v[3] != 0;
                month.enemy.clear();
            }
            event::ENEMY_EQUIPMENT => {
                let group = self.group(root)?;
                let index = *group
                    .indices
                    .get(&(5, item))
                    .ok_or("combat log enemy month not begun")?;
                let month = &mut group.months[5][index];
                append(
                    &mut month.1.enemy,
                    v[0],
                    data::Equipment {
                        variant: v[1],
                        amount_raw: v[2] as i64,
                    },
                )?;
            }
            event::MANPOWER => {
                let value = data::ManpowerMonth {
                    reason: v[0] as i32,
                    date: v[1] as i32,
                    losses: [v[2] as i32, v[3] as i32, v[4] as i32],
                };
                let group = self.group(root)?;
                let (target, inserted) = update(&mut group.manpower, &mut group.indices, 6, item)?;
                changed = inserted || *target != value;
                *target = value;
            }
            event::TEMPLATE => {
                let value = data::TemplateMonth {
                    template: v[0],
                    date: v[1] as i32,
                    wins: v[2] as i32,
                    total: v[3] as i32,
                };
                let group = self.group(root)?;
                let (target, inserted) = update(&mut group.templates, &mut group.indices, 7, item)?;
                changed = inserted || *target != value;
                *target = value;
            }
            event::REFERENCE => {
                let id = v[0] as u32;
                *self.members.entry(id).or_default() += 1;
                self.touched_references.insert(id);
                append(
                    &mut self.group(root)?.references,
                    item,
                    data::BattleReference {
                        id: v[0] as i32,
                        attacker: v[1] != 0,
                    },
                )?;
            }
            event::PREFIX_REMOVED => {
                if item == 8 {
                    let count =
                        usize::try_from(v[0]).map_err(|_| "combat log prune count overflow")?;
                    let group = self
                        .groups
                        .get(&root)
                        .ok_or("combat log event receiver not seeded")?;
                    if count > group.references.len() {
                        return Err("combat log prune exceeds retained rows");
                    }
                    for r in &group.references[..count] {
                        let n = self
                            .members
                            .get_mut(&(r.id as u32))
                            .ok_or("combat log missing reverse reference")?;
                        *n = n
                            .checked_sub(1)
                            .ok_or("combat log reverse reference underflow")?;
                    }
                }
                let group = self.group(root)?;
                macro_rules! prune_indexed {
                    ($rows:expr) => {{
                        let rows = $rows;
                        let count =
                            usize::try_from(v[0]).map_err(|_| "combat log prune count overflow")?;
                        if count > rows.len() {
                            return Err("combat log prune exceeds retained rows");
                        }
                        for &(pointer, _) in &rows[..count] {
                            group.indices.remove(&(item as u8, pointer));
                        }
                        prefix(rows, v[0])?;
                        for (index, &(pointer, _)) in rows.iter().enumerate() {
                            group.indices.insert((item as u8, pointer), index);
                        }
                    }};
                }
                match item {
                    0..=5 => prune_indexed!(&mut group.months[item as usize]),
                    6 => prune_indexed!(&mut group.manpower),
                    7 => prune_indexed!(&mut group.templates),
                    8 => prefix(&mut group.references, v[0])?,
                    _ => return Err("combat log prune kind unsupported"),
                }
            }
            event::GROUP_REMOVED => {
                let removed = self
                    .groups
                    .remove(&root)
                    .ok_or("combat log destroyed receiver not seeded")?;
                for r in &removed.references {
                    let n = self
                        .members
                        .get_mut(&(r.id as u32))
                        .ok_or("combat log missing reverse reference")?;
                    *n = n
                        .checked_sub(1)
                        .ok_or("combat log reverse reference underflow")?;
                }
                self.order.remove(&(removed.country, removed.ordinal));
                if let Some((&(country, ordinal), &pointer)) = self
                    .order
                    .range((removed.country, 0)..=(removed.country, u32::MAX))
                    .next_back()
                    && ordinal > removed.ordinal
                {
                    self.order.remove(&(country, ordinal));
                    self.order.insert((country, removed.ordinal), pointer);
                    self.groups.get_mut(&pointer).unwrap().ordinal = removed.ordinal;
                }
            }
            event::BATTLE => {
                if self
                    .pending_battles
                    .insert(
                        root,
                        data::Battle {
                            date: v[0] as i32,
                            province: v[1] as u32,
                            flags: (v[2] as u32).to_le_bytes(),
                            ..Default::default()
                        },
                    )
                    .is_some()
                {
                    return Err("combat log battle token repeated");
                }
            }
            event::SIDE => {
                let side = self.side(root, item)?;
                side.manpower = v[0] as i32;
                side.air_factor_raw = v[1] as i64;
                side.leader = v[2];
            }
            event::POOL => {
                let pool = self.pool(root, item)?;
                pool.serialized = v[0] != 0;
                pool.allow_zero = v[1] != 0;
                pool.equipment.clear();
            }
            event::EQUIPMENT => append(
                &mut self.pool(root, item)?.equipment,
                v[0],
                data::Equipment {
                    variant: v[1],
                    amount_raw: v[2] as i64,
                },
            )?,
            event::MODIFIER => append(
                &mut self.side(root, item)?.modifiers,
                v[0],
                data::ModifierHours {
                    modifier: v[1] as i32,
                    hours: v[2] as i32,
                },
            )?,
            event::COUNTRY => append(&mut self.side(root, item)?.countries, v[0], v[1] as u32)?,
            event::SLOT => {
                let battle = self
                    .pending_battles
                    .remove(&root)
                    .ok_or("combat log allocated battle not staged")?;
                if self
                    .battles
                    .get(&(item as u32))
                    .is_some_and(|b| b.ref_count != 0)
                    && !self.changed_counts.contains(&(item as u32))
                {
                    return Err("combat log occupied battle slot replaced");
                }
                self.battles.insert(item as u32, battle);
                engine_allocation::reserve_push_batch(&mut self.changed_counts, 1)
                    .map_err(|_| "combat log count demand allocation failed")?;
                self.changed_counts.push(item as u32);
            }
            event::COUNT | event::RELEASE => {
                if !self.battles.contains_key(&(root as u32)) {
                    return Err("combat log reference changed unstaged battle");
                }
                engine_allocation::reserve_push_batch(&mut self.changed_counts, 1)
                    .map_err(|_| "combat log count demand allocation failed")?;
                self.changed_counts.push(root as u32);
            }
            event::RESET => self.clear(),
            _ => return Err("combat log event kind unsupported"),
        }
        Ok(changed)
    }
    pub fn validate(&self) -> Result<usize, &'static str> {
        if !self.pending_battles.is_empty() {
            return Err("combat log allocation has no returned slot");
        }
        let mut visited = 0;
        for &id in self
            .touched_references
            .iter()
            .chain(self.changed_counts.iter())
        {
            visited += 1;
            if self.members.get(&id).copied().unwrap_or(0) != 0
                && self.battles.get(&id).is_none_or(|b| b.ref_count <= 0)
            {
                return Err("combat log group references unoccupied battle slot");
            }
        }
        Ok(visited)
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
    fn combat_log_pruning_pointer_reuse_repeated_references_and_slot_reuse_preserve_lifetimes() {
        let mut store = Store::default();
        store
            .apply(fact(event::GROUP, 100, 0, [7, 53 | 9 << 32, 0, 0, 0]))
            .unwrap();
        store
            .apply(fact(event::MONTH, 100, 200, [5, 0, 60_000_001, 1, 0]))
            .unwrap();
        store
            .apply(fact(event::MONTH, 100, 201, [5, 0, 60_000_721, 0, 0]))
            .unwrap();
        store
            .apply(fact(
                event::ENEMY_EQUIPMENT,
                100,
                201,
                [0, 70 | 123 << 32, i64::MIN as u64, 0, 0],
            ))
            .unwrap();
        store
            .apply(fact(event::PREFIX_REMOVED, 100, 5, [1, 0, 0, 0, 0]))
            .unwrap();
        assert_eq!(store.groups[&100].months[5][0].0, 201);
        for ordinal in 0..2 {
            store
                .apply(fact(event::REFERENCE, 100, ordinal, [4, 1, 0, 0, 0]))
                .unwrap();
        }
        store
            .apply(fact(
                event::BATTLE,
                90,
                0,
                [60_000_722, 8, 0x01000100, 0, 0],
            ))
            .unwrap();
        store.apply(fact(event::SLOT, 90, 4, [0; 5])).unwrap();
        store
            .apply(fact(event::COUNT, 4, 0, [2, 0, 0, 0, 0]))
            .unwrap();
        store.battles.get_mut(&4).unwrap().ref_count = 2;
        store.changed_counts.clear();
        store.validate().unwrap();
        store.apply(fact(event::COUNT, 4, 0, [0; 5])).unwrap();
        store
            .apply(fact(event::PREFIX_REMOVED, 100, 8, [2, 0, 0, 0, 0]))
            .unwrap();
        store
            .apply(fact(event::BATTLE, 91, 0, [60_001_000, 99, 0, 0, 0]))
            .unwrap();
        store.apply(fact(event::SLOT, 91, 4, [0; 5])).unwrap();
        assert_eq!(store.battles[&4].province, 99);
        store.changed_counts.clear();
        store
            .apply(fact(event::GROUP_REMOVED, 100, 0, [0; 5]))
            .unwrap();
        store
            .apply(fact(event::GROUP, 100, 0, [7, 53 | 10 << 32, 0, 0, 0]))
            .unwrap();
        assert!(store.groups[&100].months.iter().all(Vec::is_empty));
        store.validate().unwrap();
    }
    #[test]
    fn combat_log_occupied_slot_replacement_and_missing_publication_are_refused() {
        let mut store = Store::default();
        store.apply(fact(event::BATTLE, 1, 0, [0; 5])).unwrap();
        assert!(store.validate().is_err());
        store.apply(fact(event::SLOT, 1, 4, [0; 5])).unwrap();
        store
            .apply(fact(event::COUNT, 4, 0, [1, 0, 0, 0, 0]))
            .unwrap();
        store.battles.get_mut(&4).unwrap().ref_count = 1;
        store.changed_counts.clear();
        store.apply(fact(event::BATTLE, 2, 0, [0; 5])).unwrap();
        assert!(store.apply(fact(event::SLOT, 2, 4, [0; 5])).is_err());
        assert!(
            store
                .apply(fact(event::REFERENCE, 100, 0, [4, 0, 0, 0, 0]))
                .is_err()
        );
    }
    #[test]
    fn combat_log_unchanged_month_headers_and_counters_do_not_request_new_snapshots() {
        let mut store = Store::default();
        store
            .apply(fact(event::GROUP, 1, 0, [7, 53, 0, 0, 0]))
            .unwrap();
        let month = fact(event::MONTH, 1, 2, [0, 0, 60_000_001, 0, 0]);
        assert!(store.apply(month).unwrap());
        assert!(!store.apply(month).unwrap());
        let manpower = fact(event::MANPOWER, 1, 3, [0, 60_000_001, 7, 0, 0]);
        assert!(store.apply(manpower).unwrap());
        assert!(!store.apply(manpower).unwrap());
        assert!(
            store
                .apply(fact(event::PREFIX_REMOVED, 1, 0, [1, 0, 0, 0, 0]))
                .unwrap()
        );
        assert!(
            store.apply(month).unwrap(),
            "reused month address is new membership after pruning"
        );
    }
}
