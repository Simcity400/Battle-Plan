use super::*;
use std::hash::Hash;

struct Group {
    slots: Vec<u32>,
    ordered: bool,
}

pub(super) struct Index<K> {
    positions: Vec<Option<(K, usize)>>,
    groups: HashMap<K, Group>,
}

impl<K> Default for Index<K> {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            groups: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash> Index<K> {
    pub fn clear(&mut self) {
        self.positions.clear();
        self.groups.clear();
    }

    pub fn remove(&mut self, slot: u32) {
        let Some(position) = self.positions.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        let (key, at) = position;
        let group = self.groups.get_mut(&key).expect("indexed slot group");
        group.slots.swap_remove(at);
        group.ordered = false;
        if let Some(&moved) = group.slots.get(at) {
            self.positions[moved as usize] = Some((key, at));
        }
        if group.slots.is_empty() {
            self.groups.remove(&key);
        }
    }

    pub fn insert(&mut self, slot: u32, key: K) {
        if self
            .positions
            .get(slot as usize)
            .copied()
            .flatten()
            .is_some_and(|old| old.0 == key)
        {
            return;
        }
        self.remove(slot);
        self.positions
            .resize(self.positions.len().max(slot as usize + 1), None);
        let group = self.groups.entry(key).or_insert_with(|| Group {
            slots: Vec::new(),
            ordered: false,
        });
        self.positions[slot as usize] = Some((key, group.slots.len()));
        group.slots.push(slot);
        group.ordered = false;
    }

    pub fn active(&self, slot: u32) -> bool {
        self.positions
            .get(slot as usize)
            .is_some_and(Option::is_some)
    }

    pub fn contains(&self, key: K) -> bool {
        self.groups.contains_key(&key)
    }

    pub fn get(&self, key: K) -> &[u32] {
        self.groups
            .get(&key)
            .map_or(&[], |group| group.slots.as_slice())
    }

    pub fn take(&mut self, key: K) -> Vec<u32> {
        let Some(group) = self.groups.remove(&key) else {
            return Vec::new();
        };
        for &slot in &group.slots {
            self.positions[slot as usize] = None;
        }
        group.slots
    }

    pub fn ordered(
        &mut self,
        state: &segments::State,
        table: TableId,
        key: K,
    ) -> io::Result<&[u32]> {
        let Some(group) = self.groups.get_mut(&key) else {
            return Ok(&[]);
        };
        if !group.ordered {
            let held = columns::table(state, table)?;
            group.slots.sort_unstable_by_key(|&slot| held.key(slot));
            for (at, &slot) in group.slots.iter().enumerate() {
                self.positions[slot as usize] = Some((key, at));
            }
            group.ordered = true;
        }
        Ok(&group.slots)
    }
}

pub(super) fn structural<K: Copy + Eq + Hash>(
    index: &mut Index<K>,
    state: &segments::State,
    table: TableId,
    reset: bool,
    partial: bool,
    key: impl Fn(u128) -> K,
) -> io::Result<()> {
    let readable =
        columns::complete(state, table) || (partial && columns::truncated(state, table).is_some());
    let changes = columns::changes(state, table)?;
    if reset || changes.truncated {
        index.clear();
        if readable {
            let held = columns::table(state, table)?;
            for slot in live(state, table)? {
                index.insert(slot, key(held.key(slot).expect("live indexed row")));
            }
        }
    } else if readable {
        for &slot in &changes.deleted {
            index.remove(slot);
        }
        let held = columns::table(state, table)?;
        for &slot in &changes.inserted {
            index.insert(slot, key(held.key(slot).expect("inserted indexed row")));
        }
    }
    Ok(())
}

pub(super) fn live(state: &segments::State, table: TableId) -> io::Result<Vec<u32>> {
    Ok(match columns::truncated(state, table) {
        Some(slots) => slots.to_vec(),
        None => columns::table(state, table)?.live().ones().collect(),
    })
}

pub(super) fn edits(state: &segments::State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<_> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter_map(|(&slot, &written)| (written & mask != 0).then_some(slot))
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

pub(super) fn text(
    state: &segments::State,
    table: TableId,
    field: usize,
    slot: u32,
) -> io::Result<Vec<u8>> {
    let held = columns::table(state, table)?;
    let id = columns::text(held, field)?[slot as usize];
    state
        .dictionary()
        .get(id)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| io::Error::other("Equipment loss text has no dictionary entry"))
}

pub(super) fn mask(fields: &[usize]) -> u64 {
    fields.iter().fold(0, |mask, &field| mask | 1_u64 << field)
}
