
use super::view::EVERY;
use crate::playhead_host::mirror::Mirror;

#[derive(Debug, Default)]
pub struct Touched {
    columns: Vec<u64>,
    slots: Vec<u32>,
    keys: Vec<u128>,
}

impl Touched {
    pub fn rows(&self) -> usize {
        self.slots.len() + self.keys.len()
    }

    pub fn write(&mut self, slot: u32, column: u64) {
        let at = slot as usize;
        if self.columns.len() <= at {
            self.columns.resize(at + 1, 0);
        }
        if self.columns[at] == 0 {
            self.slots.push(slot);
        }
        self.columns[at] |= column;
    }

    pub fn key(&mut self, key: u128) {
        self.keys.push(key);
    }

    pub fn renumber(&mut self, old: &[u32]) {
        if self.slots.is_empty() {
            return;
        }
        let mut new = vec![u32::MAX; self.columns.len()];
        for (to, &from) in old.iter().enumerate() {
            if let Some(held) = new.get_mut(from as usize) {
                *held = to as u32;
            }
        }
        let mut columns = vec![0; old.len()];
        let mut slots = Vec::with_capacity(self.slots.len());
        for &slot in &self.slots {
            let to = new[slot as usize];
            if to != u32::MAX {
                columns[to as usize] = self.columns[slot as usize];
                slots.push(to);
            }
        }
        (self.columns, self.slots) = (columns, slots);
    }

    pub fn take(&mut self, mirror: &Mirror) -> Option<Vec<(u128, u64)>> {
        let mut rows: Vec<(u128, u64)> = Vec::with_capacity(self.rows());
        let mut keyed = true;
        for &slot in &self.slots {
            let columns = std::mem::take(&mut self.columns[slot as usize]);
            if mirror.live(slot) {
                match mirror.key(slot) {
                    Some(key) => rows.push((key, columns)),
                    None => keyed = false,
                }
            }
        }
        self.slots.clear();
        rows.extend(self.keys.drain(..).map(|key| (key, EVERY)));
        keyed.then(|| merge(rows))
    }
}

pub fn merge(mut rows: Vec<(u128, u64)>) -> Vec<(u128, u64)> {
    rows.sort_unstable_by_key(|&(key, _)| key);
    rows.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same {
            kept.1 |= later.1;
        }
        same
    });
    rows
}
