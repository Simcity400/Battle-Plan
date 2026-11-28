
use crate::{changes::Changes, lane::Lane};
use replay_columnar::store::{TableState, Value};
use std::io;

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
    after.report_status(out);
    if let (Some(before), Some(after)) = (before.read_rows(), after.read_rows()) {
        out.read_rows_changed = before != after;
    }
    out.set_head(head, 0, false);
    Ok(Some(held))
}

fn differs_across(was: Option<Value<'_>>, now: Option<Value<'_>>) -> bool {
    match (was, now) {
        (Some(Value::Text(_)), Some(Value::Text(_))) => true,
        (was, now) => was != now,
    }
}
