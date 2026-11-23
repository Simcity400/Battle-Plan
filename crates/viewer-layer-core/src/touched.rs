
use crate::playhead_host::mirror::Mirrors;
use playhead::changes::Changes;
use replay_data::rows::TableId;

pub const EVERY: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Gone,
    Came,
    Written(u8, u32),
}

impl Touch {
    pub fn groups(self) -> u8 {
        match self {
            Self::Gone | Self::Came => EVERY,
            Self::Written(groups, _) => groups,
        }
    }
}

#[derive(Clone, Copy)]
struct Mark {
    table: u16,
    touch: Touch,
    key: u128,
}

#[derive(Default)]
pub struct Touched {
    marks: Vec<Mark>,
    since: Option<u64>,
    written: Vec<(u32, u8)>,
}

impl Touched {
    pub fn note(&mut self, changes: &Changes, frame: u64, groups: impl Fn(usize) -> u8) {
        let table = changes.table() as u16;
        let before = self.marks.len();
        for (lives, touch) in [(changes.deleted(), Touch::Gone), (changes.inserted(), Touch::Came)] {
            self.marks.extend(lives.keys().unwrap_or_default().iter().map(|&key| Mark { table, touch, key }));
        }
        self.written.clear();
        for written in changes.columns() {
            let group = groups(written.column());
            if group != 0 {
                self.written.extend(written.slots().iter().map(|&slot| (slot, group)));
            }
        }
        self.written.sort_unstable_by_key(|&(slot, _)| slot);
        let mut at = 0;
        while at < self.written.len() {
            let (slot, mut group) = self.written[at];
            at += 1;
            while at < self.written.len() && self.written[at].0 == slot {
                group |= self.written[at].1;
                at += 1;
            }
            self.marks.push(Mark { table, touch: Touch::Written(group, slot), key: 0 });
        }
        if self.marks.len() > before {
            self.since.get_or_insert(frame);
        }
    }

    pub fn take(&mut self, mirrors: &Mirrors, mut each: impl FnMut(TableId, Touch, u128)) {
        let mut done = 0;
        while done < self.marks.len() {
            let Mark { table, touch, key } = self.marks[done];
            done += 1;
            let table = TableId::ALL[usize::from(table)];
            let key = match touch {
                Touch::Written(_, slot) => mirrors.get(table).and_then(|mirror| mirror.key(slot)),
                Touch::Gone | Touch::Came => Some(key),
            };
            if let Some(key) = key {
                each(table, touch, key);
            }
        }
        self.marks.drain(..done);
        if self.marks.is_empty() {
            self.since = None;
        }
    }

    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn clear(&mut self) {
        self.marks.clear();
        self.since = None;
    }

    pub fn bytes(&self) -> usize {
        self.marks.capacity() * std::mem::size_of::<Mark>() + self.written.capacity() * 8
    }
}
