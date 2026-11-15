//! What a layer's `apply` keeps of a step: the rows each change set touched, as the step names
//! them (rows gone and come by key, cells written by slot), read against the mirrors only when the
//! budgeted settle takes them. A written slot is read then, not at its step: a slot a later step
//! reuses names the later row, and that step's gone and come rows are marks of their own.

use super::Budget;
use crate::playhead_host::mirror::Mirrors;
use playhead::changes::Changes;
use replay_data::rows::TableId;

/// Every column group: what a row gone or come touches.
pub(crate) const EVERY: u8 = u8::MAX;

/// How a mark touched its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Touch {
    Gone,
    Came,
    /// Its cells of these column groups were written, at this slot.
    Written(u8, u32),
}

impl Touch {
    /// The column groups it touched.
    pub(crate) fn groups(self) -> u8 {
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
    /// The row's key; a written cell's is read at the take.
    key: u128,
}

/// The marks of the steps applied and not yet taken, in step order.
#[derive(Default)]
pub(crate) struct Touched {
    marks: Vec<Mark>,
    since: Option<u64>,
    written: Vec<(u32, u8)>,
}

impl Touched {
    /// Notes `changes`' rows gone and come (when its readers asked for keys), then each slot written
    /// in a column `groups` places in a group (0: none), once, with every group it was written in.
    pub(crate) fn note(&mut self, changes: &Changes, frame: u64, groups: impl Fn(usize) -> u8) {
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

    /// Takes the marks in order while `budget` lasts, looking at it every 32: each with its row's
    /// key (a written slot's as its table's mirror holds it now; one the mirror names no key for is
    /// dropped). Whether every mark was taken.
    pub(crate) fn take(&mut self, mirrors: &Mirrors, budget: &mut Budget, mut each: impl FnMut(TableId, Touch, u128)) -> bool {
        let mut done = 0;
        while done < self.marks.len() {
            if done % 32 == 0 && !budget.left() {
                break;
            }
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
        self.marks.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.marks.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// The frame the oldest mark not taken was noted at.
    pub(crate) fn since(&self) -> Option<u64> {
        self.since
    }

    pub(crate) fn clear(&mut self) {
        self.marks.clear();
        self.since = None;
    }

    pub(crate) fn bytes(&self) -> usize {
        self.marks.capacity() * std::mem::size_of::<Mark>() + self.written.capacity() * 8
    }
}
