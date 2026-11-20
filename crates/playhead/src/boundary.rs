//! Crossing a piece's boundary as a step. A piece's first hour re-records the world the piece
//! before it left an hour earlier, from another session, so almost every row stands at the slot it
//! stood at under the key it had. [`crossed`] gives the change set from the one state to the other
//! — the rows gone, the rows new, and the cells that differ — and the readers take it as they take
//! any step. A table whose rows no change set can carry over (a log, whose rows are ordinals of
//! its own run) is reset instead, alone.

use crate::{changes::Changes, lane::Lane};
use replay_columnar::store::{TableState, Value};
use std::io;

/// What one table's crossing holds: the rows before and after, those that stood at their slot
/// under their key, those gone and new, and of the ones that stood, those a cell differs at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crossed {
    pub rows_before: u32,
    pub rows_after: u32,
    pub stood: u32,
    pub gone: u32,
    pub born: u32,
    pub changed: u32,
    pub cells: u32,
}

/// Slots, one bit each.
struct Bits(Vec<u64>);

impl Bits {
    fn new(slots: usize) -> Self {
        Self(vec![0; slots.div_ceil(64)])
    }

    #[inline]
    fn set(&mut self, slot: u32) {
        self.0[slot as usize / 64] |= 1 << (slot % 64);
    }

    #[inline]
    fn get(&self, slot: u32) -> bool {
        self.0[slot as usize / 64] & (1 << (slot % 64)) != 0
    }
}

/// The change set from `before`'s cells at its piece's last hour to `after`'s at the next piece's
/// first, at `after`'s head; none where the table cannot be carried over and must be reset. The
/// strings `after` has named move into the change set: the crossing's text cells are its run's ids.
pub fn crossed(before: &Lane, after: &mut Lane, out: &mut Changes) -> io::Result<Option<Crossed>> {
    if before.table() != after.table() || before.requested() != after.requested() || before.reads() != after.reads() || before.keyed() != after.keyed() {
        return Ok(None);
    }
    if before.log().is_some() || after.log().is_some() {
        return Ok(None);
    }
    let (Some(head), Some(_)) = (after.head(), before.head()) else { return Ok(None) };
    let columns: Vec<usize> = after.reads().to_vec();
    out.begin(after.table(), &columns, after.keyed(), false);
    let mut held = Crossed::default();
    {
        let (old, new): (&TableState, &TableState) = (before.cells(), after.cells());
        let slots = old.slots().max(new.slots());
        let (mut stood, mut fresh) = (Bits::new(slots as usize), Bits::new(slots as usize));
        for slot in 0..slots {
            match (old.key(slot), new.key(slot)) {
                (Some(was), Some(now)) if was == now => {
                    stood.set(slot);
                    held.stood += 1;
                }
                (was, now) => {
                    if let Some(key) = was {
                        out.deleted.push(head, slot, key);
                        held.gone += 1;
                    }
                    if let Some(key) = now {
                        out.inserted.push(head, slot, key);
                        fresh.set(slot);
                        held.born += 1;
                    }
                }
            }
        }
        held.rows_before = held.stood + held.gone;
        held.rows_after = held.stood + held.born;
        let mut differs = Bits::new(slots as usize);
        for (written, &column) in out.columns.iter_mut().zip(&columns) {
            let (was, now) = (old.column(column), new.column(column));
            written.slots.clear();
            for slot in 0..slots {
                if fresh.get(slot) {
                    written.slots.push(slot);
                } else if stood.get(slot) && differs_across(was.map(|held| held.value(slot)), now.map(|held| held.value(slot))) {
                    written.slots.push(slot);
                    if !differs.get(slot) {
                        differs.set(slot);
                        held.changed += 1;
                    }
                }
            }
            held.cells += written.slots.len() as u32;
            written.fill(now.expect("a selected column"))?;
        }
    }
    after.texts_into(out);
    out.status = Some(after.status());
    out.set_head(head, 0, false);
    Ok(Some(held))
}

/// Whether a cell that stood across the boundary is written by it. Each run numbers its strings
/// anew, so the same id in the two runs may spell different text: a text cell is always written.
fn differs_across(was: Option<Value<'_>>, now: Option<Value<'_>>) -> bool {
    match (was, now) {
        (Some(Value::Text(_)), Some(Value::Text(_))) => true,
        (was, now) => was != now,
    }
}
