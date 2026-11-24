
use crate::{
    answer::{State, Table},
    evaluate::{Find, number},
};
use dashboard_query::coverage::Coverage;
use replay_columnar::store::TableState;
use std::borrow::Cow;

#[derive(Default)]
pub(crate) struct LiveRows {
    rows: Option<Table>,
    held: bool,
    changed: Vec<bool>,
    list: Vec<u32>,
    touched: Vec<u32>,
    whole: bool,
    cells: Vec<(i64, State)>,
}

pub(crate) enum Answer {
    Same,
    Body,
}

impl LiveRows {
    pub(crate) fn touch(&mut self, keys: &[u128], key: u128) {
        if let Ok(at) = keys.binary_search(&key) {
            self.touched.push(at as u32);
        }
    }

    pub(crate) fn touch_all(&mut self) {
        self.whole = true;
    }

    pub(crate) fn unrecorded(&mut self) {
        self.rows = None;
        self.touched.clear();
        self.whole = false;
        self.list.clear();
        self.changed.clear();
    }

    pub(crate) fn resend(&mut self) {
        self.held = false;
    }

    pub(crate) fn sent(&mut self, kind: u8) {
        self.held = kind == crate::answer::ROWS || kind == crate::answer::CHANGED;
        for &row in &self.list {
            self.changed[row as usize] = false;
        }
        self.list.clear();
    }

    #[allow(clippy::too_many_arguments, reason = "live row evaluation takes keys, columns, state, index, witness set, coverage rule and output buffers")]
    pub(crate) fn evaluate(&mut self, keys: &[u128], columns: &[usize], cells: &TableState, find: &dyn Find, read_rows: Option<&[u32]>, coverage: impl Fn(bool) -> Coverage, scratch: &mut Table, body: &mut Vec<u8>) -> (Answer, usize) {
        let slot_of = |key: u128| find.slot(key).filter(|slot| read_rows.is_none_or(|read| read.binary_search(slot).is_ok()));
        let mut read = 0;
        if self.rows.is_none() {
            let mut rows = Table::default();
            rows.clear(columns.len(), false);
            for &key in keys {
                rows.row(key);
                let slot = slot_of(key);
                match coverage(slot.is_some()) {
                    Coverage::NotRecorded(reason) => rows.absent(&reason),
                    Coverage::Recorded => match slot {
                        Some(slot) => {
                            read += row_cells(&mut self.cells, columns, cells, Some(slot));
                            self.cells.iter().for_each(|&(value, state)| rows.cell(value, state));
                        }
                        None => rows.absent(crate::live::NO_ROW),
                    },
                }
            }
            self.rows = Some(rows);
            self.held = false;
            self.changed = vec![false; keys.len()];
            self.list.clear();
        } else {
            if self.whole {
                self.touched.clear();
                self.touched.extend(0..keys.len() as u32);
            } else {
                self.touched.sort_unstable();
                self.touched.dedup();
            }
            let rows = self.rows.as_mut().expect("rows built");
            for &row in &self.touched {
                let row = row as usize;
                let slot = slot_of(keys[row]);
                let missing = match coverage(slot.is_some()) {
                    Coverage::NotRecorded(reason) => Some(Cow::Owned(reason)),
                    Coverage::Recorded => slot.is_none().then_some(Cow::Borrowed(crate::live::NO_ROW)),
                };
                read += row_cells(&mut self.cells, columns, cells, if missing.is_none() { slot } else { None });
                if rows.set_row(row, &self.cells, missing.as_deref()) && !self.changed[row] {
                    self.changed[row] = true;
                    self.list.push(row as u32);
                }
            }
        }
        let rows = self.rows.as_ref().expect("rows built");
        self.touched.clear();
        self.whole = false;
        body.clear();
        if !self.held {
            rows.encode(body);
            return (Answer::Body, read);
        }
        if self.list.is_empty() {
            return (Answer::Same, read);
        }
        self.list.sort_unstable();
        rows.rows_into(&self.list, scratch);
        scratch.encode_changed(body);
        (Answer::Body, read)
    }

    pub(crate) fn whole(&self) -> u32 {
        self.rows.as_ref().map_or(0, |rows| rows.encoded_len() as u32)
    }

    pub(crate) fn bytes(&self) -> usize {
        self.rows.as_ref().map_or(0, Table::bytes) + self.changed.capacity() + (self.list.capacity() + self.touched.capacity()) * 4 + self.cells.capacity() * 16
    }
}

fn row_cells(out: &mut Vec<(i64, State)>, columns: &[usize], cells: &TableState, slot: Option<u32>) -> usize {
    out.clear();
    match slot {
        Some(slot) => {
            out.extend(columns.iter().map(|&column| number(cells.value(column, slot))));
            columns.len()
        }
        None => {
            out.extend(columns.iter().map(|_| (0, State::NoRow)));
            0
        }
    }
}
